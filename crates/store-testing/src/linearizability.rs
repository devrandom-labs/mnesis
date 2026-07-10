//! Linearizability/Isolation conformance: genuinely-overlapping writers and a
//! parked subscriber. Real overlap via `tokio::spawn` + `Barrier` (CLAUDE
//! rule 8 — never sequential-then-check).

use core::future::Future;
use core::time::Duration;
use std::sync::Arc;

use futures::pin_mut;
use nexus::Version;
use nexus_store::store::RawEventStore;
use nexus_store::wake::WakeSource;
use nexus_store::{AppendError, Step, StreamKey, Subscription};
use tokio::sync::Barrier;
use tokio::time::timeout;

use crate::row::{
    ConformanceRow, SubId, append_event, append_rows, assert_strictly_increasing, drain_all,
    drain_stream, envelope_for,
};

const WAIT: Duration = Duration::from_secs(10);
const WRITERS: usize = 8;

/// N overlapping appenders race the same fresh stream with the same
/// expectation: exactly ONE wins, every loser sees Conflict, and the store
/// holds exactly the winner's event.
pub async fn check_concurrent_same_stream_single_winner<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = Arc::new(raw);
    let id = StreamKey::from_slice(b"race");
    let barrier = Arc::new(Barrier::new(WRITERS));

    let mut handles = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let task_store = Arc::clone(&store);
        let task_barrier = Arc::clone(&barrier);
        let task_id = id.clone();
        handles.push(tokio::spawn(async move {
            let payload = vec![u8::try_from(i).unwrap_or(0)];
            let env = envelope_for(&ConformanceRow::new(1, "E", payload));
            task_barrier.wait().await;
            task_store.append(&task_id, None, &[env]).await
        }));
    }

    let mut winners = Vec::new();
    let mut conflicts = 0;
    for (i, h) in handles.into_iter().enumerate() {
        match h.await.expect("writer task panicked") {
            Ok(()) => winners.push(i),
            Err(AppendError::Conflict { .. }) => conflicts += 1,
            Err(other) => panic!("writer {i} hit a non-conflict error: {other:?}"),
        }
    }
    assert_eq!(
        winners.len(),
        1,
        "exactly one concurrent appender must win, got {winners:?}"
    );
    assert_eq!(conflicts, WRITERS - 1, "every loser must see Conflict");

    let got = drain_stream(store.as_ref(), &id, Version::INITIAL).await;
    assert_eq!(got.len(), 1, "store must hold exactly the winner's event");
    assert_eq!(
        got[0].payload,
        vec![u8::try_from(winners[0]).unwrap_or(0)],
        "the persisted event must be the winner's",
    );
}

/// Overlapping appenders on DISTINCT streams never conflict; every event
/// lands; `$all` holds all of them with strictly increasing positions.
pub async fn check_concurrent_distinct_streams_all_land<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    const PER_STREAM: u64 = 5;
    let (raw, _guard) = factory().await;
    let store = Arc::new(raw);
    let barrier = Arc::new(Barrier::new(WRITERS));

    let mut handles = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let task_store = Arc::clone(&store);
        let task_barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let id = StreamKey::from_slice(format!("s{i}").as_bytes());
            task_barrier.wait().await;
            for v in 1..=PER_STREAM {
                append_event(task_store.as_ref(), &id, v, format!("s{i}v{v}").as_bytes()).await;
            }
        }));
    }
    for h in handles {
        h.await.expect("writer task panicked");
    }

    let all = drain_all(store.as_ref(), None).await;
    assert_eq!(
        all.len(),
        WRITERS * usize::try_from(PER_STREAM).unwrap_or(usize::MAX),
        "every concurrently appended event must land in $all",
    );
    assert_strictly_increasing(&all);

    for i in 0..WRITERS {
        let id = StreamKey::from_slice(format!("s{i}").as_bytes());
        let got = drain_stream(store.as_ref(), &id, Version::INITIAL).await;
        let want: Vec<Vec<u8>> = (1..=PER_STREAM)
            .map(|v| format!("s{i}v{v}").into_bytes())
            .collect();
        let payloads: Vec<Vec<u8>> = got.iter().map(|r| r.payload.clone()).collect();
        assert_eq!(
            payloads, want,
            "stream s{i} must hold its own events in order"
        );
    }
}

/// Wake-after-idle: a subscriber parked at `CaughtUp` is woken by a later
/// append from another task — the lost-wakeup race the arm-before-rescan
/// discipline exists to prevent.
pub async fn check_wake_after_idle<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("wake-idle");
    append_event(&store, &id.key(), 1, b"p1").await;

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, None)
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    // Drain to CaughtUp.
    loop {
        match timeout(WAIT, futures::StreamExt::next(&mut stream))
            .await
            .expect("backlog must not hang")
            .expect("subscription must not end")
            .unwrap_or_else(|e| panic!("item errored: {e:?}"))
        {
            Step::CaughtUp => break,
            Step::Event(_) => {}
        }
    }

    // Park, then append from another task after a real delay.
    let writer_store = store.clone();
    let key = id.key();
    let writer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        append_event(&writer_store, &key, 2, b"p2").await;
    });

    let woke = timeout(WAIT, futures::StreamExt::next(&mut stream))
        .await
        .expect("parked subscriber was never woken — lost wakeup")
        .expect("subscription must not end")
        .unwrap_or_else(|e| panic!("item errored: {e:?}"));
    match woke {
        Step::Event(env) => assert_eq!(env.version().as_u64(), 2, "wake must deliver v2"),
        Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
    }
    writer.await.expect("writer task panicked");
}

/// Appends racing the catch-up→live boundary are neither lost nor duplicated,
/// and `CaughtUp` is still emitted exactly once.
pub async fn check_caught_up_boundary_race<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    const BACKLOG: u64 = 100;
    const LIVE: u64 = 100;
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("boundary-race");
    let rows: Vec<_> = (1..=BACKLOG)
        .map(|v| ConformanceRow::new(v, "E", vec![]))
        .collect();
    append_rows(&store, &id.key(), &rows).await;

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, None)
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    // Writer races the reader's catch-up.
    let writer_store = store.clone();
    let key = id.key();
    let writer = tokio::spawn(async move {
        for v in (BACKLOG + 1)..=(BACKLOG + LIVE) {
            append_event(&writer_store, &key, v, b"live").await;
        }
    });

    // Loop until BOTH every event has arrived AND `CaughtUp` has been seen —
    // exiting on event-count alone would miss a `CaughtUp` that lands after
    // the writer has raced ahead of the reader's catch-up scan and delivered
    // the entire backlog+live run before the boundary is detected.
    let total = BACKLOG + LIVE;
    let want_events = usize::try_from(total).unwrap_or(usize::MAX);
    let mut versions = Vec::with_capacity(want_events);
    let mut caught_up = 0u32;
    while versions.len() < want_events || caught_up == 0 {
        match timeout(WAIT, futures::StreamExt::next(&mut stream))
            .await
            .expect("boundary race hung — event lost across the catch-up→live seam")
            .expect("subscription must not end")
            .unwrap_or_else(|e| panic!("item errored: {e:?}"))
        {
            Step::Event(env) => versions.push(env.version().as_u64()),
            Step::CaughtUp => caught_up += 1,
        }
    }
    writer.await.expect("writer task panicked");

    assert_eq!(
        caught_up, 1,
        "CaughtUp must be emitted exactly once, got {caught_up}"
    );
    let want: Vec<u64> = (1..=total).collect();
    assert_eq!(
        versions, want,
        "all {total} events must arrive exactly once, in order, across the boundary",
    );
}
