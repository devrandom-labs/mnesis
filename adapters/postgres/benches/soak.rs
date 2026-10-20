//! Native mixed-workload soak; short explicit durations are smoke runs only.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native benchmark assertions"
)]

#[path = "../tests/support/cluster.rs"]
mod cluster;
#[path = "../tests/support/memory.rs"]
mod memory;

use std::io::{Write, stderr};
use std::mem::replace;
use std::num::{NonZeroU32, NonZeroU64};
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_postgres::{Limits, PgAllPos, PostgresError, PostgresStore};
use mnesis_store::checkpoint::{
    CheckpointHydrated, CheckpointMode, CheckpointStore, CheckpointWrite,
};
use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_store::{
    AppendError, BatchSize, PendingBatch, PendingEnvelope, RawEventStore, Store, StreamKey,
    Subscription, pending_envelope,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{Error as SqlxError, PgPool};
use tokio::sync::Barrier;
use tokio::task::{JoinSet, yield_now};
use tokio::time::timeout;

use cluster::Cluster;
use memory::{LIVE, PEAK, TOTAL};

const CLIENTS: u32 = 16;
const POOL: u32 = 24;
const RECOVERY: Duration = Duration::from_secs(3);
const INTERVAL: Duration = Duration::from_secs(60);

fn limits() -> Limits {
    Limits {
        rows: BatchSize::new(128).unwrap(),
        bytes: NonZeroU32::new(1_048_576).unwrap(),
    }
}

fn consumer(index: u32) -> StreamKey {
    StreamKey::from_bytes(format!("consumer-{index}"))
}

async fn pool(options: &PgConnectOptions, name: &str, capacity: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(capacity)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(
            options
                .clone()
                .application_name(name)
                .options([("statement_timeout", "5s"), ("lock_timeout", "1s")]),
        )
        .await
        .unwrap()
}

struct Database {
    cluster: Cluster,
    writer: PostgresStore,
    reader: PostgresStore,
    write_pool: PgPool,
    read_pool: PgPool,
    control: PgPool,
    fault: i64,
}

impl Database {
    async fn open() -> Self {
        let cluster = Cluster::initialize();
        cluster.launch(&format!(
            "-k {} -p 55438 -c listen_addresses=''",
            cluster.socket().display()
        ));
        let options = PgConnectOptions::new()
            .host(cluster.socket().to_str().unwrap())
            .port(55438)
            .username("mnesis")
            .database("postgres")
            .ssl_mode(PgSslMode::Disable);
        let control = pool(&options, "mnesis-soak-control", 4).await;
        PostgresStore::migrate(&control).await.unwrap();
        let settings: (String, String, String, String) = sqlx::query_as(
            "SELECT current_setting('server_version'), current_setting('fsync'), \
             current_setting('full_page_writes'), current_setting('synchronous_commit')",
        )
        .fetch_one(&control)
        .await
        .unwrap();
        assert_eq!(
            (&settings.1, &settings.2, &settings.3),
            (&"on".to_owned(), &"on".to_owned(), &"on".to_owned())
        );
        writeln!(
            stderr().lock(),
            "soak server/settings={settings:?}, clients={CLIENTS}, pool={POOL}, pages={:?}",
            limits()
        )
        .unwrap();
        let write_pool = pool(&options, "mnesis-soak-writer", POOL).await;
        let read_pool = pool(&options, "mnesis-soak-reader", POOL).await;
        let writer = PostgresStore::from_pool_with_limits(write_pool.clone(), limits())
            .await
            .unwrap();
        let reader = PostgresStore::from_pool_with_limits(read_pool.clone(), limits())
            .await
            .unwrap();
        let fault = sqlx::query_scalar(
            "INSERT INTO mnesis.keys(value) VALUES ('unnotified'::bytea) RETURNING number",
        )
        .fetch_one(&control)
        .await
        .unwrap();
        Self {
            cluster,
            writer,
            reader,
            write_pool,
            read_pool,
            control,
            fault,
        }
    }

    async fn close(self) {
        drop(self.writer);
        drop(self.reader);
        timeout(RECOVERY, self.write_pool.close()).await.unwrap();
        timeout(RECOVERY, self.read_pool.close()).await.unwrap();
        assert_eq!((self.write_pool.size(), self.read_pool.size()), (0, 0));
        timeout(RECOVERY, async {
            loop {
                let remaining: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity \
                     WHERE application_name IN ('mnesis-soak-reader', 'mnesis-soak-writer')",
                )
                .fetch_one(&self.control)
                .await
                .unwrap();
                if remaining == 0 {
                    break;
                }
                yield_now().await;
            }
        })
        .await
        .expect("quiescence must release every client backend");
        self.control.close().await;
        drop(self.cluster);
    }
}

