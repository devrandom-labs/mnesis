//! `mnesis-store` benchmarks.
//!
//! Measures the hot paths in the store layer:
//! - `PendingEnvelope` builder throughput
//! - `PersistedEnvelope` construction (zero-alloc)
//! - production `InMemoryStore` append throughput at various batch sizes
//! - production `InMemoryStore` `read_stream` throughput at various sizes
//! - upcaster chain throughput
//!
//! Run: `cargo bench --bench store_bench -p mnesis-store`
//! Reports: `target/criterion/report/index.html`
//! Prior runs used a benchmark-only adapter and are not measurements of this
//! production implementation; rerun before comparing timing results.
#![allow(clippy::unwrap_used, reason = "benchmarks use unwrap for brevity")]
#![allow(clippy::expect_used, reason = "benchmarks use expect for brevity")]
#![allow(
    clippy::as_conversions,
    reason = "benchmarks use as casts for index conversions"
)]
#![allow(
    clippy::str_to_string,
    reason = "benchmarks use to_string for convenience"
)]
#![allow(clippy::print_stdout, reason = "criterion may print to stdout")]
#![allow(clippy::print_stderr, reason = "criterion may print to stderr")]
#![allow(
    clippy::significant_drop_tightening,
    reason = "the `Criterion` temporary is held by the expansion of \
              `codspeed-criterion-compat`'s `criterion_group!`; its scope is \
              upstream's, not ours, and an item-level allow on the macro call \
              is discarded as an unused attribute"
)]
#![allow(
    clippy::missing_panics_doc,
    reason = "benchmark functions do not need panic docs"
)]
#![allow(
    clippy::unnecessary_wraps,
    reason = "plain-function upcasters keep Result<_, E> so they can be passed to load_with"
)]

use std::convert::Infallible;
use std::hint::black_box;

use bytes::Bytes;
use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use futures::StreamExt;
use mnesis::Version;
use mnesis_inmemory::InMemoryStore;
use mnesis_store::PendingBatch;
use mnesis_store::StreamKey;
use mnesis_store::envelope::{PendingEnvelope, PersistedEnvelope};
use mnesis_store::pending_envelope;
use mnesis_store::store::RawEventStore;

fn build_persisted(version: u64, event_type: &str, payload: &[u8]) -> PersistedEnvelope {
    let mut buf = Vec::with_capacity(event_type.len() + payload.len());
    buf.extend_from_slice(event_type.as_bytes());
    buf.extend_from_slice(payload);
    let value = Bytes::from(buf);
    let et_end = u32::try_from(event_type.len()).expect("fits u32");
    let pl_end = u32::try_from(event_type.len() + payload.len()).expect("fits u32");
    PersistedEnvelope::try_new(
        mnesis::Version::new(version).expect("non-zero version"),
        value,
        mnesis_store::value::SchemaVersion::INITIAL,
        0..et_end,
        et_end..pl_end,
        None,
    )
    .expect("valid fixture")
}

// =============================================================================
// Noop upcaster for benchmarks — plain-function form, walks v1 through v6
// =============================================================================

fn noop_v1_to_v6_upcast(
    mut morsel: mnesis_store::upcasting::EventMorsel<'_>,
) -> Result<mnesis_store::upcasting::EventMorsel<'_>, Infallible> {
    loop {
        morsel = match (morsel.event_type(), morsel.schema_version()) {
            ("UserCreated", v) if v == mnesis_store::SchemaVersion::from_u32(1).unwrap() => {
                mnesis_store::upcasting::EventMorsel::new(
                    "UserCreated",
                    mnesis_store::SchemaVersion::from_u32(2).unwrap(),
                    morsel.payload().to_vec(),
                )
            }
            ("UserCreated", v) if v == mnesis_store::SchemaVersion::from_u32(2).unwrap() => {
                mnesis_store::upcasting::EventMorsel::new(
                    "UserCreated",
                    mnesis_store::SchemaVersion::from_u32(3).unwrap(),
                    morsel.payload().to_vec(),
                )
            }
            ("UserCreated", v) if v == mnesis_store::SchemaVersion::from_u32(3).unwrap() => {
                mnesis_store::upcasting::EventMorsel::new(
                    "UserCreated",
                    mnesis_store::SchemaVersion::from_u32(4).unwrap(),
                    morsel.payload().to_vec(),
                )
            }
            ("UserCreated", v) if v == mnesis_store::SchemaVersion::from_u32(4).unwrap() => {
                mnesis_store::upcasting::EventMorsel::new(
                    "UserCreated",
                    mnesis_store::SchemaVersion::from_u32(5).unwrap(),
                    morsel.payload().to_vec(),
                )
            }
            ("UserCreated", v) if v == mnesis_store::SchemaVersion::from_u32(5).unwrap() => {
                mnesis_store::upcasting::EventMorsel::new(
                    "UserCreated",
                    mnesis_store::SchemaVersion::from_u32(6).unwrap(),
                    morsel.payload().to_vec(),
                )
            }
            _ => break,
        };
    }
    Ok(morsel)
}

