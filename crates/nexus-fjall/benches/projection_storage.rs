//! Reproducible storage-cost benchmark for the projection-state partition
//! (issues #164 follow-up; CLAUDE rule 9 — "measure the fork, don't assert it").
//!
//! Run:
//!
//! ```sh
//! nix develop -c cargo bench -p nexus-fjall --bench projection_storage
//! ```
//!
//! It is deterministic (seeded xorshift for "random" payloads, fixed sizes and
//! counts), so the on-disk numbers reproduce run to run; only the wall-clock
//! timings drift and are reported as indicative, not load-bearing. The decision
//! metric is `disk_space()` — fjall's SSTable footprint — which is what a
//! flash-constrained `IoT`/mobile device actually pays.
//!
//! ## Fork 1 — projection-partition config
//!
//! `nexus_fjall`'s `projections` partition uses `point_read_defaults`: 4 KiB
//! data blocks + a 15-bit bloom filter, and — because it sets no compression
//! policy — fjall's default `[None, None, Lz4]` (L0/L1 uncompressed, L2+ LZ4).
//! It is tuned for small point-read metadata, but projection *state* can be
//! large. This measures three configs across payload sizes × compressibility:
//!
//! - `current`   — 4 KiB blocks, default `[None,None,Lz4]`, bloom (today's config)
//! - `scan_lz4`  — 32 KiB blocks, all-levels LZ4 (the `events` scan config)
//! - `pr_lz4`    — 4 KiB blocks, all-levels LZ4, bloom (point-read + full LZ4)
//!
//! ## Fork 2 — separate partition vs key-prefix
//!
//! `snapshots` and `projections` are separate fjall partitions (separate LSM
//! trees). The alternative is one partition with `s:`/`p:` key prefixes. This
//! measures the on-disk delta and reports the fixed per-partition cost (each
//! partition carries its own active memtable up to `max_memtable_size`).

#![allow(clippy::unwrap_used, reason = "bench code")]
#![allow(clippy::expect_used, reason = "bench code")]
#![allow(clippy::missing_panics_doc, reason = "bench code")]
#![allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "ratio reporting divides u64 byte counts as f64"
)]
#![allow(clippy::shadow_reuse, reason = "bench-local rebinding is fine")]
#![allow(
    clippy::print_stdout,
    reason = "reporting bench: emitting the result tables is the point"
)]
#![allow(clippy::doc_markdown, reason = "bench doc is prose, not API")]
#![allow(
    clippy::type_complexity,
    reason = "the config table is a small slice of (name, fn) pairs"
)]
#![allow(
    clippy::items_after_statements,
    reason = "bench-local consts sit next to their use site"
)]

use std::time::Instant;

use fjall::config::{
    BlockSizePolicy, BloomConstructionPolicy, CompressionPolicy, FilterPolicy, FilterPolicyEntry,
};
use fjall::{CompressionType, KeyspaceCreateOptions, SingleWriterTxDatabase};
use tempfile::TempDir;

/// Small memtable so writing a few MB rotates several times and the bulk of the
/// data lands in (compressed) SSTables rather than the active memtable — so
/// `disk_space()` reflects the on-disk, post-compression cost.
const MEMTABLE_BYTES: u64 = 256 * 1024;

/// The 12-byte snapshot/projection value header
/// (`[u32 LE schema_version][u64 BE position]`), mirrored so measured values are
/// the same shape the real `write_projection` stores.
const VALUE_HEADER: usize = 12;

// ── partition configs (mirror `nexus_fjall::partition`) ─────────────────────

fn bloom() -> FilterPolicy {
    FilterPolicy::all(FilterPolicyEntry::Bloom(
        BloomConstructionPolicy::BitsPerKey(15.0),
    ))
}

/// Today's `projections` config: `point_read_defaults` (no explicit compression
/// → fjall default `[None, None, Lz4]`).
fn current_config() -> KeyspaceCreateOptions {
    KeyspaceCreateOptions::default()
        .max_memtable_size(MEMTABLE_BYTES)
        .data_block_size_policy(BlockSizePolicy::all(4_096))
        .filter_policy(bloom())
        .expect_point_read_hits(true)
}

