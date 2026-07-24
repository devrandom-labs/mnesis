//! Decision-gate benchmark for #333 (CLAUDE rule 9 — "measure the fork, don't
//! assert it"): which layout should the fjall `events_global` (`$all` index)
//! partition use once each row carries the writing stream's id?
//!
//! - **A2** — key = `[u64 BE global_seq][u16 BE id_len][id][u64 BE version]`,
//!   value = the frame bytes **unchanged** (the same `Slice` clone already
//!   shared with the `events` partition in production — an `Arc` bump, no
//!   extra allocation on append).
//! - **A1** — key = the existing 16-byte `[u64 BE global_seq][u64 BE version]`,
//!   value = `[u16 BE id_len][id][frame]` (a fresh `Vec` built on every append
//!   to wrap the id in front of the frame).
//!
//! Both layouts are measured raw against fjall partitions — no `mnesis-store`
//! or `mnesis-fjall` store code changes are needed for this comparison, since
//! the fork is purely about the `events_global` key/value shape. The
//! partition config mirrors production's `events_global` tuning
//! (`mnesis_fjall::partition::scan_defaults` — 32 KiB blocks, all-levels LZ4;
//! that function is crate-private, so it is mirrored here verbatim, the same
//! way `projection_storage.rs` mirrors the `projections` config).
//!
//! Two measurements, decided by CLAUDE rule 9 ("measure the fork, don't assert
//! it") — default is A2; A1 only wins if it beats A2 by >5% on **either**:
//! - insert wall-clock (via criterion; `sample_size(10)` — 20k inserts per
//!   iteration is expensive, same accommodation `append_sequential_throughput`
//!   / `lifecycle_benchmarks` make in `fjall_benchmarks.rs`)
//! - on-disk footprint (`disk_space()` after a forced memtable flush, printed
//!   once outside the criterion timing loop — not a criterion metric, same
//!   `measure`-then-`println!` pattern `projection_storage.rs` uses)
//!
//! Both layouts are fed **identical** event data (20 000 events, 120-byte
//! payloads, cycling through 100 synthetic 36-byte ids, uuid-string-sized) so
//! the comparison is apples-to-apples.
//!
//! Run:
//!
//! ```sh
//! nix develop -c cargo bench -p mnesis-fjall --bench all_index_layout
//! ```

#![allow(clippy::unwrap_used, reason = "bench code")]
#![allow(clippy::expect_used, reason = "bench code")]
#![allow(clippy::missing_panics_doc, reason = "bench code")]
#![allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "byte-count/ratio reporting casts u64 counters to f64; bench code, not a production path"
)]
#![allow(clippy::shadow_reuse, reason = "bench-local rebinding is fine")]
#![allow(
    clippy::print_stdout,
    reason = "reporting bench: emitting the decision-record numbers is the point"
)]
#![allow(clippy::doc_markdown, reason = "bench doc is prose, not API")]
#![allow(
    clippy::significant_drop_tightening,
    reason = "the `Criterion` temporary is held by the expansion of \
              `codspeed-criterion-compat`'s `criterion_group!`; its scope is \
              upstream's, not ours, and an item-level allow on the macro call \
              is discarded as an unused attribute"
)]
#![allow(
    clippy::items_after_statements,
    reason = "bench-local consts sit next to their use site"
)]

use std::time::Instant;

use bytes::Bytes;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use fjall::config::{BlockSizePolicy, CompressionPolicy};
use fjall::{
    CompressionType, KeyspaceCreateOptions, SingleWriterTxDatabase, SingleWriterTxKeyspace,
};
use mnesis_store::value::{EventType, Payload, SchemaVersion};
use mnesis_store::wire;
use tempfile::TempDir;

// ── fixed benchmark parameters (shared by both layouts) ─────────────────────

const EVENTS: u64 = 20_000;
const STREAMS: u64 = 100;
const PAYLOAD: usize = 120;
const ID_LEN: usize = 36;

// ── production `events_global` partition config, mirrored ──────────────────
//
// `mnesis_fjall::partition::scan_defaults` is crate-private, so its exact
// options are reproduced here (same approach `projection_storage.rs` takes
// for the `projections`/`snapshots` configs it measures).

/// Mirrors `mnesis_fjall::partition::scan_defaults` — the config
/// `FjallStoreBuilder` applies to both `events` and `events_global`.
fn events_global_config() -> KeyspaceCreateOptions {
    KeyspaceCreateOptions::default()
        .data_block_size_policy(BlockSizePolicy::all(32_768))
        .data_block_compression_policy(CompressionPolicy::all(CompressionType::Lz4))
}

