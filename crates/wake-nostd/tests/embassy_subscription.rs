//! #302 acceptance: `GlobalWake` drives a real `Subscription` under a
//! `no_std` executor (embassy). The store double delegates `RawEventStore`
//! to `InMemoryStore` (rule 8: reuse the shipped adapter, don't
//! reimplement) and `WakeSource` to `GlobalWake`; `append` wakes AFTER the
//! inner commit returns (the MUST-wake-after-durable-commit ordering).
//!
//! Flow: seed a 2-event backlog → embassy task subscribes and reports each
//! `Step` over a std mpsc channel → assert backlog, then `CaughtUp`, then
//! append from OUTSIDE the executor and assert the live event arrives —
//! the cross-thread wake path through `GlobalWake`.

#![allow(clippy::unwrap_used, reason = "test code")]
#![allow(clippy::expect_used, reason = "test code")]

use std::sync::mpsc;
use std::time::Duration;

use embassy_executor::Executor;
use futures::StreamExt;
use mnesis::Version;
use mnesis_inmemory::{
    InMemoryAllPos, InMemoryAllStream, InMemoryStore, InMemoryStoreError, InMemoryStream,
};
use mnesis_store::wake::WakeSource;
use mnesis_store::{
    AppendError, PendingBatch, RawEventStore, Step, Store, StreamKey, Subscription,
    pending_envelope,
};
use mnesis_wake_nostd::GlobalWake;

const MUST_DELIVER: Duration = Duration::from_secs(5);
const STREAM: &[u8] = b"device";

/// On-device store shape: the in-memory adapter for persistence, the
/// `no_std` `GlobalWake` for wake routing.
struct DeviceStore {
    inner: InMemoryStore,
    wake: GlobalWake,
}

impl RawEventStore for DeviceStore {
    type Error = InMemoryStoreError;
    type Stream = InMemoryStream;
    type AllPosition = InMemoryAllPos;
    type AllStream = InMemoryAllStream;

    async fn append(
        &self,
        id: &StreamKey,
        expected_version: Option<Version>,
        envelopes: PendingBatch<'_>,
    ) -> Result<Self::AllPosition, AppendError<Self::Error>> {
        let position = self.inner.append(id, expected_version, envelopes).await?;
        // Wake only after the commit returned Ok — the WakeSource contract.
        WakeSource::wake(&self.wake, id.as_ref());
        Ok(position)
    }

    async fn read_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> Result<Self::Stream, Self::Error> {
        self.inner.read_stream(id, from).await
    }

    async fn read_all(
        &self,
        from: Option<Self::AllPosition>,
    ) -> Result<Self::AllStream, Self::Error> {
        self.inner.read_all(from).await
    }
}

impl WakeSource for DeviceStore {
    type Registration = <GlobalWake as WakeSource>::Registration;
    type Error = <GlobalWake as WakeSource>::Error;

    fn register(&self, stream: Option<&[u8]>) -> Result<Self::Registration, Self::Error> {
        self.wake.register(stream)
    }

    fn wake(&self, stream: &[u8]) {
        WakeSource::wake(&self.wake, stream);
    }
}

enum Msg {
    Event(u64),
    CaughtUp,
}

#[embassy_executor::task]
async fn drive(store: Store<DeviceStore>, tx: mpsc::Sender<Msg>) {
    let sub = Subscription::new(&store);
    let raw_stream = match sub.subscribe(&StreamKey::from_slice(STREAM), None) {
        Ok(stream) => stream,
        Err(never) => match never {},
    };
    let mut stream = core::pin::pin!(raw_stream);
    while let Some(item) = stream.next().await {
        let msg = match item.expect("in-memory reads never fail") {
            Step::Event(env) => Msg::Event(env.version().as_u64()),
            Step::CaughtUp => Msg::CaughtUp,
        };
        if tx.send(msg).is_err() {
            return; // test thread is done asserting; wind the task down
        }
    }
}

async fn append_one(store: &Store<DeviceStore>, version: u64, expected_raw: Option<u64>) {
    let env = pending_envelope(Version::new(version).expect("nonzero version"))
        .event_type("DeviceEvent")
        .payload(b"p".to_vec())
        .build()
        .expect("valid envelope");
    let expected_version = expected_raw.and_then(Version::new);
    store
        .append(
            &StreamKey::from_slice(STREAM),
            expected_version,
            PendingBatch::of(&env),
        )
        .await
        .expect("append must succeed");
}

#[test]
fn embassy_executor_drives_catch_up_then_live_tail() {
    let store = DeviceStore {
        inner: InMemoryStore::new(),
        wake: GlobalWake::new(),
    }
    .into_store();

    // Seed a 2-event backlog before subscribing.
    futures::executor::block_on(async {
        append_one(&store, 1, None).await;
        append_one(&store, 2, Some(1)).await;
    });

    let (tx, rx) = mpsc::channel();
    let sub_store = store.clone();
    // `Executor::run` never returns, so this thread can never be joined —
    // `std::thread::scope` (which blocks until its spawned threads finish)
    // does not fit. The thread is intentionally detached; the leaked
    // executor and the thread itself are reclaimed at process exit
    // (nextest runs one process per test).
    #[allow(
        clippy::disallowed_methods,
        reason = "the embassy executor loop never returns, so there is no join point for \
                  `std::thread::scope` to use; a detached thread is unavoidable here"
    )]
    std::thread::spawn(move || {
        let executor: &'static mut Executor = Box::leak(Box::new(Executor::new()));
        executor.run(|spawner| spawner.spawn(drive(sub_store, tx).unwrap()));
    });

    // Catch-up: the backlog in order, then the boundary marker.
    assert!(matches!(rx.recv_timeout(MUST_DELIVER), Ok(Msg::Event(1))));
    assert!(matches!(rx.recv_timeout(MUST_DELIVER), Ok(Msg::Event(2))));
    assert!(matches!(rx.recv_timeout(MUST_DELIVER), Ok(Msg::CaughtUp)));

    // Live: append from OUTSIDE the embassy executor — the wake must cross
    // threads through GlobalWake and rouse the parked subscription.
    futures::executor::block_on(append_one(&store, 3, Some(2)));
    assert!(matches!(rx.recv_timeout(MUST_DELIVER), Ok(Msg::Event(3))));
}
