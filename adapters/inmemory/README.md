# mnesis-inmemory

In-memory event-store adapter for [Mnesis](https://github.com/devrandom-labs/mnesis) — a full `RawEventStore + WakeSource + SnapshotStore` (plus `StreamLister`/`ConsistentExporter`/`AtomicAppend` behind `export`/`import`) backed by ordinary maps. Ideal for tests, examples, and prototyping where persistence isn't needed.

It is also `mnesis-store`'s own test fixture (via a path-only dev-dependency), so it tracks the store contract exactly.

## Quickstart

```toml
[dependencies]
mnesis-inmemory = "0.3.1"
```

```rust
let store = InMemoryStore::default().into_store();
```

## Features

| Feature | Adds |
|---------|------|
| `export` / `import` | Backup/restore (`StreamLister`, `ConsistentExporter`, `AtomicAppend`) |

## MSRV & stability

MSRV **1.99**. Ships in the **0.x tier**. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).

## Consistent export

With `export` enabled, `ConsistentExporter::open_export_session(lifetime)`
captures every stream id and head under the same lock used for atomic commits.
Use that session's `list_streams` and `export_stream` for a backup while writers
remain active. Ordinary `EventExporter` reads still open independently.

The session stores one key/head entry per stream. Reads clone at most the
configured batch size of existing frame handles; they do not copy payloads or
materialize an entire history. The adapter's immutable, append-only histories
allow those captured heads to define the fixed view without additional history
copies or storage pins. The source remains borrowed until the session is released.

Supply a nonzero finite lifetime. A request or cursor poll after its deadline
returns `ExportError::Expired`; treat the backup as incomplete and restart with
a new session. Exhausted cursors remain exhausted. Release cursors before
`ExportSession::close`; the compiler rejects closing a session that has a live
cursor. Already-yielded envelopes remain owned and usable after close.

## Read paging

Ordinary stream reads use binary search to skip versions before the inclusive
start and clone at most one configured page of frame handles under the commit
lock. Each refill costs O(log N + B) for N stored rows and page size B. Reads
can observe appends before a later refill; once exhausted, they stay exhausted.
Use a consistent export session when every page must belong to a fixed view.

The retained `paged_reads_bound_prefix_search_work` test counts the actual
production search comparisons for 256/512/1024 rows with eight-row pages. Run
`nix develop -c cargo test -p mnesis-inmemory --all-features --locked`. The
production adapter benchmark separates seeding from timed reads at
1024/2048/4096 rows: `nix develop -c cargo bench -p mnesis-inmemory --bench paged_reads --locked`.