async fn listener(control: &PgPool, excluded: &[i32]) -> i32 {
    timeout(RECOVERY, async {
        loop {
            let pid: Option<i32> = sqlx::query_scalar(
                "SELECT pid FROM pg_stat_activity WHERE application_name = 'mnesis-soak-reader' \
                 AND state = 'idle' AND query LIKE 'LISTEN %mnesis_events%' \
                 AND pid <> ALL($1::int[]) LIMIT 1",
            )
            .bind(excluded)
            .fetch_optional(control)
            .await
            .unwrap();
            if let Some(value) = pid {
                break value;
            }
            yield_now().await;
        }
    })
    .await
    .expect("the real listener must subscribe or reconnect")
}

#[derive(Debug, Default)]
struct Counters {
    rounds: u64,
    events: u64,
    faults: u64,
    rejections: u64,
}

async fn writers(store: &PostgresStore, counts: &mut Counters, length: u64) {
    let key = StreamKey::from_bytes(format!("round-{}", counts.rounds));
    let events: Arc<Vec<PendingEnvelope>> = Arc::new(
        (1..=length)
            .map(|offset| {
                pending_envelope(Version::new(offset).unwrap())
                    .event_type("Soak")
                    .payload(
                        counts
                            .events
                            .checked_add(offset)
                            .unwrap()
                            .to_be_bytes()
                            .to_vec(),
                    )
                    .build()
                    .unwrap()
            })
            .collect(),
    );
    let start = Arc::new(Barrier::new(usize::try_from(CLIENTS).unwrap()));
    let mut pending = JoinSet::new();
    for _ in 0..CLIENTS {
        let target = key.clone();
        let backing = store.clone();
        let ready = Arc::clone(&start);
        let batch = Arc::clone(&events);
        pending.spawn(async move {
            ready.wait().await;
            backing
                .append(&target, None, PendingBatch::new(batch.as_slice()).unwrap())
                .await
        });
    }
    let mut committed = 0_u32;
    let mut conflicts = 0_u32;
    let mut rejected = 0_u32;
    timeout(Duration::from_secs(10), async {
        while let Some(result) = pending.join_next().await {
            match result.unwrap() {
                Ok(_) => committed = committed.checked_add(1).unwrap(),
                Err(AppendError::Conflict { .. }) => conflicts = conflicts.checked_add(1).unwrap(),
                Err(AppendError::Store(PostgresError::Sqlx(SqlxError::Database(error))))
                    if error.code().as_deref() == Some("55P03") =>
                {
                    rejected = rejected.checked_add(1).unwrap();
                }
                other => assert!(other.is_ok(), "unexpected write result: {other:?}"),
            }
        }
    })
    .await
    .expect("competing writes must settle without an unknown retry");
    assert_eq!(
        (committed, conflicts.checked_add(rejected).unwrap()),
        (1, CLIENTS.checked_sub(1).unwrap())
    );
    counts.events = counts.events.checked_add(length).unwrap();
    counts.rejections = counts.rejections.checked_add(u64::from(rejected)).unwrap();
}