// ── key/value builders under test ───────────────────────────────────────────

/// A2: id lives in the key; the value is left untouched (the frame bytes as
/// stored in `events`).
fn a2_key(gs: u64, id: &[u8], ver: u64) -> Vec<u8> {
    let mut k = Vec::with_capacity(8 + 2 + id.len() + 8);
    k.extend_from_slice(&gs.to_be_bytes());
    k.extend_from_slice(
        &u16::try_from(id.len())
            .expect("bench id fits u16")
            .to_be_bytes(),
    );
    k.extend_from_slice(id);
    k.extend_from_slice(&ver.to_be_bytes());
    k
}

/// A1: the existing 16-byte `[gs BE][ver BE]` key — no id.
fn a1_key(gs: u64, ver: u64) -> Vec<u8> {
    let mut k = Vec::with_capacity(16);
    k.extend_from_slice(&gs.to_be_bytes());
    k.extend_from_slice(&ver.to_be_bytes());
    k
}

/// A1: id lives in the value, wrapped in front of the frame bytes — a fresh
/// buffer built on every append (unlike A2's shared-`Slice` clone).
fn a1_value(id: &[u8], frame: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(2 + id.len() + frame.len());
    v.extend_from_slice(
        &u16::try_from(id.len())
            .expect("bench id fits u16")
            .to_be_bytes(),
    );
    v.extend_from_slice(id);
    v.extend_from_slice(frame);
    v
}

// ── deterministic event data, shared by both layouts ────────────────────────

/// `STREAMS` synthetic uuid-string-sized (36-byte) ids, e.g.
/// `"00000005-0000-4000-8000-000000000005"`.
fn synthetic_ids() -> Vec<[u8; ID_LEN]> {
    (0..STREAMS)
        .map(|i| {
            let s = format!("{i:08x}-0000-4000-8000-{i:012x}");
            let bytes = s.as_bytes();
            assert_eq!(
                bytes.len(),
                ID_LEN,
                "synthetic id must be uuid-string-sized"
            );
            let mut buf = [0u8; ID_LEN];
            buf.copy_from_slice(bytes);
            buf
        })
        .collect()
}

/// The frame bytes every event shares — built once via the real production
/// encoder (`mnesis_store::wire::encode_frame`), mirroring
/// `fjall_benchmarks.rs::build_frame_value`.
fn build_frame_value() -> Bytes {
    let sv = SchemaVersion::from_u32(1).unwrap();
    let et = EventType::from_bytes(Bytes::from_static(b"BenchEvent")).unwrap();
    let pl = Payload::from_bytes(Bytes::from(vec![0xAB_u8; PAYLOAD])).unwrap();
    wire::encode_frame(sv, &et, &pl, None).unwrap().value
}

/// Per-stream version counters, one entry per synthetic id, so each stream's
/// events carry a strictly increasing version — the same invariant `append`
/// enforces in production.
struct VersionCounters(Vec<u64>);

impl VersionCounters {
    fn new(streams: usize) -> Self {
        Self(vec![0; streams])
    }

    /// Advance and return the next version for stream `idx`.
    fn next(&mut self, idx: usize) -> u64 {
        self.0[idx] += 1;
        self.0[idx]
    }
}

/// Insert `EVENTS` rows under the A2 layout, cycling through `ids` and
/// assigning each stream sequential versions. Returns the logical (pre-storage)
/// byte total.
fn insert_a2(ks: &SingleWriterTxKeyspace, ids: &[[u8; ID_LEN]], frame: &Bytes) -> u64 {
    let mut versions = VersionCounters::new(ids.len());
    let mut logical = 0u64;
    for i in 0..EVENTS {
        let idx = usize::try_from(i % STREAMS).expect("stream index fits usize");
        let id = &ids[idx];
        let version = versions.next(idx);
        let key = a2_key(i + 1, id, version);
        logical += (key.len() + frame.len()) as u64;
        ks.insert(key, frame.clone()).expect("insert a2 row");
    }
    logical
}

/// Insert `EVENTS` rows under the A1 layout, same cycling/versioning as
/// [`insert_a2`]. Returns the logical (pre-storage) byte total.
fn insert_a1(ks: &SingleWriterTxKeyspace, ids: &[[u8; ID_LEN]], frame: &Bytes) -> u64 {
    let mut versions = VersionCounters::new(ids.len());
    let mut logical = 0u64;
    for i in 0..EVENTS {
        let idx = usize::try_from(i % STREAMS).expect("stream index fits usize");
        let id = &ids[idx];
        let version = versions.next(idx);
        let key = a1_key(i + 1, version);
        let value = a1_value(id, frame);
        logical += (key.len() + value.len()) as u64;
        ks.insert(key, value).expect("insert a1 row");
    }
    logical
}

