# mnesis-store

The persistence edge layer for [Mnesis](https://github.com/devrandom-labs/mnesis) — codecs, envelopes, the append/read seams adapters implement (`RawEventStore` + `WakeSource`), schema upcasting, snapshots, projections, subscriptions, and backup export/import. Kernel-pure in, bytes-on-disk out.

`no_std + alloc` capable (disable default features); the subscription loop is generic over `WakeSource`, so it pulls in no runtime.

## Quickstart

```toml
[dependencies]
mnesis-store = "0.3.1"
```

Pick a storage adapter to back it: [`mnesis-fjall`](../../adapters/fjall) (embedded LSM), [`mnesis-postgres`](../../adapters/postgres), or [`mnesis-inmemory`](../../adapters/inmemory). See the [root README](../../README.md) for a full lifecycle example.

## Storage contracts

`read_stream` starts at an inclusive stream version; `read_all` resumes strictly
after its exclusive global position and supplies the raw origin key separately
from the envelope. Aggregate snapshots are unconditional caches, written
separately from event append. Projection checkpoints use `CheckpointStore`
revision CAS for the matching schema/position/state record; a projection owns
that pair, and failed or canceled writes require reload. See the trait docs for
exact failure, replay and rebuild behavior.

## Features

| Feature | Adds |
|---------|------|
| `std` *(default)* | `std::error::Error` bridge; disable for `no_std + alloc + core::error::Error` |
| `serde` / `json` | Serde codec; JSON alias |
| `bytemuck` / `rkyv` | Zero-copy POD / archived codecs |
| `subscription` | Generic catch-up-then-live-tail loop + `WakeSource` traits (dep-free) |
| `snapshot` / `snapshot-json` | Aggregate snapshot persistence |
| `projection` / `projection-json` | Projection stepper primitives (no runner) |
| `export` / `import` | Backup/restore contract |
| `cbor` | Default CBOR backup box (implies `export` + `import`) |

## MSRV & stability

MSRV **1.99** (pinned stable, no nightly). `mnesis-store` is part of the **1.0 tier** — its documented trait semantics are semver surface. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).

### Import routing

Within one import request, every section must route to a distinct target stream
by raw bytes, including empty/corrupt sections and repeated origins. Both
`WholeChunk` and `PerStream` evaluate routing once per section and validate the
entire target list before writing. A duplicate returns typed
`ImportError::InvalidRoute` with the target, first index and duplicate index;
no section is appended and no OCC retry is indicated. Per-stream partial commits
remain possible for later storage/version failures after valid routing.

Raw `AtomicAppend::atomic_append_many` independently enforces distinct targets
and returns `AtomicAppendError::InvalidRoute` before storage work. It also
validates runs against their declared expected versions before consulting
storage heads: malformed versions return `AtomicAppendError::InvalidRun`,
including the run index and exact expected/actual version or required successor
overflow. A last event at `u64::MAX` needs no successor and remains valid input.
`Conflict` describes a storage head mismatch for an otherwise valid request. Version
adjacency never grants permission to merge origins implicitly. To continue a
stream across separate import requests, provide its explicit expected version
in each request; to combine origins, create and validate a deliberate merged
history outside this interface.

## Consistent export

`EventExporter` reads streams independently. For a backup while writers are
active, use `ConsistentExporter::open_export_session(lifetime)` and the returned
`ExportSession` for both listing and every stream read. A session includes one
complete committed view, so an intervening cross-stream transaction cannot
split the backup. In-memory and Fjall implement this capability. The session
borrows its source, and its cursors borrow it; release cursors before closing.

Choose a finite nonzero lifetime. `ExportError` preserves the adapter error or
identifies invalid lifetime/expiration. A failed export is incomplete and must
be restarted before restoring it. Ordinary per-stream export semantics remain
unchanged; import allocates new destination global positions separately from
the stored envelopes. Adapter docs describe snapshot retention and resource
limits. The contract and error types remain usable with `no_std + alloc`.

CBOR backups use format 2. Export all streams from one `ExportSession`, then
call `ChunkWriter::finish()` to write completion evidence. `decode_chunk`
validates the entire artifact before returning sections for import; missing
completion, corruption and trailing bytes reject before destination writes.
`salvage_chunk` is an explicit recovery path for partial or legacy format-1
artifacts and retains their header and completion status. See the
[backup format and recovery procedure](../../docs/backup-format.md).
