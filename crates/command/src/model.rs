use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::{self, Debug};
use std::io::Cursor;
use std::io::ErrorKind;
use std::marker::PhantomData;
use std::num::TryFromIntError;
use std::num::{NonZeroU64, NonZeroUsize};

use bytes::Bytes;
use mnesis::{
    Aggregate, AggregateState, DomainEvent, EventOf, Events, Handle, KernelError, Message, Version,
    events,
};
use mnesis_store::{Decode, Encode, PersistedEnvelope, StoreError};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const MAX_ACCEPTED: usize = 4096;
const MAX_BYTES: usize = 4096;
const EVENT_TYPE: &str = "controller.accepted.v2";

/// Encoding, decoding or acceptance validation failure.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// The serialized request or record exceeds the supported 4 KiB bound.
    #[error("encoded value exceeds 4 KiB")]
    Bytes {
        /// The original writer error; absent when input length was checked first.
        #[source]
        source: Option<serde_json::Error>,
    },
    /// The original JSON serialization or deserialization failure.
    #[error("JSON codec failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Serialization cannot reproduce the original typed value.
    #[error("lossy serialization")]
    Lossy,
    /// The envelope has an unsupported event type or schema.
    #[error("unsupported acceptance envelope")]
    Envelope,
    /// The acceptance's schema, request, receipt or fact metadata disagree.
    #[error("invalid acceptance metadata")]
    Acceptance,
    /// The encoded length cannot be represented on this target.
    #[error("encoded length overflow: {0}")]
    Length(#[from] TryFromIntError),
}

#[cfg(test)]
#[path = "../tests/unit/model.rs"]
mod tests;

/// Exact retry identity, expected physical stream position and domain command.
///
/// Application authorization runs before this request reaches the processor.
/// Use a validated identity type; the default rejects zero during decoding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request<C, I = NonZeroU64> {
    /// Original identity within this aggregate's stream.
    pub id: I,
    /// Caller-observed physical stream version; `None` means an empty stream.
    #[serde(with = "optional_version")]
    pub expected: Option<Version>,
    /// Domain payload; generation and domain deadlines belong here.
    pub command: C,
}
impl<C: Serialize, I: Serialize> Request<C, I> {
    /// Encode the bounded request without allocating unbounded output.
    ///
    /// # Errors
    /// Rejects serialization failures and output larger than 4 KiB.
    pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        bounded(self)
    }
}
impl<C: DeserializeOwned, I: DeserializeOwned> Request<C, I> {
    /// Decode the bounded request; payload types own their field/schema rules.
    ///
    /// # Errors
    /// Rejects oversized, malformed and unsupported requests.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() > MAX_BYTES {
            return Err(CodecError::Bytes { source: None });
        }
        serde_json::from_slice(bytes).map_err(CodecError::Json)
    }
}
impl<C: Serialize + DeserializeOwned + Eq, I: Serialize + DeserializeOwned + Eq> Request<C, I> {
    pub(super) fn validate(&self) -> Result<(), CodecError> {
        let encoded = self.encode()?;
        let decoded = Self::decode(&encoded)?;
        if &decoded != self {
            return Err(CodecError::Lossy);
        }
        Ok(())
    }
}

/// Original acceptance; it does not certify any external effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt<I = NonZeroU64> {
    /// Original command identity.
    pub id: I,
    /// Physical accepted-record version, independent of domain fact count.
    #[serde(with = "version")]
    pub version: Version,
    /// Number of decided domain facts; zero is an accepted no-op.
    pub events: usize,
}