/// The `events` scan config: 32 KiB blocks, all-levels LZ4, no bloom.
fn scan_lz4_config() -> KeyspaceCreateOptions {
    KeyspaceCreateOptions::default()
        .max_memtable_size(MEMTABLE_BYTES)
        .data_block_size_policy(BlockSizePolicy::all(32_768))
        .data_block_compression_policy(CompressionPolicy::all(CompressionType::Lz4))
}

/// Hybrid: point-read 4 KiB blocks + bloom, but LZ4 on every level.
fn pr_lz4_config() -> KeyspaceCreateOptions {
    KeyspaceCreateOptions::default()
        .max_memtable_size(MEMTABLE_BYTES)
        .data_block_size_policy(BlockSizePolicy::all(4_096))
        .data_block_compression_policy(CompressionPolicy::all(CompressionType::Lz4))
        .filter_policy(bloom())
        .expect_point_read_hits(true)
}

// ── deterministic payloads ──────────────────────────────────────────────────

/// Structured, highly-compressible payload — a repeated record, standing in for
/// real projection state (JSON/struct with repeated keys).
fn structured(size: usize) -> Vec<u8> {
    let unit = br#"{"id":1234567,"count":42,"total":1000000,"status":"active"},"#;
    let mut v = Vec::with_capacity(size + unit.len());
    while v.len() < size {
        v.extend_from_slice(unit);
    }
    v.truncate(size);
    v
}