fn loaded(
    checkpoint: CheckpointHydrated<Vec<u8>, PgAllPos>,
) -> (Option<NonZeroU64>, Option<PgAllPos>, u64) {
    if let CheckpointHydrated::Found {
        revision,
        position,
        state,
    } = checkpoint
    {
        (
            Some(revision),
            Some(position),
            u64::from_be_bytes(state.as_slice().try_into().unwrap()),
        )
    } else {
        assert_eq!(checkpoint, CheckpointHydrated::Absent);
        (None, None, 0)
    }
}

async fn advance(store: PostgresStore, key: StreamKey, expected: u64) {
    let initial = store
        .hydrate_checkpoint(&key, NonZeroU32::MIN)
        .await
        .unwrap();
    let (revision, from, mut count) = loaded(initial);
    let mut last = from;
    let backing = Store::new(store.clone());
    let subscription = Subscription::new(&backing);
    {
        let mut cursor = pin!(subscription.subscribe_all(from).unwrap());
        timeout(RECOVERY, async {
            while count < expected {
                let step = cursor
                    .try_next()
                    .await
                    .unwrap()
                    .expect("live subscription cannot end");
                if let Some((position, _origin, event)) = step.event() {
                    assert!(last.is_none_or(|previous| position > previous));
                    count = count.checked_add(1).unwrap();
                    assert_eq!(event.payload(), count.to_be_bytes());
                    last = Some(position);
                }
            }
        })
        .await
        .expect("known committed round must become visible within recovery deadline");
    }
    assert_eq!(count, expected);
    let position = last.expect("each round adds at least one event");
    let state = count.to_be_bytes().to_vec();
    let saved = store
        .commit_checkpoint(
            &key,
            CheckpointWrite {
                expected: revision,
                schema_version: NonZeroU32::MIN,
                position,
                state: &state,
                mode: CheckpointMode::Advance,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        saved.get(),
        revision
            .map_or(Some(1), |value| value.get().checked_add(1))
            .unwrap()
    );
    assert_eq!(
        store
            .hydrate_checkpoint(&key, NonZeroU32::MIN)
            .await
            .unwrap(),
        CheckpointHydrated::Found {
            revision: saved,
            position,
            state
        }
    );
}

async fn readers(store: &PostgresStore, count: u64) {
    let mut pending = JoinSet::new();
    for index in 0..CLIENTS {
        pending.spawn(advance(store.clone(), consumer(index), count));
    }
    timeout(Duration::from_secs(10), async {
        while let Some(result) = pending.join_next().await {
            result.unwrap();
        }
    })
    .await
    .expect("all reconstructed consumers must save their exact pair");
}

async fn rebuild(store: &PostgresStore, expected: u64) {
    let mut cursor = store.read_all(None).await.unwrap();
    let mut count = 0_u64;
    let mut last = None;
    while let Some((position, _origin, event)) = cursor.try_next().await.unwrap() {
        assert!(last.is_none_or(|previous| position > previous));
        count = count.checked_add(1).unwrap();
        assert_eq!(event.payload(), count.to_be_bytes());
        last = Some(position);
    }
    assert_eq!(count, expected);
}

async fn recover(database: &mut Database, count: u64) -> u64 {
    let old_pid = listener(&database.control, &[]).await;
    let replacement = PostgresStore::from_pool_with_limits(database.read_pool.clone(), limits())
        .await
        .unwrap();
    drop(replace(&mut database.reader, replacement));
    let current_pid = listener(&database.control, &[old_pid]).await;
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(current_pid)
        .fetch_one(&database.control)
        .await
        .unwrap();
    assert!(terminated);
    listener(&database.control, &[old_pid, current_pid]).await;
    let previous = database
        .reader
        .hydrate_checkpoint(&consumer(0), NonZeroU32::MIN)
        .await
        .unwrap();
    let (_revision, from, restored) = loaded(previous);
    assert_eq!(restored, count);
    let registration = database.reader.register(None).unwrap();
    let wake = registration.arm();
    let next = count.checked_add(1).unwrap();
    let payload = next.to_be_bytes();
    timeout(RECOVERY, async {
        let written = sqlx::query(
            "INSERT INTO mnesis.events(stream_id, version, event_type, schema_version, payload) \
             VALUES ($1, (SELECT COALESCE(max(version), 0) + 1 FROM mnesis.events WHERE stream_id = $1), \
             'Soak', 1, $2)",
        ).bind(database.fault).bind(payload.as_slice()).execute(&database.control).await.unwrap();
        assert_eq!(written.rows_affected(), 1);
        wake.await;
        let mut probe = database.reader.read_all(from).await.unwrap();
        assert_eq!(probe.try_next().await.unwrap().unwrap().2.payload(), payload);
        assert!(probe.try_next().await.unwrap().is_none());
    }).await.expect("unnotified final commit must recover without another write");
    next
}

async fn report(database: &Database, started: Instant, counts: &Counters) {
    let connections: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE application_name LIKE 'mnesis-soak-%'",
    )
    .fetch_one(&database.control)
    .await
    .unwrap();
    assert!(connections <= i64::from(POOL.checked_mul(2).unwrap().checked_add(4).unwrap()));
    writeln!(stderr().lock(), "soak elapsed={:?} counts={counts:?} connections={connections} live={} peak={} allocated_bytes={}",
        started.elapsed(), LIVE.load(Ordering::Relaxed), PEAK.load(Ordering::Relaxed), TOTAL.load(Ordering::Relaxed)).unwrap();
}