/// A rejected request, unavailable read or unresolved write, preserving sources.
#[derive(Debug, thiserror::Error)]
pub enum Error<D = Infallible, I = NonZeroU64, E = Infallible> {
    /// A definite failure of this attempt. Earlier unresolved writes remain held.
    #[error("command processing failed: {0}")]
    Failed(#[source] Box<Cause<D, E>>),
    /// This ID already belongs to different request content.
    #[error("command identity has different content")]
    Mismatch,
    /// Confirmed version conflict without a matching existing receipt.
    #[error("expected version is stale")]
    Conflict,
    /// Waiting capacity is occupied by queued work or readiness reservations.
    #[error("command queue is full")]
    QueueFull,
    /// The configured queue exceeds Tokio's supported capacity.
    #[error("queue capacity {requested} exceeds {maximum}")]
    QueueLimit {
        /// The rejected capacity.
        requested: usize,
        /// Tokio's maximum supported capacity.
        maximum: usize,
    },
    /// Store operations require a nonzero deadline.
    #[error("operation deadline must be nonzero")]
    DeadlineZero,
    /// Admission is closed.
    #[error("command processor is closed")]
    Closed,
    /// The observed shutdown belongs to a different stream.
    #[error("shutdown belongs to a different aggregate stream")]
    Scope,
    /// Startup was attempted outside a Tokio runtime.
    #[error("command processor requires a Tokio runtime: {0}")]
    Runtime(#[source] tokio::runtime::TryCurrentError),
    /// The append may still commit; retain the exact original request.
    #[error("command {command:?} has an uncertain write outcome")]
    Uncertain {
        /// Original identity, including any application-owned namespace.
        command: I,
        /// Original typed source, absent for timeout or a dropped reply.
        #[source]
        source: Option<Box<Cause<D, E>>>,
    },
    /// Request encoding/decoding failed before acceptance.
    #[error("invalid request: {0}")]
    Request(#[source] CodecError),
    /// Persisted acceptance metadata violates its schema or replay laws.
    #[error("invalid acceptance history")]
    Corrupt,
    /// Read timeout; this does not settle any earlier write.
    #[error("read timed out")]
    Timeout,
}

/// The original failure, shared by definite and uncertain outcomes.
///
/// A store failure during append can be uncertain even when its source is a
/// kernel error: the repository folds application state after persistence.
#[derive(Debug, thiserror::Error)]
pub enum Cause<D, E> {
    /// The application rejected the decision before append.
    #[error("domain rejected the command: {0}")]
    Rejected(#[source] D),
    /// The original typed repository failure.
    #[error("store operation failed: {0}")]
    Store(#[source] StoreError<E, CodecError, CodecError>),
    /// The original kernel failure, with its execution phase held by the outcome.
    #[error("aggregate integrity failed: {0}")]
    Kernel(#[source] KernelError),
    /// The physical accepted-record limit was reached before a new append.
    #[error("accepted-record history is full")]
    HistoryFull,
    /// The decision would exceed the aggregate's cumulative domain-fact limit.
    #[error("domain replay limit {maximum} exceeded")]
    ReplayLimit {
        /// The aggregate's maximum cumulative fact count.
        maximum: usize,
    },
    /// Persisted acceptance metadata failed validation after append.
    #[error("invalid committed acceptance")]
    Corrupt,
}
impl<D, I, E> From<Cause<D, E>> for Error<D, I, E> {
    fn from(cause: Cause<D, E>) -> Self {
        // Keep large application/adapter causes out of the reply's inline error.
        Self::Failed(Box::new(cause))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecisionError<D> {
    #[error("domain rejected the command: {0}")]
    Rejected(#[source] D),
    #[error("accepted-record history is full")]
    HistoryFull,
    #[error("domain replay limit {maximum} exceeded")]
    ReplayLimit { maximum: usize },
}
impl<D, E> From<DecisionError<D>> for Cause<D, E> {
    fn from(cause: DecisionError<D>) -> Self {
        match cause {
            DecisionError::Rejected(error) => Self::Rejected(error),
            DecisionError::HistoryFull => Self::HistoryFull,
            DecisionError::ReplayLimit { maximum } => Self::ReplayLimit { maximum },
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("command identity has different content")]
pub struct Mismatch;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record<C, I, E> {
    schema: u8,
    request: Request<C, I>,
    receipt: Receipt<I>,
    facts: Vec<E>,
}
impl<C, I, E> Message for Record<C, I, E>
where
    C: Debug + Send + Sync + 'static,
    I: Debug + Send + Sync + 'static,
    E: DomainEvent,
{
}
impl<C, I, E> DomainEvent for Record<C, I, E>
where
    C: Debug + Send + Sync + 'static,
    I: Debug + Send + Sync + 'static,
    E: DomainEvent,
{
    fn name(&self) -> &'static str {
        EVENT_TYPE
    }
}

/// One decoded acceptance with its original request, receipt and ordered facts.
///
/// [`AcceptanceCodec`] validates each record. Aggregate replay validates stream
/// ordering, repeated identities and cumulative limits across records. Consumers
/// checkpoint after folding all facts at the record's original physical position.
#[derive(Debug)]
pub struct Acceptance<C, E, I = NonZeroU64>(Record<C, I, E>);

impl<C, E, I> Acceptance<C, E, I> {
    /// Borrow the exact original request, including its expected version.
    #[must_use]
    pub const fn request(&self) -> &Request<C, I> {
        &self.0.request
    }

    /// Borrow the original receipt; its version counts acceptance records.
    #[must_use]
    pub const fn receipt(&self) -> &Receipt<I> {
        &self.0.receipt
    }

    /// Borrow the ordered facts; an accepted no-op has an empty slice.
    #[must_use]
    pub const fn facts(&self) -> &[E] {
        self.0.facts.as_slice()
    }
}

impl<C, E, I> Message for Acceptance<C, E, I>
where
    C: Debug + Send + Sync + 'static,
    I: Debug + Send + Sync + 'static,
    E: DomainEvent,
{
}

impl<C, E, I> DomainEvent for Acceptance<C, E, I>
where
    C: Debug + Send + Sync + 'static,
    I: Debug + Send + Sync + 'static,
    E: DomainEvent,
{
    fn name(&self) -> &'static str {
        self.0.name()
    }
}

/// Decode acceptance records through the same validation used by the processor.
#[derive(Debug)]
pub struct AcceptanceCodec;

impl<C: DeserializeOwned + Eq, E: DeserializeOwned, I: DeserializeOwned + Eq>
    Decode<Acceptance<C, E, I>> for AcceptanceCodec
{
    type Output<'a> = Acceptance<C, E, I>;
    type Error = CodecError;

    fn decode(&self, envelope: &PersistedEnvelope) -> Result<Acceptance<C, E, I>, Self::Error> {
        Codec.decode(envelope).map(Acceptance)
    }
}

pub struct State<A: Aggregate, C, I, const N: usize> {
    pub(super) domain: A::State,
    receipts: BTreeMap<I, (Request<C, I>, Receipt<I>)>,
    facts: usize,
    pub(super) integrity: Result<(), ()>,
}
impl<A: Aggregate, C: Debug, I: Debug, const N: usize> Debug for State<A, C, I, N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("State")
            .field("domain", &self.domain)
            .field("receipts", &self.receipts)
            .field("facts", &self.facts)
            .field("integrity", &self.integrity)
            .finish()
    }
}
impl<A: Aggregate, C: Clone + Eq, I: Clone + Ord, const N: usize> State<A, C, I, N> {
    pub(super) fn receipt(&self, request: &Request<C, I>) -> Result<Option<Receipt<I>>, Mismatch> {
        match self.receipts.get(&request.id) {
            Some((original, receipt)) if original == request => Ok(Some(receipt.clone())),
            Some(_) => Err(Mismatch),
            None => Ok(None),
        }
    }
    pub(super) fn original(&self, id: &I) -> Option<Request<C, I>> {
        self.receipts.get(id).map(|(request, _)| request.clone())
    }
    fn total(&self, count: usize) -> Option<usize> {
        if count.checked_sub(1).is_some_and(|tail| tail > N) {
            return None;
        }
        self.facts
            .checked_add(count)
            .filter(|total| *total <= A::MAX_REHYDRATION_EVENTS.get())
    }
}
impl<A, C, I, const N: usize> AggregateState for State<A, C, I, N>
where
    A: Aggregate + 'static,
    C: Clone + Debug + Eq + Send + Sync + 'static,
    I: Clone + Debug + Ord + Send + Sync + 'static,
    EventOf<A>: Clone,
{
    type Event = Record<C, I, EventOf<A>>;
    fn initial() -> Self {
        Self {
            domain: A::State::initial(),
            receipts: BTreeMap::new(),
            facts: 0,
            integrity: Ok(()),
        }
    }
    fn apply(mut self, record: &Self::Event) -> Self {
        // The codec validates record metadata; the kernel validates physical
        // order and its replay limit. Only cross-record rules belong here.
        if self.integrity.is_err() || self.receipts.contains_key(&record.request.id) {
            self.integrity = Err(());
            return self;
        }
        let Some(total) = self.total(record.facts.len()) else {
            self.integrity = Err(());
            return self;
        };
        for fact in &record.facts {
            self.domain = self.domain.apply(fact);
        }
        self.facts = total;
        self.receipts.insert(
            record.request.id.clone(),
            (record.request.clone(), record.receipt.clone()),
        );
        self
    }
}

type Marker<A, C, I> = fn() -> (A, C, I);
pub struct Adapted<A, C, I, const N: usize>(PhantomData<Marker<A, C, I>>);
impl<A, C, I, const N: usize> Aggregate for Adapted<A, C, I, N>
where
    A: Handle<C, N> + 'static,
    C: Clone + Debug + Eq + Send + Sync + 'static,
    I: Clone + Debug + Ord + Send + Sync + 'static,
    EventOf<A>: Clone,
{
    type State = State<A, C, I, N>;
    type Error = DecisionError<A::Error>;
    type Id = A::Id;
    const MAX_REHYDRATION_EVENTS: NonZeroUsize = NonZeroUsize::new(MAX_ACCEPTED).unwrap();
}
pub struct Decision<C, I> {
    pub(super) request: Request<C, I>,
    pub(super) version: Version,
}
impl<A, C, I, const N: usize> Handle<Decision<C, I>> for Adapted<A, C, I, N>
where
    A: Handle<C, N> + 'static,
    C: Clone + Debug + Eq + Send + Sync + 'static,
    I: Clone + Debug + Ord + Send + Sync + 'static,
    EventOf<A>: Clone,
{
    fn handle(
        state: &Self::State,
        decision: Decision<C, I>,
    ) -> Result<Option<Events<EventOf<Self>>>, Self::Error> {
        // Engine has already checked integrity, retry identity and expected
        // version on this exclusively owned root; no await can mutate it.
        if state.receipts.len() >= MAX_ACCEPTED {
            return Err(DecisionError::HistoryFull);
        }
        let facts = A::handle(&state.domain, decision.request.command.clone())
            .map_err(DecisionError::Rejected)?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if state.total(facts.len()).is_none() {
            return Err(DecisionError::ReplayLimit {
                maximum: A::MAX_REHYDRATION_EVENTS.get(),
            });
        }
        let receipt = Receipt {
            id: decision.request.id.clone(),
            version: decision.version,
            events: facts.len(),
        };
        Ok(Some(events![Record {
            schema: 2,
            request: decision.request,
            receipt,
            facts
        }]))
    }
}

pub struct Codec;
impl<C, I, E> Encode<Record<C, I, E>> for Codec
where
    C: Serialize + DeserializeOwned + Eq,
    I: Serialize + DeserializeOwned + Eq,
    E: Serialize + DeserializeOwned + Eq,
{
    type Error = CodecError;
    fn encode(&self, record: &Record<C, I, E>) -> Result<Bytes, Self::Error> {
        let bytes = bounded(record)?;
        let decoded: Record<C, I, E> = serde_json::from_slice(&bytes)?;
        if &decoded != record {
            return Err(CodecError::Lossy);
        }
        Ok(bytes.into())
    }
}
impl<C: DeserializeOwned + Eq, I: DeserializeOwned + Eq, E: DeserializeOwned>
    Decode<Record<C, I, E>> for Codec
{
    type Output<'a> = Record<C, I, E>;
    type Error = CodecError;
    fn decode(&self, envelope: &PersistedEnvelope) -> Result<Record<C, I, E>, Self::Error> {
        if envelope.payload().len() > MAX_BYTES {
            return Err(CodecError::Bytes { source: None });
        }
        if envelope.schema_version() != 1 || envelope.event_type() != EVENT_TYPE {
            return Err(CodecError::Envelope);
        }
        let record: Record<C, I, E> = serde_json::from_slice(envelope.payload())?;
        let next = record
            .request
            .expected
            .map_or(Some(Version::INITIAL), Version::next);
        if record.schema != 2
            || record.receipt.version != envelope.version()
            || Some(record.receipt.version) != next
            || record.receipt.id != record.request.id
            || record.receipt.events != record.facts.len()
        {
            return Err(CodecError::Acceptance);
        }
        Ok(record)
    }
}
fn bounded<T: Serialize>(value: &T) -> Result<Vec<u8>, CodecError> {
    let mut buffer = [0; MAX_BYTES];
    let mut writer = Cursor::new(buffer.as_mut_slice());
    serde_json::to_writer(&mut writer, value).map_err(|error| {
        if error.io_error_kind() == Some(ErrorKind::WriteZero) {
            CodecError::Bytes {
                source: Some(error),
            }
        } else {
            CodecError::Json(error)
        }
    })?;
    let length = usize::try_from(writer.position())?;
    Ok(buffer[..length].to_vec())
}
mod version {
    use super::{Deserialize, Serialize, Version};
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "Serde's with hook requires a borrowed field"
    )]
    pub(super) fn serialize<S: serde::Serializer>(
        value: &Version,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_u64().serialize(serializer)
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Version, D::Error> {
        Version::new(u64::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("zero version"))
    }
}
mod optional_version {
    use super::{Deserialize, Serialize, Version};
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        clippy::ref_option,
        reason = "Serde's with hook requires a borrowed field"
    )]
    pub(super) fn serialize<S: serde::Serializer>(
        value: &Option<Version>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value.map(Version::as_u64).serialize(serializer)
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Version>, D::Error> {
        Option::<u64>::deserialize(deserializer)?
            .map(|number| {
                Version::new(number).ok_or_else(|| serde::de::Error::custom("zero version"))
            })
            .transpose()
    }
}