/// Incompressible payload — a seeded xorshift stream (deterministic).
fn random(size: usize, state: &mut u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(size + 8);
    while v.len() < size {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.truncate(size);
    v
}

fn value(payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(VALUE_HEADER + payload.len());
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(&1u64.to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

// ── measurement ─────────────────────────────────────────────────────────────

struct SizeCase {
    label: &'static str,
    bytes: usize,
    count: usize,
}

/// Payload sizes × blob counts, chosen so each config writes a few–tens of MB
/// (enough to trigger memtable rotations) while the bench stays quick.
const SIZE_CASES: &[SizeCase] = &[
    SizeCase {
        label: "128 B",
        bytes: 128,
        count: 8_000,
    },
    SizeCase {
        label: "1 KiB",
        bytes: 1_024,
        count: 4_000,
    },
    SizeCase {
        label: "16 KiB",
        bytes: 16_384,
        count: 512,
    },
    SizeCase {
        label: "256 KiB",
        bytes: 262_144,
        count: 96,
    },
];

struct Sample {
    disk: u64,
    logical: u64,
    write_ms: u128,
}

/// Write `count` blobs of `bytes` each under `config`, flush to SSTables, and
/// return the on-disk footprint, the logical (pre-storage) byte total, and the
/// write wall-clock.
fn measure(config: KeyspaceCreateOptions, case: &SizeCase, compressible: bool) -> Sample {
    let dir = TempDir::new().expect("tempdir");
    let db = SingleWriterTxDatabase::builder(dir.path().join("db"))
        .open()
        .expect("open db");
    let ks = db.keyspace("bench", || config).expect("keyspace");

    let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
    let mut logical = 0u64;
    let start = Instant::now();
    for i in 0..case.count {
        let payload = if compressible {
            structured(case.bytes)
        } else {
            random(case.bytes, &mut seed)
        };
        let v = value(&payload);
        logical += v.len() as u64;
        ks.insert(format!("proj-{i:08}").as_bytes(), v)
            .expect("insert");
    }
    let write_ms = start.elapsed().as_millis();

    // Force the active memtable to SSTable so compression is realised on disk.
    ks.inner().rotate_memtable_and_wait().expect("flush");

    Sample {
        disk: ks.inner().disk_space(),
        logical,
        write_ms,
    }
}

fn fork1_partition_config() {
    println!("\n## Fork 1 — projection-partition config\n");
    println!("Disk = fjall `disk_space()` (SSTable bytes) after flush. `×` = disk / logical.\n");
    println!("| payload | data | config | logical | disk | ×raw | write |");
    println!("|--------:|:-----|:-------|--------:|-----:|-----:|------:|");

    let configs: &[(&str, fn() -> KeyspaceCreateOptions)] = &[
        ("current", current_config),
        ("scan_lz4", scan_lz4_config),
        ("pr_lz4", pr_lz4_config),
    ];

    for case in SIZE_CASES {
        for (compressible, kind) in [(true, "structured"), (false, "random")] {
            for (name, cfg) in configs {
                let s = measure(cfg(), case, compressible);
                println!(
                    "| {} | {} | {} | {} | {} | {:.2} | {} ms |",
                    case.label,
                    kind,
                    name,
                    mib(s.logical),
                    mib(s.disk),
                    s.disk as f64 / s.logical as f64,
                    s.write_ms,
                );
            }
        }
    }
}

fn fork2_partition_vs_prefix() {
    println!("\n## Fork 2 — separate partitions vs one key-prefixed partition\n");

    // Representative mixed workload: small aggregate snapshots + larger
    // projection state, equal counts.
    const SNAP_BYTES: usize = 256;
    const PROJ_BYTES: usize = 4_096;
    const COUNT: usize = 3_000;

    // (a) two separate partitions.
    let sep_dir = TempDir::new().unwrap();
    let sep_db = SingleWriterTxDatabase::builder(sep_dir.path().join("db"))
        .open()
        .unwrap();
    let snaps = sep_db.keyspace("snapshots", current_config).unwrap();
    let projs = sep_db.keyspace("projections", current_config).unwrap();
    for i in 0..COUNT {
        snaps
            .insert(
                format!("a-{i:08}").as_bytes(),
                value(&structured(SNAP_BYTES)),
            )
            .unwrap();
        projs
            .insert(
                format!("p-{i:08}").as_bytes(),
                value(&structured(PROJ_BYTES)),
            )
            .unwrap();
    }
    snaps.inner().rotate_memtable_and_wait().unwrap();
    projs.inner().rotate_memtable_and_wait().unwrap();
    let sep_disk = snaps.inner().disk_space() + projs.inner().disk_space();

    // (b) one partition, `s:` / `p:` key prefixes.
    let pre_dir = TempDir::new().unwrap();
    let pre_db = SingleWriterTxDatabase::builder(pre_dir.path().join("db"))
        .open()
        .unwrap();
    let one = pre_db.keyspace("state", current_config).unwrap();
    for i in 0..COUNT {
        one.insert(
            format!("s:a-{i:08}").as_bytes(),
            value(&structured(SNAP_BYTES)),
        )
        .unwrap();
        one.insert(
            format!("p:p-{i:08}").as_bytes(),
            value(&structured(PROJ_BYTES)),
        )
        .unwrap();
    }
    one.inner().rotate_memtable_and_wait().unwrap();
    let pre_disk = one.inner().disk_space();

    println!(
        "Workload: {COUNT} × {SNAP_BYTES} B snapshots + {COUNT} × {PROJ_BYTES} B projections.\n"
    );
    println!("| layout | partitions (memtables) | disk |");
    println!("|:-------|:----------------------:|-----:|");
    println!("| separate | 2 | {} |", mib(sep_disk));
    println!("| prefixed | 1 | {} |", mib(pre_disk));
    println!(
        "\nOn-disk delta: {:+.1}% (prefixed vs separate). Each separate partition also \
         holds its own active memtable (up to {} KiB resident), so N partitions = N write \
         buffers — the fixed RAM cost that dominates on a constrained device.",
        (pre_disk as f64 / sep_disk as f64 - 1.0) * 100.0,
        MEMTABLE_BYTES / 1024,
    );
}

fn mib(bytes: u64) -> String {
    format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn main() {
    println!("# Projection storage benchmark (fjall {})", fjall_version());
    println!(
        "\nDeterministic; memtable cap {} KiB. Decision metric = on-disk `disk_space()`.",
        MEMTABLE_BYTES / 1024
    );
    fork1_partition_config();
    fork2_partition_vs_prefix();
}

const fn fjall_version() -> &'static str {
    "3.1.x"
}