// =============================================================================
// Helpers
// =============================================================================

fn make_envelopes(n: usize) -> Vec<PendingEnvelope> {
    (0..n)
        .map(|i| {
            let version = u64::try_from(i + 1).unwrap_or(u64::MAX);
            pending_envelope(Version::new(version).unwrap())
                .event_type("BenchEvent")
                .payload(vec![1, 2, 3, 4])
                .build()
                .expect("valid envelope")
        })
        .collect()
}

// =============================================================================
// Benchmarks
// =============================================================================

fn bench_builder_throughput(c: &mut Criterion) {
    c.bench_function("PendingEnvelope builder", |b| {
        b.iter(|| {
            black_box(
                pending_envelope(black_box(Version::INITIAL))
                    .event_type("UserCreated")
                    .payload(vec![1, 2, 3, 4])
                    .build()
                    .expect("valid envelope"),
            )
        });
    });
}

fn bench_persisted_envelope_construction(c: &mut Criterion) {
    let event_type = "UserCreated";
    let payload = [1u8, 2, 3, 4, 5, 6, 7, 8];
    c.bench_function("PersistedEnvelope::try_new", |b| {
        b.iter(|| {
            black_box(build_persisted(
                1,
                black_box(event_type),
                black_box(&payload),
            ))
        });
    });
}

fn bench_append(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("append");

    for size in [1, 10, 100, 1000] {
        let envelopes = make_envelopes(size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &envelopes, |b, envs| {
            b.iter_batched_ref(
                InMemoryStore::new,
                |store| {
                    rt.block_on(async {
                        store
                            .append(
                                &StreamKey::from_slice(b"bench-stream"),
                                None,
                                PendingBatch::new(black_box(envs))
                                    .expect("bench batch is non-empty"),
                            )
                            .await
                            .unwrap();
                    });
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_read_stream(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("read_stream");

    for size in [10, 100, 1000] {
        let envelopes = make_envelopes(size);

        // Pre-populate the store once for this size
        let store = InMemoryStore::new();
        rt.block_on(async {
            store
                .append(
                    &StreamKey::from_slice(b"bench-stream"),
                    None,
                    PendingBatch::new(&envelopes).expect("non-empty batch"),
                )
                .await
                .unwrap();
        });

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    let mut stream = store
                        .read_stream(&StreamKey::from_slice(b"bench-stream"), Version::INITIAL)
                        .await
                        .unwrap();
                    while let Some(__item) = stream.next().await {
                        let env = __item.unwrap();
                        black_box(env);
                    }
                });
            });
        });
    }
    group.finish();
}

fn bench_upcaster(c: &mut Criterion) {
    let payload = vec![1u8, 2, 3, 4, 5, 6, 7, 8];

    c.bench_function("upcaster (5 noop steps)", |b| {
        b.iter(|| {
            let morsel = mnesis_store::EventMorsel::borrowed(
                "UserCreated",
                mnesis_store::SchemaVersion::from_u32(1).unwrap(),
                black_box(&payload),
            );
            let result = noop_v1_to_v6_upcast(morsel).unwrap();
            black_box(result)
        });
    });
}

criterion_group!(
    benches,
    bench_builder_throughput,
    bench_persisted_envelope_construction,
    bench_append,
    bench_read_stream,
    bench_upcaster
);
criterion_main!(benches);
