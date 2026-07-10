//! In-crate store double for the white-box subscription-loop tests
//! (`catchup.rs`, `subscription_cursor.rs`).
//!
//! Those tests probe `pub(crate)` seams (`StreamCatchup`, `AllCatchup`,
//! `live_stepped`, `CATCHUP_CHUNK`), so they must run inside this crate's
//! lib-test build — where the real in-memory adapter (`nexus-inmemory`)
//! cannot be used: a dev-dependency cycle unifies types only for integration
//! tests in `tests/`, while the lib-test target recompiles this crate under
//! `cfg(test)` as a *distinct* crate, so `nexus-inmemory`'s trait impls name
//! the other build's traits and satisfy none of this build's bounds.
//!
//! [`TestStore`] therefore implements just enough of [`RawEventStore`] +
//! [`WakeSource`] to drive the loop. Its wake is a single store-wide
//! generation counter — every wake rouses every waiter, which the
//! [`WakeRegistration`] contract explicitly permits as a spurious wake. The
//! public subscription surface is separately proven against the real adapter
//! and the real `nexus-wake` registry in `tests/subscription_tests.rs`.

use core::future::Future;
use std::collections::HashMap;
use std::convert::Infallible;
use std::num::NonZeroU64;

use thiserror::Error;
use tokio::sync::{Mutex, watch};

use nexus::{ErrorId, Version};

use crate::envelope::{EnvelopeError, PendingEnvelope, PersistedEnvelope};
use crate::error::AppendError;
use crate::store::{AllPosition, RawEventStore};
use crate::stream_id::StreamKey;
use crate::wake::{WakeRegistration, WakeSource};
use crate::wire::{self, WireError};

/// Error domain of [`TestStore`].
#[derive(Debug, Error)]
pub enum TestStoreError {
    /// Envelope failed wire-format validation when building the row.
    #[error("wire-format build error in test store")]
    Wire(#[from] WireError),

    /// Persisted envelope failed integrity validation.
    #[error("envelope integrity error in test store")]
    Envelope(#[from] EnvelopeError),
}

/// [`TestStore`]'s `$all` position — dense append order (a double need not
/// exercise gap tolerance; the adapter conformance suites do).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TestAllPos(NonZeroU64);

impl TestAllPos {
    pub const fn as_u64(self) -> u64 {
        self.0.get()
    }
}

impl AllPosition for TestAllPos {}

/// Minimal in-crate [`RawEventStore`] + [`WakeSource`] double.
#[derive(Debug)]
pub struct TestStore {
    inner: Mutex<Inner>,
    wake_tx: watch::Sender<u64>,
}

#[derive(Debug, Default)]
struct Inner {
    streams: HashMap<Vec<u8>, Vec<PersistedEnvelope>>,
    all: Vec<(TestAllPos, PersistedEnvelope)>,
}

impl TestStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            wake_tx: watch::Sender::new(0),
        }
    }
}

fn persist(env: &PendingEnvelope) -> Result<PersistedEnvelope, TestStoreError> {
    let frame = wire::encode_frame(
        env.schema_version_value(),
        &env.event_type_value(),
        &env.payload_value(),
        env.metadata_value().as_ref(),
    )?;
    Ok(PersistedEnvelope::try_new(
        env.version(),
        frame.value,
        env.schema_version_value(),
        frame.offsets.event_type,
        frame.offsets.payload,
        frame.offsets.metadata,
    )?)
}

impl RawEventStore for TestStore {
    type Error = TestStoreError;
    type Stream =
        futures::stream::Iter<std::vec::IntoIter<Result<PersistedEnvelope, TestStoreError>>>;
    type AllPosition = TestAllPos;
    type AllStream = futures::stream::Iter<
        std::vec::IntoIter<Result<(TestAllPos, PersistedEnvelope), TestStoreError>>,
    >;

    async fn append(
        &self,
        id: &StreamKey,
        expected_version: Option<Version>,
        envelopes: &[PendingEnvelope],
    ) -> Result<(), AppendError<TestStoreError>> {
        let mut inner = self.inner.lock().await;
        let head = inner
            .streams
            .get(id.as_bytes())
            .and_then(|rows| rows.last())
            .map(PersistedEnvelope::version);
        if head != expected_version {
            return Err(AppendError::Conflict {
                stream_id: ErrorId::from_display(id),
                expected: expected_version,
                actual: head,
            });
        }
        let persisted: Vec<PersistedEnvelope> = envelopes
            .iter()
            .map(persist)
            .collect::<Result<_, _>>()
            .map_err(AppendError::Store)?;
        for env in persisted {
            let next = inner
                .all
                .len()
                .checked_add(1)
                .and_then(|n| u64::try_from(n).ok())
                .and_then(NonZeroU64::new)
                .map(TestAllPos);
            // Vec length + 1 always fits u64 and is nonzero; expressed as a
            // checked chain to honour the arithmetic-safety rule anyway.
            let Some(pos) = next else {
                unreachable!("Vec length + 1 is always a valid TestAllPos")
            };
            inner.all.push((pos, env.clone()));
            inner
                .streams
                .entry(id.as_bytes().to_vec())
                .or_default()
                .push(env);
        }
        drop(inner);
        self.wake_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
        Ok(())
    }

    async fn read_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> Result<Self::Stream, Self::Error> {
        let inner = self.inner.lock().await;
        let rows: Vec<Result<PersistedEnvelope, TestStoreError>> = inner
            .streams
            .get(id.as_bytes())
            .into_iter()
            .flatten()
            .filter(|env| env.version() >= from)
            .cloned()
            .map(Ok)
            .collect();
        drop(inner);
        Ok(futures::stream::iter(rows))
    }

    async fn read_all(&self, from: Option<TestAllPos>) -> Result<Self::AllStream, Self::Error> {
        let inner = self.inner.lock().await;
        let rows: Vec<Result<(TestAllPos, PersistedEnvelope), TestStoreError>> = inner
            .all
            .iter()
            .filter(|(pos, _)| from.is_none_or(|f| *pos > f))
            .cloned()
            .map(Ok)
            .collect();
        drop(inner);
        Ok(futures::stream::iter(rows))
    }
}

/// Store-wide generation wait: any commit wakes every armed waiter (a
/// permitted spurious wake), and `mark_unchanged` at arm time pins the seen
/// generation so a wake between `arm` and the await is never lost.
#[derive(Debug)]
pub struct TestWakeReg {
    rx: watch::Receiver<u64>,
}

impl WakeRegistration for TestWakeReg {
    fn arm(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut rx = self.rx.clone();
        rx.mark_unchanged();
        async move {
            let _ = rx.changed().await;
        }
    }
}

impl WakeSource for TestStore {
    type Registration = TestWakeReg;
    type Error = Infallible;

    fn register(&self, _stream: Option<&[u8]>) -> Result<TestWakeReg, Infallible> {
        Ok(TestWakeReg {
            rx: self.wake_tx.subscribe(),
        })
    }

    fn wake(&self, _stream: &[u8]) {
        self.wake_tx.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }
}
