use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt::{self, Debug};
use std::io::Cursor;
use std::marker::PhantomData;
use std::num::{NonZeroU64, NonZeroUsize};

use bytes::Bytes;
use mnesis::{
    Aggregate, AggregateState, DomainEvent, EventOf, Events, Handle, Message, Version, events,
};
use mnesis_store::{Decode, Encode, PersistedEnvelope};
use serde::de::DeserializeOwned;
use serde::ser::Error as _;
use serde::{Deserialize, Serialize};

pub const MAX_ACCEPTED: usize = 4096;
const MAX_BYTES: usize = 4096;
const EVENT_TYPE: &str = "controller.accepted.v2";

#[cfg(test)]
#[path = "../tests/unit/model.rs"]
mod tests;

/// Exact retry identity, expected physical stream position and domain command.
///
/// Application authorization runs before this request reaches the controller.
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
    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        bounded(self)
    }
}
impl<C: DeserializeOwned, I: DeserializeOwned> Request<C, I> {
    /// Decode the bounded request; payload types own their field/schema rules.
    ///
    /// # Errors
    /// Rejects oversized, malformed and unsupported requests.
    pub fn decode(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        if bytes.len() > MAX_BYTES {
            return Err(serde_json::Error::custom("request exceeds 4 KiB"));
        }
        serde_json::from_slice(bytes)
    }
}
impl<C: Serialize + DeserializeOwned + Eq, I: Serialize + DeserializeOwned + Eq> Request<C, I> {
    pub(super) fn validate(&self) -> Result<(), serde_json::Error> {
        let encoded = self.encode()?;
        let decoded = Self::decode(&encoded)?;
        if &decoded != self {
            return Err(serde_json::Error::custom("lossy request serialization"));
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
pub enum Error<D = Infallible, I = NonZeroU64> {
    /// The domain rejected a new command; no append was issued.
    #[error("domain rejected the command: {0}")]
    Rejected(#[source] D),
    /// This ID already belongs to different request content.
    #[error("command identity has different content")]
    Mismatch,
    /// Confirmed version conflict without a matching existing receipt.
    #[error("expected version is stale")]
    Conflict,
    /// Queue, retained history, batch or encoded record capacity was reached.
    #[error("capacity reached")]
    Capacity,
    /// Admission is closed.
    #[error("controller closed")]
    Closed,
    /// The observed shutdown belongs to a different stream.
    #[error("shutdown belongs to a different aggregate stream")]
    Scope,
    /// Startup was attempted outside a Tokio runtime.
    #[error("controller requires a Tokio runtime: {0}")]
    Runtime(#[source] tokio::runtime::TryCurrentError),
    /// The append may still commit; retain the exact original request.
    #[error("command {command:?} has an uncertain write outcome")]
    Uncertain {
        /// Original identity, including any application-owned namespace.
        command: I,
        /// Original typed source, absent for timeout or a dropped reply.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
    /// A definite read, pre-append codec or kernel failure.
    #[error("store operation failed: {0}")]
    Store(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Request encoding/decoding failed before acceptance.
    #[error("invalid request: {0}")]
    Json(#[source] serde_json::Error),
    /// Persisted controller metadata violates its schema or replay laws.
    #[error("invalid controller history")]
    Corrupt,
    /// Read timeout; this does not settle any earlier write.
    #[error("read timed out")]
    Timeout,
}

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
    pub(super) fn receipt(
        &self,
        request: &Request<C, I>,
    ) -> Result<Option<Receipt<I>>, Error<A::Error, I>> {
        match self.receipts.get(&request.id) {
            Some((original, receipt)) if original == request => Ok(Some(receipt.clone())),
            Some(_) => Err(Error::Mismatch),
            None => Ok(None),
        }
    }
    pub(super) fn original(&self, id: &I) -> Option<Request<C, I>> {
        self.receipts.get(id).map(|(request, _)| request.clone())
    }
    fn fits(&self, count: usize) -> bool {
        count <= N.saturating_add(1)
            && self
                .facts
                .checked_add(count)
                .is_some_and(|total| total <= A::MAX_REHYDRATION_EVENTS.get())
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
        if self.integrity.is_err()
            || self.receipts.contains_key(&record.request.id)
            || !self.fits(record.facts.len())
        {
            self.integrity = Err(());
            return self;
        }
        for fact in &record.facts {
            self.domain = self.domain.apply(fact);
        }
        self.facts += record.facts.len();
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
    type Error = Error<A::Error, I>;
    type Id = A::Id;
    const MAX_REHYDRATION_EVENTS: NonZeroUsize = NonZeroUsize::MIN.saturating_add(MAX_ACCEPTED - 1);
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
            return Err(Error::Capacity);
        }
        let facts = A::handle(&state.domain, decision.request.command.clone())
            .map_err(Error::Rejected)?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if !state.fits(facts.len()) {
            return Err(Error::Capacity);
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
    type Error = serde_json::Error;
    fn encode(&self, record: &Record<C, I, E>) -> Result<Bytes, Self::Error> {
        let bytes = bounded(record)?;
        let decoded: Record<C, I, E> = serde_json::from_slice(&bytes)?;
        if &decoded != record {
            return Err(serde_json::Error::custom("lossy record serialization"));
        }
        Ok(bytes.into())
    }
}
impl<C: DeserializeOwned + Eq, I: DeserializeOwned + Eq, E: DeserializeOwned>
    Decode<Record<C, I, E>> for Codec
{
    type Output<'a> = Record<C, I, E>;
    type Error = serde_json::Error;
    fn decode(&self, envelope: &PersistedEnvelope) -> Result<Record<C, I, E>, Self::Error> {
        if envelope.payload().len() > MAX_BYTES
            || envelope.schema_version() != 1
            || envelope.event_type() != EVENT_TYPE
        {
            return Err(serde_json::Error::custom("unsupported controller envelope"));
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
            return Err(serde_json::Error::custom("invalid controller acceptance"));
        }
        Ok(record)
    }
}
fn bounded<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut buffer = [0; MAX_BYTES];
    let mut writer = Cursor::new(buffer.as_mut_slice());
    serde_json::to_writer(&mut writer, value)?;
    let length = usize::try_from(writer.position()).map_err(serde_json::Error::custom)?;
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
