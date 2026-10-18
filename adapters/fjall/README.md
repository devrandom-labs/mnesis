# mnesis-fjall

Embedded **[fjall](https://crates.io/crates/fjall) LSM-tree** event-store adapter for [Mnesis](https://github.com/devrandom-labs/mnesis) — the default on-device store for the IoT/mobile-first target. Implements `RawEventStore`, `WakeSource`, `AtomicAppend`, `StreamLister`, and the `SnapshotStore` cache and conditional `CheckpointStore` seams.

Reads reuse aligned Fjall buffers through its `bytes_1` feature. Unaligned SST
buffers are copied once into an aligned owner so payload addresses remain
16-byte aligned for borrowing decoders. Envelopes retain ownership of that buffer.

## Quickstart

```toml
[dependencies]
mnesis-fjall = "0.3.1"
```

```rust
let store = FjallStore::builder(path).open()?.into_store();
```

## Features

| Feature | Adds |
|---------|------|
| `snapshot` | Aggregate snapshot persistence |
| `projection` | Projection-state persistence (all-levels LZ4 partition) |
| `export` / `import` | Backup/restore (`StreamLister`, `ConsistentExporter`, `AtomicAppend`) |

## MSRV & stability

MSRV **1.99**. Ships in the **0.x tier** — storage internals iterate without forcing kernel major bumps. The on-disk frame format is major-bounded (see STABILITY.md). See [STABILITY.md](../../STABILITY.md).

Stream IDs must contain 1–65,517 bytes in either index mode (`MAX_STREAM_ID_LEN`).
Snapshot and projection IDs use raw keys and permit 1–65,535 bytes (`MAX_KEY_LEN`).
Invalid IDs return `FjallError::InvalidKey` before engine access. State payloads
plus their header (12 bytes for snapshots, 20 for checkpoints) must fit the engine's u32 value-length limit; event
frame size is checked by the shared wire encoder.

Database creation synchronously persists layout version 2 and the selected
`AllIndex` mode. Reopening requires that same mode, even for an empty store.
Malformed manifests, unknown layouts and nonempty databases without a manifest
return errors. Layout version 1 is also rejected: its unconditional projection records cannot silently become revision-checked checkpoints. Empty unmarked databases can initialize normally.

To migrate a nonempty legacy database, export it with the original adapter and
import into a new database, then rebuild derived state from the imported log. Do not add a manifest to legacy data by hand. A
legacy disabled index has no original global order to recover: any new ordering
is a new history and requires discarding old global checkpoints.

The default `Durability::SyncAll` requires a journal data/metadata sync before
acknowledging event, atomic multi-stream, snapshot or projection writes.
`SyncData` requires a data sync where that is sufficient for the host filesystem.
`Buffered` flushes to OS buffers and does not promise power-loss survival;
`store.flush().await` persists all preceding completed writes on the blocking worker. These modes change
persistence, not transaction consistency. The layout manifest always uses `SyncAll`.

Persistence depends on the filesystem and device honoring sync requests. A
journal write or sync error may leave an uncertain commit outcome, and Fjall
may poison the database. Close/reopen and inspect committed versions and
checkpoints before retrying. An error does not promise rollback. Wakes and
successful acknowledgments follow completion of the selected persistence policy.

The `crash_recovery` test kills acknowledged writers over a pipe handshake.
On Linux/macOS, `journal_faults` compiles a test-only C syscall injector using
the Nix shell's C compiler and exercises real journal write/sync errors in
isolated children. It checks error reporting, wakes, complete recovery records
and counters. These tests cover process termination and injected EIO; they do
not simulate loss of the OS page cache or measure hardware power-loss survival.

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).

### Async execution and shutdown

Opening and recovery through `FjallStore::builder(...).open()` are synchronous
startup work. Store methods submit synchronous engine operations to one dedicated
worker; no Tokio runtime is required. Scan polling consumes buffered rows and
fetches subsequent batches on that worker. `BlockingConfig` bounds the number of
queued jobs, live cursors plus queued cleanup jobs, and the rows and serialized
bytes per batch. `max_open_scans` defaults to 32; opening a scan at the limit
returns `ScanCapacity` instead of waiting while callers may retain existing
scans. Every cursor reserves cleanup capacity before it is opened, so dropping
a scan sends its cursor to the worker without waiting or doing filesystem work. One row may exceed the
byte budget. Engine caches, retained snapshots, caller-owned inputs and the size
of an individual write have separate limits; these settings do not bound total
process memory. Inputs are copied only after queue admission.

Dropping a future cancels waiting admission and skips queued work that has not
started. Started operations finish, including persistence and wake bookkeeping;
a dropped future does not promise rollback. A worker panic returns a typed error
and leaves the write outcome uncertain. Inspect committed history before retrying.

`store.close().await` drains work and waits for engine shutdown. Release other
store clones and unfinished scan streams first; otherwise it returns
`OutstandingHandles`. Exhausted scans release their worker handle. Dropping the
last handle also closes admission and drains work in the background, but only
explicit close provides a shutdown boundary before reopening or deleting the
store directory. Canceling close still allows background shutdown to finish.

`CheckpointStore<Vec<u8>, Version>` and `CheckpointStore<Vec<u8>, GlobalSeq>` use
separate `checkpoints_stream` and `checkpoints_global` keyspaces. Records contain
schema, revision, position and payload. A single serialized writer transaction
compares the expected revision, validates schema/position, and replaces the
whole record using the selected durability policy. Revisions never reset on a
schema rebuild. Snapshot cache writes cannot bypass checkpoint revisions.

The projection owns its state; `flush()` saves its matching pair. Failed or
canceled writes require reloading, since they may have committed. Schema
replacement requires an explicit `Projection::rebuild`.

### Consistent multi-stream export

With `export` enabled, call `ConsistentExporter::open_export_session(lifetime)`
and use that session's `list_streams` and `export_stream`. They share one
cross-keyspace Fjall view, including cursors opened after concurrent commits.
This works with either `$all` index mode. Ordinary `EventExporter` calls still
open independent views and cannot establish a consistent multi-stream backup.

Supply a finite nonzero `Duration`. The worker owns the snapshot and removes it
on the first worker turn at or after its deadline, even if callers leave the
session and cursors idle. Timer resolution, scheduling and an already-running
synchronous operation can delay cleanup; no hard real-time deadline is promised.
The adapter's private timer driver requires no Tokio runtime on the calling
thread. Zero or an unrepresentable lifetime returns `ExportError::InvalidLifetime`;
expired operations return `ExportError::Expired`. Every error makes the backup
incomplete: discard it and restart from a new session.

A live database-wide snapshot can retain old versions across keyspaces and delay
reclamation. Other readers may also retain older views. Each session and cursor
uses one of `BlockingConfig::max_open_scans` slots, including pending cleanup;
consistent export requires capacity for a session and at least one cursor.
An expired handle keeps its admission slot until released, but it no longer
retains the database view. Each fetch opens a temporary iterator on the worker,
returns a batch subject to the configured row/byte limits, and drops the
iterator there. Already-yielded envelopes retain their bytes independently.

Release cursors before `ExportSession::close().await`, which waits for snapshot
cleanup. Drop queues cleanup without waiting, using reserved capacity even when
the ordinary job queue is full. The source store remains borrowed while a
session is live; release sessions before `FjallStore::close().await`.
