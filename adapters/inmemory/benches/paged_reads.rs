//! Production in-memory paging at N, 2N and 4N with a fixed eight-row page.
#![allow(
    clippy::unwrap_used,
    reason = "benchmark fixtures have known valid bounds"
)]
#![allow(
    clippy::significant_drop_tightening,
    reason = "criterion_group expands an upstream temporary held across registration"
)]

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use futures::StreamExt;
use mnesis::Version;
use mnesis_inmemory::InMemoryStore;
use mnesis_store::batch::BatchSize;
use mnesis_store::store::RawEventStore;
use mnesis_store::{PendingBatch, StreamKey, pending_envelope};

fn paged_reads(criterion: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut group = criterion.benchmark_group("inmemory_paged_reads_batch_8");
    for count in [1024u64, 2048, 4096] {
        let store = InMemoryStore::with_batch_size(BatchSize::new(8).unwrap());
        let id = StreamKey::from_slice(b"benchmark");
        let events: Vec<_> = (1..=count)
            .map(|version| {
                pending_envelope(Version::new(version).unwrap())
                    .event_type("Event")
                    .payload(b"payload".as_slice())
                    .build()
                    .unwrap()
            })
            .collect();
        runtime.block_on(async {
            store
                .append(&id, None, PendingBatch::new(&events).unwrap())
                .await
                .unwrap();
        });
        group.throughput(Throughput::Elements(count));
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |bench, _| {
            bench.iter(|| {
                runtime.block_on(async {
                    let mut stream = store.read_stream(&id, Version::INITIAL).await.unwrap();
                    while let Some(row) = stream.next().await {
                        black_box(row.unwrap());
                    }
                });
            });
        });
    }
    group.finish();
}

criterion_group!(benches, paged_reads);
criterion_main!(benches);