#[tokio::main]
async fn main() {
    let seconds =
        std::env::var("MNESIS_SOAK_SECONDS").map_or(1800, |value| value.parse::<u64>().unwrap());
    assert!(seconds > 0);
    let duration = Duration::from_secs(seconds);
    let mut database = Database::open().await;
    let mut counts = Counters::default();
    writers(&database.writer, &mut counts, 8).await;
    readers(&database.reader, counts.events).await;
    let baseline = LIVE.load(Ordering::Relaxed);
    let allocated = TOTAL.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let started = Instant::now();
    let mut next_fault = Duration::ZERO;
    while started.elapsed() < duration {
        counts.rounds = counts.rounds.checked_add(1).unwrap();
        let length = [1, 8][usize::try_from(counts.rounds % 2).unwrap()];
        writers(&database.writer, &mut counts, length).await;
        readers(&database.reader, counts.events).await;
        if started.elapsed() >= next_fault {
            counts.events = recover(&mut database, counts.events).await;
            readers(&database.reader, counts.events).await;
            rebuild(&database.reader, counts.events).await;
            counts.faults = counts.faults.checked_add(1).unwrap();
            next_fault = next_fault.checked_add(INTERVAL).unwrap();
            report(&database, started, &counts).await;
        }
    }
    assert!(started.elapsed() >= duration);
    assert!(counts.faults >= seconds.div_ceil(INTERVAL.as_secs()));
    let persisted: i64 = sqlx::query_scalar("SELECT count(*) FROM mnesis.events")
        .fetch_one(&database.control)
        .await
        .unwrap();
    assert_eq!(u64::try_from(persisted).unwrap(), counts.events);
    report(&database, started, &counts).await;
    let peak = PEAK.load(Ordering::Relaxed);
    let budget = usize::try_from(CLIENTS)
        .unwrap()
        .checked_mul(usize::try_from(limits().bytes.get()).unwrap())
        .unwrap()
        .checked_mul(4)
        .unwrap()
        .checked_add(8_388_608)
        .unwrap();
    assert!(
        peak <= baseline.checked_add(budget).unwrap(),
        "unbounded client allocations: {baseline}/{peak}/{budget}"
    );
    database.close().await;
    let retained = LIVE.load(Ordering::Relaxed);
    assert!(retained <= baseline.checked_add(8_388_608).unwrap());
    writeln!(stderr().lock(), "soak complete seconds={seconds} elapsed={:?} counts={counts:?} baseline={baseline} peak={peak} retained={retained} allocated_bytes={}",
        started.elapsed(), TOTAL.load(Ordering::Relaxed).checked_sub(allocated).unwrap()).unwrap();
}
