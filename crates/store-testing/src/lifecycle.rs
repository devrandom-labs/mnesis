//! Lifecycle conformance (opt-in): close → reopen must preserve events,
//! versions, and the `$all` position watermark. In-memory adapters have
//! nothing to reopen and skip this module.

use core::future::Future;
use core::time::Duration;

use futures::StreamExt;
use futures::pin_mut;
use nexus::Version;
use nexus_store::store::RawEventStore;
use nexus_store::wake::WakeSource;
use nexus_store::{Step, StreamKey, Subscription};
use tokio::time::timeout;

use crate::row::{
    ConformanceRow, SubId, append_event, append_rows, drain_all, drain_stream, envelope_for,
};

/// Everything written before close reads back identically after reopen.
pub async fn check_reopen_preserves_events<S, C, O, OFut, R, RFut>(open: &O, reopen: &R)
where
    S: RawEventStore + WakeSource,
    C: Send,
    O: Fn() -> OFut + Send + Sync,
    OFut: Future<Output = (S, C)> + Send,
    R: Fn(S, C) -> RFut + Send + Sync,
    RFut: Future<Output = (S, C)> + Send,
{
    let (opened, ctx) = open().await;
    let id = StreamKey::from_slice(b"persist");
    let rows = vec![
        ConformanceRow::new(1, "Created", vec![1]).with_metadata(vec![7]),
        ConformanceRow::new(2, "Updated", vec![2]).with_schema_version(3),
    ];
    append_rows(&opened, &id, &rows).await;
    let before = drain_stream(&opened, &id, Version::INITIAL).await;

    let (reopened, _ctx) = reopen(opened, ctx).await;
    let after = drain_stream(&reopened, &id, Version::INITIAL).await;
    assert_eq!(
        after, before,
        "reopen must preserve every event byte-for-byte"
    );
}

/// The `$all` position watermark survives reopen: a post-reopen append lands
/// strictly after every pre-close position (a reset counter would violate
/// resume and corrupt projections).
pub async fn check_reopen_preserves_position_watermark<S, C, O, OFut, R, RFut>(open: &O, reopen: &R)
where
    S: RawEventStore + WakeSource,
    C: Send,
    O: Fn() -> OFut + Send + Sync,
    OFut: Future<Output = (S, C)> + Send,
    R: Fn(S, C) -> RFut + Send + Sync,
    RFut: Future<Output = (S, C)> + Send,
{
    let (opened, ctx) = open().await;
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    append_event(&opened, &a, 1, b"a1").await;
    append_event(&opened, &b, 1, b"b1").await;
    let before = drain_all(&opened, None).await;
    let last = before.last().expect("non-empty").0;

    let (reopened, _ctx) = reopen(opened, ctx).await;
    append_event(&reopened, &a, 2, b"a2").await;

    let resumed = drain_all(&reopened, Some(last)).await;
    assert_eq!(
        resumed.len(),
        1,
        "resume from the pre-close watermark must yield only the new event"
    );
    assert_eq!(resumed[0].1, b"a2".to_vec());
    assert!(
        resumed[0].0 > last,
        "post-reopen position {:?} must be strictly after the pre-close last {:?} — the watermark must survive reopen",
        resumed[0].0,
        last,
    );
}

/// Optimistic-concurrency state survives reopen: a stale expectation still
/// conflicts against the persisted head.
pub async fn check_reopen_conflict_state_intact<S, C, O, OFut, R, RFut>(open: &O, reopen: &R)
where
    S: RawEventStore + WakeSource,
    C: Send,
    O: Fn() -> OFut + Send + Sync,
    OFut: Future<Output = (S, C)> + Send,
    R: Fn(S, C) -> RFut + Send + Sync,
    RFut: Future<Output = (S, C)> + Send,
{
    let (opened, ctx) = open().await;
    let id = StreamKey::from_slice(b"head");
    append_event(&opened, &id, 1, b"p1").await;
    append_event(&opened, &id, 2, b"p2").await;

    let (reopened, _ctx) = reopen(opened, ctx).await;
    let stale = envelope_for(&ConformanceRow::new(1, "E", vec![9]));
    reopened
        .append(&id, None, &[stale])
        .await
        .expect_err("the persisted head must still conflict after reopen");
    // The corrected append succeeds.
    append_event(&reopened, &id, 3, b"p3").await;
    let got = drain_stream(&reopened, &id, Version::INITIAL).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(versions, vec![1, 2, 3]);
}

/// A subscription opened after reopen catches up over the pre-close backlog.
pub async fn check_reopen_subscription_catches_up<S, C, O, OFut, R, RFut>(open: &O, reopen: &R)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    O: Fn() -> OFut + Send + Sync,
    OFut: Future<Output = (S, C)> + Send,
    R: Fn(S, C) -> RFut + Send + Sync,
    RFut: Future<Output = (S, C)> + Send,
{
    let (opened, ctx) = open().await;
    let id = SubId::new("reopen-sub");
    for v in 1..=3u64 {
        append_event(&opened, &id.key(), v, b"p").await;
    }

    let (reopened, _ctx) = reopen(opened, ctx).await;
    let store = reopened.into_store();
    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, None)
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    let mut versions = Vec::new();
    while let Step::Event(env) = timeout(Duration::from_secs(10), stream.next())
        .await
        .expect("catch-up after reopen must not hang")
        .expect("subscription must not end")
        .unwrap_or_else(|e| panic!("item errored: {e:?}"))
    {
        versions.push(env.version().as_u64());
    }
    assert_eq!(versions, vec![1, 2, 3], "reopen backlog must replay fully");
}
