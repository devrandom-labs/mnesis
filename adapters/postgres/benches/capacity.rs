//! Native capacity measurements; preparation and warmup are outside samples.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native benchmark assertions"
)]

#[path = "../tests/support/cluster.rs"]
mod cluster;
#[path = "../tests/support/memory.rs"]
mod memory;

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Write, stderr};
use std::num::NonZeroU32;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use futures::{StreamExt, TryStreamExt};
use mnesis::Version;
use mnesis_postgres::{Limits, PgAllPos, PostgresStore};
use mnesis_store::{
    AppendError, BatchSize, PendingBatch, PendingEnvelope, RawEventStore, StreamKey,
    pending_envelope,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use sqlx::{PgConnection, PgPool, Postgres, QueryBuilder};
use tokio::sync::Barrier;
use tokio::task::{JoinSet, yield_now};
use tokio::time::timeout;

use cluster::Cluster;
use memory::{COUNT, LIVE, PEAK, TOTAL};

const PAGE: u64 = 128;
const BYTES: u32 = 1_048_576;
const POOL: u32 = 24;
const SAMPLES: u32 = 100;
const WINDOWS: u32 = 3;

#[derive(Clone, Copy, Debug)]
enum Scan {
    Stream,
    All,
}
#[derive(Clone, Copy, Debug)]
enum Edge {
    Start,
    End,
}

struct Fixture {
    cluster: Cluster,
    pool: PgPool,
    store: PostgresStore,
    total: u64,
    length: u64,
    payload: Vec<u8>,
    tail: Option<PgAllPos>,
}

fn key(index: u64) -> StreamKey {
    StreamKey::from_bytes(format!("capacity-{index}"))
}

fn event(ordinal: u64, version: u64, template: &[u8]) -> PendingEnvelope {
    let mut payload = template.to_vec();
    payload[..8].copy_from_slice(&ordinal.to_be_bytes());
    pending_envelope(Version::new(version).unwrap())
        .event_type("Capacity")
        .payload(payload)
        .build()
        .unwrap()
}

impl Fixture {
    async fn open(total: u64, length: u64, payload: usize) -> Self {
        let cluster = Cluster::initialize();
        cluster.launch(&format!(
            "-k {} -p 55439 -c listen_addresses=''",
            cluster.socket().display()
        ));
        let options = PgConnectOptions::new()
            .host(cluster.socket().to_str().unwrap())
            .port(55439)
            .username("mnesis")
            .database("postgres")
            .application_name("mnesis-capacity")
            .ssl_mode(PgSslMode::Disable)
            .options([("statement_timeout", "30s"), ("lock_timeout", "5s")]);
        let pool = PgPoolOptions::new()
            .max_connections(POOL)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options)
            .await
            .unwrap();
        PostgresStore::migrate(&pool).await.unwrap();
        let settings: (String, String, String, String) = sqlx::query_as(
            "SELECT current_setting('server_version'), current_setting('fsync'), current_setting('full_page_writes'), current_setting('synchronous_commit')"
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(
            (&settings.1, &settings.2, &settings.3),
            (&"on".to_owned(), &"on".to_owned(), &"on".to_owned())
        );
        let store = PostgresStore::from_pool_with_limits(
            pool.clone(),
            Limits {
                rows: BatchSize::new(usize::try_from(PAGE).unwrap()).unwrap(),
                bytes: NonZeroU32::new(BYTES).unwrap(),
            },
        )
        .await
        .unwrap();
        writeln!(stderr().lock(), "capacity profile total={total} stream_length={length} payload={payload} pages={PAGE}/{BYTES} pool={POOL} settings={settings:?}").unwrap();
        let mut template = vec![0; payload];
        File::open("/dev/urandom")
            .unwrap()
            .read_exact(&mut template)
            .unwrap();
        writeln!(stderr().lock(), "capacity payload_template={template:02x?}").unwrap();
        Self {
            cluster,
            pool,
            store,
            total,
            length,
            payload: template,
            tail: None,
        }
    }

    async fn seed(&mut self) {
        for start in (0..self.total).step_by(100) {
            let stream = key(start / self.length);
            let prior = start % self.length;
            let batch: Vec<_> = (1..=100)
                .map(|offset| {
                    event(
                        start.checked_add(offset).unwrap(),
                        prior.checked_add(offset).unwrap(),
                        &self.payload,
                    )
                })
                .collect();
            let assigned = self
                .store
                .append(
                    &stream,
                    Version::new(prior),
                    PendingBatch::new(&batch).unwrap(),
                )
                .await
                .unwrap();
            if start.checked_add(100).unwrap() == self.total.checked_sub(1000).unwrap() {
                self.tail = Some(assigned);
            }
        }
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM mnesis.events")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        assert_eq!(u64::try_from(count).unwrap(), self.total);
        assert!(self.tail.is_some());
        for statement in [
            "VACUUM (ANALYZE) mnesis.events",
            "VACUUM (ANALYZE) mnesis.keys",
        ] {
            sqlx::query(statement).execute(&self.pool).await.unwrap();
        }
        let started = Instant::now();
        timeout(Duration::from_secs(60), async {
            loop {
                let eligible: bool = sqlx::query_scalar("SELECT pg_catalog.pg_snapshot_xmin(pg_catalog.pg_current_snapshot()) OPERATOR(pg_catalog.>) (SELECT txid FROM mnesis.events ORDER BY events.txid DESC, global_seq DESC LIMIT 1)")
                    .fetch_one(&self.pool).await.unwrap();
                if eligible { break; }
                yield_now().await;
            }
        }).await.expect("seed history must become eligible before measurement");
        writeln!(
            stderr().lock(),
            "capacity seed_visibility_wait={:?}",
            started.elapsed()
        )
        .unwrap();
    }

    async fn close(self) {
        drop(self.store);
        timeout(Duration::from_secs(30), self.pool.close())
            .await
            .unwrap();
        assert_eq!(self.pool.size(), 0);
        drop(self.cluster);
    }
}

async fn page(fixture: &Fixture, scan: Scan, edge: Edge) -> (Duration, u64) {
    let base = match (scan, edge) {
        (_, Edge::Start) => 0,
        (Scan::All, Edge::End) => fixture.total.checked_sub(1000).unwrap(),
        (Scan::Stream, Edge::End) if fixture.length == fixture.total => {
            fixture.total.checked_sub(1000).unwrap()
        }
        (Scan::Stream, Edge::End) => fixture.total.checked_sub(50).unwrap(),
    };
    let origin = key(base / fixture.length);
    let version = Version::new((base % fixture.length).checked_add(1).unwrap()).unwrap();
    let from = match edge {
        Edge::Start => None,
        Edge::End => fixture.tail,
    };
    let expected = PAGE.min(match scan {
        Scan::All => fixture.total.checked_sub(base).unwrap(),
        Scan::Stream => fixture.length.checked_sub(base % fixture.length).unwrap(),
    });
    let started = Instant::now();
    let mut cursor = match scan {
        Scan::Stream => fixture
            .store
            .read_stream(&origin, version)
            .await
            .unwrap()
            .map_ok(|event| (None, event))
            .boxed(),
        Scan::All => fixture
            .store
            .read_all(from)
            .await
            .unwrap()
            .map_ok(|(position, key, event)| (Some((position, key)), event))
            .boxed(),
    };
    let mut previous = from;
    for offset in 1..=expected {
        let (location, event) = cursor.try_next().await.unwrap().unwrap();
        let ordinal = base.checked_add(offset).unwrap();
        assert_eq!(event.payload().len(), fixture.payload.len());
        assert_eq!(&event.payload()[..8], ordinal.to_be_bytes());
        assert_eq!(&event.payload()[8..], &fixture.payload[8..]);
        assert_eq!(
            event.version().as_u64(),
            ordinal.checked_sub(1).unwrap() % fixture.length + 1
        );
        if let Some((position, actual_key)) = location {
            assert!(previous.is_none_or(|value| position > value));
            assert_eq!(
                actual_key,
                key(ordinal.checked_sub(1).unwrap() / fixture.length)
            );
            previous = Some(position);
        }
    }
    if expected < PAGE {
        assert!(cursor.try_next().await.unwrap().is_none());
    }
    drop(cursor);
    (started.elapsed(), expected)
}

fn rss() -> u64 {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse::<u64>()
        .unwrap()
        .checked_mul(1024)
        .unwrap()
}

struct Samples {
    durations: Vec<Duration>,
    rows: u64,
    wall: Duration,
    baseline: usize,
    allocated: usize,
    count: usize,
    rss: u64,
}

const fn percentile(samples: &[Duration], percent: usize) -> Duration {
    samples[samples
        .len()
        .checked_mul(percent)
        .unwrap()
        .div_ceil(100)
        .checked_sub(1)
        .unwrap()]
}

impl Samples {
    fn start() -> Self {
        let baseline = LIVE.load(Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        Self {
            durations: Vec::new(),
            rows: 0,
            wall: Duration::ZERO,
            baseline,
            allocated: TOTAL.load(Ordering::Relaxed),
            count: COUNT.load(Ordering::Relaxed),
            rss: rss(),
        }
    }

    fn report(mut self, label: &str, clients: u32) {
        self.durations.sort_unstable();
        assert!(self.durations.len() >= usize::try_from(SAMPLES).unwrap());
        let peak = PEAK.load(Ordering::Relaxed);
        let retained = LIVE.load(Ordering::Relaxed);
        let allocated = TOTAL
            .load(Ordering::Relaxed)
            .checked_sub(self.allocated)
            .unwrap();
        let calls = COUNT
            .load(Ordering::Relaxed)
            .checked_sub(self.count)
            .unwrap();
        let budget = usize::try_from(clients)
            .unwrap()
            .checked_mul(usize::try_from(BYTES).unwrap())
            .unwrap()
            .checked_mul(4)
            .unwrap()
            .checked_add(8_388_608)
            .unwrap();
        assert!(peak <= self.baseline.checked_add(budget).unwrap());
        assert!(retained <= self.baseline.checked_add(8_388_608).unwrap());
        let seconds = self.wall.as_secs_f64();
        let events = u32::try_from(self.rows).unwrap();
        let operations = u32::try_from(self.durations.len()).unwrap();
        writeln!(stderr().lock(), "capacity {label} clients={clients} samples={} wall={:?} p50={:?} p95={:?} p99={:?} events={} events_per_second={} operations_per_second={} allocated_bytes={allocated} allocation_calls={calls} baseline_bytes={} peak_bytes={peak} retained_bytes={retained} rss_before={} rss_after={}", self.durations.len(), self.wall, percentile(&self.durations, 50), percentile(&self.durations, 95), percentile(&self.durations, 99), self.rows, f64::from(events) / seconds, f64::from(operations) / seconds, self.baseline, self.rss, rss()).unwrap();
    }
}

async fn reads(fixture: &Arc<Fixture>, scan: Scan, edge: Edge, clients: u32) {
    for _ in 0..2 {
        cohort(fixture, scan, edge, clients).await;
    }
    for window in 0..WINDOWS {
        let mut samples = Samples::start();
        let started = Instant::now();
        for _ in 0..SAMPLES.div_ceil(clients) {
            for (duration, rows) in cohort(fixture, scan, edge, clients).await {
                samples.durations.push(duration);
                samples.rows = samples.rows.checked_add(rows).unwrap();
            }
        }
        samples.wall = started.elapsed();
        samples.report(&format!("read {scan:?}/{edge:?} window={window}"), clients);
        writeln!(
            stderr().lock(),
            "capacity pool_size={} idle={}",
            fixture.pool.size(),
            fixture.pool.num_idle()
        )
        .unwrap();
    }
}

async fn cohort(
    fixture: &Arc<Fixture>,
    scan: Scan,
    edge: Edge,
    clients: u32,
) -> Vec<(Duration, u64)> {
    let barrier = Arc::new(Barrier::new(usize::try_from(clients).unwrap()));
    let mut pending = JoinSet::new();
    for _ in 0..clients {
        let backing = Arc::clone(fixture);
        let ready = Arc::clone(&barrier);
        pending.spawn(async move {
            ready.wait().await;
            page(&backing, scan, edge).await
        });
    }
    let mut results = Vec::new();
    while let Some(result) = pending.join_next().await {
        results.push(result.unwrap());
    }
    results
}

async fn drain(fixture: &Fixture) {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let allocated = TOTAL.load(Ordering::Relaxed);
    let started = Instant::now();
    let mut cursor = fixture.store.read_all(None).await.unwrap();
    let mut count = 0_u64;
    let mut previous = None;
    while let Some((position, origin, event)) = cursor.try_next().await.unwrap() {
        count = count.checked_add(1).unwrap();
        assert!(previous.is_none_or(|value| position > value));
        assert_eq!(origin, key(count.checked_sub(1).unwrap() / fixture.length));
        assert_eq!(&event.payload()[..8], count.to_be_bytes());
        assert_eq!(&event.payload()[8..], &fixture.payload[8..]);
        previous = Some(position);
    }
    drop(cursor);
    assert_eq!(count, fixture.total);
    writeln!(stderr().lock(), "capacity full_global_drain events={count} elapsed={:?} baseline={baseline} peak={} retained={} allocated_bytes={} rss={}", started.elapsed(), PEAK.load(Ordering::Relaxed), LIVE.load(Ordering::Relaxed), TOTAL.load(Ordering::Relaxed).checked_sub(allocated).unwrap(), rss()).unwrap();
    assert!(PEAK.load(Ordering::Relaxed) <= baseline.checked_add(12_582_912).unwrap());
}

#[derive(Clone, Copy, Debug)]
enum Routing {
    Shared,
    Distinct,
}

enum Response {
    Committed(Duration),
    Conflict(Duration),
}

async fn writers(
    store: &PostgresStore,
    batch: &Arc<Vec<PendingEnvelope>>,
    clients: u32,
    prefix: &str,
    routing: Routing,
) -> Vec<Response> {
    let barrier = Arc::new(Barrier::new(usize::try_from(clients).unwrap()));
    let mut pending = JoinSet::new();
    for index in 0..clients {
        let backing = store.clone();
        let ready = Arc::clone(&barrier);
        let events = Arc::clone(batch);
        let identity = match routing {
            Routing::Shared => 0,
            Routing::Distinct => index,
        };
        let target = StreamKey::from_bytes(format!("writer-{prefix}-{identity}"));
        pending.spawn(async move {
            ready.wait().await;
            let begin = Instant::now();
            let result = backing
                .append(&target, None, PendingBatch::new(events.as_slice()).unwrap())
                .await;
            let response = match result {
                Ok(_) => Ok(Response::Committed(begin.elapsed())),
                Err(AppendError::Conflict { .. }) => Ok(Response::Conflict(begin.elapsed())),
                Err(error) => Err(error),
            };
            response.expect("unexpected write result, never retried")
        });
    }
    let mut results = Vec::new();
    let mut winners = 0_u32;
    while let Some(result) = pending.join_next().await {
        let response = result.unwrap();
        if matches!(response, Response::Committed(_)) {
            winners = winners.checked_add(1).unwrap();
        }
        results.push(response);
    }
    assert_eq!(
        winners,
        match routing {
            Routing::Shared => 1,
            Routing::Distinct => clients,
        }
    );
    results
}

async fn writes(fixture: &Arc<Fixture>, clients: u32, length: u64, routing: Routing) {
    let batch: Arc<Vec<_>> = Arc::new(
        (1..=length)
            .map(|offset| event(offset, offset, &fixture.payload))
            .collect(),
    );
    writers(
        &fixture.store,
        &batch,
        clients,
        &format!("warmup-{clients}-{length}-{routing:?}"),
        routing,
    )
    .await;
    for window in 0..WINDOWS {
        let before: i64 = sqlx::query_scalar("SELECT count(*) FROM mnesis.events")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
        let mut samples = Samples::start();
        let mut committed = Vec::new();
        let mut conflicts = 0_u32;
        let cohorts = match routing {
            Routing::Shared => SAMPLES,
            Routing::Distinct => SAMPLES.div_ceil(clients),
        };
        let started = Instant::now();
        for cohort in 0..cohorts {
            let prefix = format!("{clients}-{length}-{routing:?}-{window}-{cohort}");
            for response in writers(&fixture.store, &batch, clients, &prefix, routing).await {
                match response {
                    Response::Committed(duration) => {
                        samples.durations.push(duration);
                        committed.push(duration);
                        samples.rows = samples.rows.checked_add(length).unwrap();
                    }
                    Response::Conflict(duration) => {
                        samples.durations.push(duration);
                        conflicts = conflicts.checked_add(1).unwrap();
                    }
                }
            }
        }
        samples.wall = started.elapsed();
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM mnesis.events")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
        assert_eq!(
            after.checked_sub(before).unwrap(),
            i64::try_from(samples.rows).unwrap()
        );
        committed.sort_unstable();
        assert!(committed.len() >= usize::try_from(SAMPLES).unwrap());
        writeln!(stderr().lock(), "capacity committed_writes={} conflicts={conflicts} native_rows_before={before} native_rows_after={after} commit_p50={:?} commit_p95={:?} commit_p99={:?}", committed.len(), percentile(&committed, 50), percentile(&committed, 95), percentile(&committed, 99)).unwrap();
        samples.report(
            &format!("append batch={length} routing={routing:?} window={window}"),
            clients,
        );
    }
}

#[tokio::main]
async fn main() {
    let histories = std::env::var("MNESIS_CAPACITY_EVENTS").map_or_else(
        |_| vec![10_000, 100_000, 1_000_000],
        |value| vec![value.parse::<u64>().unwrap()],
    );
    for total in histories {
        assert!(total >= 10_000 && total % 100 == 0);
        for payload in [32, 4096] {
            for length in [100, total] {
                let mut prepared = Fixture::open(total, length, payload).await;
                prepared.seed().await;
                let fixture = Arc::new(prepared);
                drain(&fixture).await;
                for clients in [1, 16, 128] {
                    for scan in [Scan::Stream, Scan::All] {
                        for edge in [Edge::Start, Edge::End] {
                            reads(&fixture, scan, edge, clients).await;
                        }
                    }
                }
                plans(&fixture).await;
                for clients in [1, 16, 128] {
                    for batch in [1, 8] {
                        for routing in [Routing::Distinct, Routing::Shared] {
                            writes(&fixture, clients, batch, routing).await;
                        }
                    }
                }
                let before = rss();
                Arc::into_inner(fixture)
                    .expect("all measurement tasks joined")
                    .close()
                    .await;
                writeln!(
                    stderr().lock(),
                    "capacity closed rss_before={before} rss_after={} retained_allocator_bytes={}",
                    rss(),
                    LIVE.load(Ordering::Relaxed)
                )
                .unwrap();
            }
        }
    }
}

async fn parameters(
    fixture: &Fixture,
    statement: &str,
    edge: Edge,
    connection: &mut PgConnection,
) -> Vec<String> {
    let mut arguments = if statement.contains("WHERE stream_id = ") {
        let base = match edge {
            Edge::Start => 0,
            Edge::End if fixture.length == fixture.total => {
                fixture.total.checked_sub(1000).unwrap()
            }
            Edge::End => fixture.total.checked_sub(50).unwrap(),
        };
        let number: i64 = sqlx::query_scalar("SELECT number FROM mnesis.keys WHERE value = $1")
            .bind(key(base / fixture.length).as_bytes())
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        vec![
            number.to_string(),
            (base % fixture.length).checked_add(1).unwrap().to_string(),
            fixture.length.to_string(),
        ]
    } else {
        let cutoff: String = sqlx::query_scalar(
            "SELECT pg_catalog.pg_snapshot_xmin(pg_catalog.pg_current_snapshot())::text",
        )
        .fetch_one(&mut *connection)
        .await
        .unwrap();
        let mut values = vec![cutoff.parse::<u64>().unwrap().to_string()];
        if statement.contains("AND (txid, global_seq)") {
            let position = fixture.tail.unwrap();
            values.extend([position.txid().to_string(), position.seq().to_string()]);
        }
        values
    };
    arguments.extend([PAGE.to_string(), BYTES.to_string()]);
    arguments
        .into_iter()
        .map(|value| format!("'{value}'"))
        .collect()
}

async fn plans(fixture: &Fixture) {
    let mut connections = Vec::new();
    for _ in 1..POOL {
        connections.push(fixture.pool.acquire().await.unwrap());
    }
    let mut seen = BTreeSet::new();
    for connection in &mut connections {
        let statements: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT pg_catalog.quote_ident(name), statement, generic_plans, custom_plans FROM pg_prepared_statements WHERE statement LIKE 'WITH candidates AS%'"
        ).fetch_all(&mut **connection).await.unwrap();
        for (name, statement, generic, custom) in statements {
            if statement.contains("AND version > ") || !seen.insert(statement.clone()) {
                continue;
            }
            let edges = if statement.contains("WHERE stream_id = ") {
                vec![Edge::Start, Edge::End]
            } else if statement.contains("AND (txid, global_seq)") {
                vec![Edge::End]
            } else {
                vec![Edge::Start]
            };
            for (mode, setting) in [
                ("auto", "SET plan_cache_mode = auto"),
                ("generic", "SET plan_cache_mode = force_generic_plan"),
            ] {
                sqlx::query(setting)
                    .execute(&mut **connection)
                    .await
                    .unwrap();
                for &edge in &edges {
                    let arguments = parameters(fixture, &statement, edge, connection).await;
                    let mut explain =
                        QueryBuilder::<Postgres>::new("EXPLAIN (ANALYZE, BUFFERS) EXECUTE ");
                    explain
                        .push(&name)
                        .push("(")
                        .push(arguments.join(","))
                        .push(")");
                    let rows: Vec<String> = explain
                        .build_query_scalar()
                        .fetch_all(&mut **connection)
                        .await
                        .unwrap();
                    writeln!(
                    stderr().lock(),
                    "capacity plan mode={mode} prior_generic={generic} prior_custom={custom} edge={edge:?} query={statement} parameters={arguments:?}\n{}",
                    rows.join("\n")
                )
                .unwrap();
                }
            }
            sqlx::query("RESET plan_cache_mode")
                .execute(&mut **connection)
                .await
                .unwrap();
        }
    }
    assert_eq!(
        seen.len(),
        3,
        "initial/resumed all and initial stream shapes must be observed"
    );
}