// ── on-disk footprint (the rule-9 decision metric) ──────────────────────────

struct Sample {
    disk: u64,
    logical: u64,
    write_ms: u128,
}

fn measure_disk(
    ids: &[[u8; ID_LEN]],
    frame: &Bytes,
    insert: impl FnOnce(&SingleWriterTxKeyspace, &[[u8; ID_LEN]], &Bytes) -> u64,
) -> Sample {
    let dir = TempDir::new().expect("tempdir");
    let db = SingleWriterTxDatabase::builder(dir.path().join("db"))
        .open()
        .expect("open db");
    let ks = db
        .keyspace("events_global", events_global_config)
        .expect("keyspace");

    let start = Instant::now();
    let logical = insert(&ks, ids, frame);
    let write_ms = start.elapsed().as_millis();

    // Force the active memtable to SSTable so the footprint reflects the
    // on-disk, post-compression cost (same discipline as
    // `projection_storage.rs::measure`).
    ks.inner().rotate_memtable_and_wait().expect("flush");

    Sample {
        disk: ks.inner().disk_space(),
        logical,
        write_ms,
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn report_disk_usage() {
    let ids = synthetic_ids();
    let frame = build_frame_value();

    let a2 = measure_disk(&ids, &frame, insert_a2);
    let a1 = measure_disk(&ids, &frame, insert_a1);

    println!(
        "\n# $all-index layout decision (#333, rule 9) — {EVENTS} events, {PAYLOAD} B payload, \
         {STREAMS} streams, {ID_LEN} B ids\n"
    );
    println!("Disk = fjall `disk_space()` (SSTable bytes) after flush. `×` = disk / logical.\n");
    println!("| layout | logical | disk | ×raw | insert (one-shot) |");
    println!("|:-------|--------:|-----:|-----:|-------------------:|");
    println!(
        "| A2 (id in key, shared value) | {} | {} | {:.3} | {} ms |",
        mib(a2.logical),
        mib(a2.disk),
        a2.disk as f64 / a2.logical as f64,
        a2.write_ms,
    );
    println!(
        "| A1 (id in value wrap) | {} | {} | {:.3} | {} ms |",
        mib(a1.logical),
        mib(a1.disk),
        a1.disk as f64 / a1.logical as f64,
        a1.write_ms,
    );
    let disk_delta = (a1.disk as f64 / a2.disk as f64 - 1.0) * 100.0;
    println!(
        "\nOn-disk delta: A1 is {disk_delta:+.1}% vs A2. Decision rule: default A2; pick A1 only \
         if A2 is worse by >5% on disk size or append time."
    );
}

// ── criterion: insert wall-clock ─────────────────────────────────────────────

fn insert_benchmarks(c: &mut Criterion) {
    report_disk_usage();

    let ids = synthetic_ids();
    let frame = build_frame_value();

    let mut group = c.benchmark_group("all_index_layout_insert");
    group.sample_size(10);
    group.throughput(Throughput::Elements(EVENTS));

    group.bench_function("a2_id_in_key", |b| {
        b.iter_with_setup(
            || {
                let dir = TempDir::new().expect("tempdir");
                let db = SingleWriterTxDatabase::builder(dir.path().join("db"))
                    .open()
                    .expect("open db");
                let ks = db
                    .keyspace("events_global", events_global_config)
                    .expect("keyspace");
                (dir, ks)
            },
            |(dir, ks)| {
                insert_a2(&ks, &ids, &frame);
                // Return the handles so criterion drops them OUTSIDE the timed
                // window — database shutdown + TempDir deletion are teardown,
                // not insert cost.
                (dir, ks)
            },
        );
    });

    group.bench_function("a1_id_in_value", |b| {
        b.iter_with_setup(
            || {
                let dir = TempDir::new().expect("tempdir");
                let db = SingleWriterTxDatabase::builder(dir.path().join("db"))
                    .open()
                    .expect("open db");
                let ks = db
                    .keyspace("events_global", events_global_config)
                    .expect("keyspace");
                (dir, ks)
            },
            |(dir, ks)| {
                insert_a1(&ks, &ids, &frame);
                // Same untimed-teardown discipline as the A2 routine above.
                (dir, ks)
            },
        );
    });

    group.finish();
}

criterion_group!(benches, insert_benchmarks);
criterion_main!(benches);
