# Conformance Kit PR1 Implementation Plan (#281)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand `mnesis-store-testing` into the full 4-category conformance kit with `conformance!` macro entry points, and switch the three adapters onto it.

**Architecture:** Public generic async check fns organized into `sequence` / `boundary` / `linearizability` / `lifecycle` modules (plus `atomic` / `snapshot` capability modules), driven by a factory contract `Fn() -> Fut<Output = (S, C)>` where `S: RawEventStore + WakeSource` and `C` is an adapter-chosen keep-alive guard (fjall's `TempDir`; `()` for in-memory). `macro_rules!` entry points generate one `#[tokio::test]` per check so nextest reports each contract rule as its own named test. The existing two suites stay as a legacy shim until each adapter ports (every commit must pass the pre-commit `nix flake check`), then die.

**Tech Stack:** Rust 2024, tokio (`macros, rt, rt-multi-thread, sync, time`), futures, mnesis-store (`subscription` feature; `snapshot`/`import` behind kit features).

**Branch:** `feat/281-conformance-kit` (already created off `origin/main`).

**Spec:** `docs/superpowers/specs/2026-07-10-281-conformance-kit-design.md`

---

## Pinned API facts (verified against source, 2026-07-10)

Do NOT re-derive these; they were read from the current code.

- `RawEventStore::append(&self, id: &StreamKey, expected_version: Option<Version>, envelopes: &[PendingEnvelope]) -> Result<(), AppendError<Self::Error>>`. Adapters validate BOTH the head (`expected_version` vs actual) AND that each envelope's version is sequential from `expected+1` — a version-gap in the envelopes themselves is an `AppendError::Conflict` (verified in `adapters/inmemory/src/lib.rs:402-419`).
- `AppendError::Conflict { stream_id: ErrorId, expected: Option<Version>, actual: Option<Version> } | Store(E)`.
- `read_stream(&StreamKey, from: Version) -> Result<Self::Stream, Self::Error>` — `from` INCLUSIVE. Reading an absent stream yields an empty stream, not an error.
- `read_all(Option<Self::AllPosition>) -> Result<Self::AllStream, Self::Error>` — `from` EXCLUSIVE; items are `(AllPosition, PersistedEnvelope)`; positions strictly increasing, NOT gapless.
- `WakeSource::register(&self, stream: Option<&[u8]>) -> Result<Self::Registration, Self::Error>`; `wake(&self, stream: &[u8])`.
- `Subscription::new(&Store<S>)`; `subscribe<I: Id>(&self, id: &I, from: Option<Version>) -> Result<impl Stream<Item = Result<Step<PersistedEnvelope>, S::Error>> + Send + use<S, I>, <S as WakeSource>::Error>` — the stream is OWNED (`use<S, I>`, no `&self` borrow) but `!Unpin` (needs `pin_mut!`). `subscribe_all(Option<AllPosition>)` yields `Step<(AllPosition, PersistedEnvelope)>`.
- `Step<T> = Event(T) | CaughtUp` — `CaughtUp` emitted exactly once at backlog→live.
- `AtomicAppend::atomic_append_many(&self, writes: &[PlannedAppend]) -> Result<(), AtomicAppendError<Self::Error>>`; `PlannedAppend { target: StreamKey, expected_version: Option<Version>, events: Vec<PendingEnvelope> }`; `AtomicAppendError::Conflict { index, actual } | Store(E)`. Empty `writes` is a no-op `Ok(())`. Gated behind mnesis-store feature `import`.
- `SnapshotStore<S, P>::hydrate(&self, id: &impl Id, schema_version: NonZeroU32) -> Result<Hydrated<S, P>, Self::Error>`; `commit(&self, id: &impl Id, schema_version: NonZeroU32, position: P, state: &S)`. `Hydrated = Absent | Stale { stored_schema: NonZeroU32 } | Found { position, state }`. `InMemorySnapshotStore<S, P>` is generic over `P`. Gated behind mnesis-store feature `snapshot`.
- Envelope builder: `pending_envelope(Version) -> NeedsEventType`; `.event_type(&'static str)` (infallible) or `.event_type_bytes(Bytes) -> Result<WithEventType, EnvelopeError>` (dynamic); `.payload(impl Into<Bytes>)`; then optional `.schema_version(SchemaVersion)` / `.metadata(impl Into<Bytes>)`; `.build() -> Result<PendingEnvelope, EnvelopeError>`.
- `SchemaVersion::new(NonZeroU32)` (infallible) and `SchemaVersion::from_u32(u32) -> Result<_, ValueError>`.
- `Id` has a blanket impl for any `Clone + Send + Sync + Debug + Hash + Eq + Display + AsRef<[u8]> + 'static` type — `crates/store/tests/subscription_tests.rs` defines `TestId(String)` with only those impls and passes it to `subscribe`.
- `PersistedEnvelope` accessors: `version() -> Version`, `event_type() -> &str`, `schema_version() -> u32`, `payload() -> &[u8]`. Metadata accessor: VERIFY the exact name in Task 1 (expected `metadata() -> Option<&[u8]>`; fjall has `metadata_roundtrip_tests.rs` proving it exists — copy whatever those tests call).
- `CATCHUP_CHUNK` = 1024 (`crates/store/src/subscription_cursor.rs`) — the subscription loop reopens its scan every 1024 delivered rows.
- fjall rejects an EMPTY stream id (adapter limitation) — the kit only uses non-empty ids.
- Postgres tests skip when `DATABASE_URL` is unset and run serially (nixosTest passes `--test-threads=1`); each test TRUNCATEs first.
- Dev-dep cycles are legal when path-only (no `version =`): the workspace already runs `mnesis-store (dev)→ mnesis-inmemory (lib)→ mnesis-store`.

## File structure

```
crates/store-testing/
  Cargo.toml            — modify: features (snapshot, atomic-append), deps (bytes, tokio features), dev-deps (self, mnesis-inmemory)
  src/lib.rs            — modify: becomes docs + mod decls + macros + legacy shim (old assert_* fns kept until adapters port, deleted in Task 11)
  src/row.rs            — create: ConformanceRow (+metadata field), SubId, envelope_for/append/drain helpers
  src/sequence.rs       — create: 17 Sequence/Protocol checks (absorbs both legacy suites)
  src/boundary.rs       — create: 6 Defensive Boundary checks
  src/linearizability.rs— create: 4 concurrency checks
  src/lifecycle.rs      — create: 4 opt-in reopen checks
  src/atomic.rs         — create (feature atomic-append): 3 AtomicAppend checks
  src/snapshot.rs       — create (feature snapshot): 3 SnapshotStore checks
  tests/self_check.rs   — create: full kit run against InMemoryStore (the kit's own CI)
adapters/inmemory/tests/inmemory_conformance.rs — rewrite onto macros
adapters/fjall/tests/fjall_conformance.rs       — rewrite onto macros (+ lifecycle)
adapters/postgres/tests/conformance_tests.rs    — rewrite onto macros (skip_unless + lifecycle)
```

**Note on TDD inversion:** the deliverable IS a test suite; its "failing test" is a reference adapter run. `tests/self_check.rs` against `InMemoryStore` is the red/green loop: after each check module lands, the self-check exercises it. Until Task 7 (macros) the self-check calls check fns directly.

**Commit discipline:** the pre-commit hook runs `nix flake check` — never bypass it (no `--no-verify` on code commits). Run `nix develop -c cargo fmt --all` before every commit. `git add` new files BEFORE the commit attempt (flake check ignores untracked files).

---

### Task 1: Cargo wiring + `row.rs` (shared helpers) + lib.rs reshuffle

**Files:**
- Modify: `crates/store-testing/Cargo.toml`
- Create: `crates/store-testing/src/row.rs`
- Modify: `crates/store-testing/src/lib.rs` (move `ConformanceRow` out; keep the two legacy suites compiling)
- Create: `crates/store-testing/tests/self_check.rs` (skeleton)

- [ ] **Step 1.1: Update Cargo.toml**

```toml
[package]
name = "mnesis-store-testing"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
description = "Executable conformance kit for mnesis-store adapters — the store contract as a runnable test suite"
readme = "../../README.md"
keywords = ["event-sourcing", "testing", "conformance"]
categories = ["data-structures", "development-tools::testing"]

[features]
snapshot = ["mnesis-store/snapshot"]
atomic-append = ["mnesis-store/import"]

[dependencies]
bytes = { workspace = true }
futures = { workspace = true, features = ["std", "async-await", "executor"] }
mnesis = { version = "0.1.0", path = "../mnesis" }
mnesis-store = { version = "0.1.0", path = "../store", features = ["subscription"] }
tokio = { workspace = true, features = ["macros", "rt", "rt-multi-thread", "sync", "time"] }
workspace-hack = { version = "0.1", path = "../workspace-hack" }

[dev-dependencies]
# Path-only (no version) — stripped at publish; unifies kit features for the
# self-check test target (flake nextest runs default features only).
mnesis-store-testing = { path = ".", features = ["snapshot", "atomic-append"] }
mnesis-inmemory = { path = "../../adapters/inmemory" }

[lints]
workspace = true
```

Check whether `mnesis-inmemory` gates `InMemorySnapshotStore` / `AtomicAppend` behind features (`rg "feature" adapters/inmemory/Cargo.toml`); if so add those features to the dev-dep.

- [ ] **Step 1.2: Verify the `PersistedEnvelope` metadata accessor name**

Run: `rg -n "\.metadata\(\)" adapters/fjall/tests/metadata_roundtrip_tests.rs | head -3`
Expected: calls like `env.metadata()`. If the name differs, use the actual name everywhere this plan writes `env.metadata()`.

- [ ] **Step 1.3: Write `src/row.rs`**

```rust
//! Shared test-data row, id type, and drive/drain helpers used by every
//! conformance module.

use core::fmt;

use bytes::Bytes;
use futures::StreamExt;
use futures::pin_mut;
use mnesis::Version;
use mnesis_store::envelope::{PendingEnvelope, PersistedEnvelope, pending_envelope};
use mnesis_store::store::RawEventStore;
use mnesis_store::value::SchemaVersion;
use mnesis_store::StreamKey;
use std::num::NonZeroU32;

/// One row of test data fed into an adapter for the conformance suite to
/// observe back out. All fields must round-trip byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceRow {
    pub version: u64,
    pub event_type: String,
    pub schema_version: u32,
    pub payload: Vec<u8>,
    pub metadata: Option<Vec<u8>>,
}

impl ConformanceRow {
    /// Convenience constructor: `schema_version = 1`, no metadata.
    #[must_use]
    pub fn new(version: u64, event_type: &str, payload: Vec<u8>) -> Self {
        Self {
            version,
            event_type: event_type.to_owned(),
            schema_version: 1,
            payload,
            metadata: None,
        }
    }

    /// Set the schema version (defaults to 1).
    #[must_use]
    pub fn with_schema_version(mut self, schema_version: u32) -> Self {
        self.schema_version = schema_version;
        self
    }

    /// Attach metadata (defaults to absent).
    #[must_use]
    pub fn with_metadata(mut self, metadata: Vec<u8>) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

/// Subscription/snapshot id: satisfies the `Id` blanket bounds.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SubId(String);

impl SubId {
    #[must_use]
    pub fn new(s: &str) -> Self {
        Self(s.to_owned())
    }

    /// The `StreamKey` carrying the same bytes — for driving `append` on the
    /// stream this id subscribes to.
    #[must_use]
    pub fn key(&self) -> StreamKey {
        StreamKey::from_slice(self.0.as_bytes())
    }
}

impl fmt::Display for SubId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<[u8]> for SubId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

/// Build the `PendingEnvelope` a row describes. Panics on invalid rows — the
/// suite only constructs valid ones.
#[must_use]
pub fn envelope_for(row: &ConformanceRow) -> PendingEnvelope {
    let version = Version::new(row.version).expect("row version must be >= 1");
    let mut staged = pending_envelope(version)
        .event_type_bytes(Bytes::from(row.event_type.clone().into_bytes()))
        .expect("valid event type")
        .payload(row.payload.clone())
        .schema_version(SchemaVersion::new(
            NonZeroU32::new(row.schema_version).expect("schema_version must be >= 1"),
        ));
    if let Some(m) = &row.metadata {
        staged = staged.metadata(m.clone());
    }
    staged.build().expect("valid envelope")
}

/// Read a `PersistedEnvelope` back into row form.
#[must_use]
pub fn row_of(env: &PersistedEnvelope) -> ConformanceRow {
    ConformanceRow {
        version: env.version().as_u64(),
        event_type: env.event_type().to_owned(),
        schema_version: env.schema_version(),
        payload: env.payload().to_vec(),
        metadata: env.metadata().map(<[u8]>::to_vec),
    }
}

/// Append `rows` to `id` as one batch on a fresh stream (`expected = None`).
pub async fn append_rows<S: RawEventStore>(store: &S, id: &StreamKey, rows: &[ConformanceRow]) {
    if rows.is_empty() {
        return;
    }
    let envs: Vec<PendingEnvelope> = rows.iter().map(envelope_for).collect();
    store
        .append(id, None, &envs)
        .await
        .unwrap_or_else(|e| panic!("append of {} rows failed: {e:?}", rows.len()));
}

/// Append one event at `version` with the matching optimistic expectation
/// (`None` for version 1). Panics on failure — callers drive clean sequences.
pub async fn append_event<S: RawEventStore>(
    store: &S,
    id: &StreamKey,
    version: u64,
    payload: &[u8],
) {
    let expected = Version::new(version.saturating_sub(1));
    let env = envelope_for(&ConformanceRow::new(version, "E", payload.to_vec()));
    store
        .append(id, expected, &[env])
        .await
        .unwrap_or_else(|e| panic!("append v{version} failed: {e:?}"));
}

/// Drain `read_stream(id, from)` fully into rows.
pub async fn drain_stream<S: RawEventStore>(
    store: &S,
    id: &StreamKey,
    from: Version,
) -> Vec<ConformanceRow> {
    let stream = store
        .read_stream(id, from)
        .await
        .unwrap_or_else(|e| panic!("read_stream failed: {e:?}"));
    pin_mut!(stream);
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let env = item.unwrap_or_else(|e| panic!("read_stream item errored: {e:?}"));
        out.push(row_of(&env));
    }
    out
}

/// Drain `read_all(from)` fully into `(position, payload)` pairs.
pub async fn drain_all<S: RawEventStore>(
    store: &S,
    from: Option<S::AllPosition>,
) -> Vec<(S::AllPosition, Vec<u8>)> {
    let stream = store
        .read_all(from)
        .await
        .unwrap_or_else(|e| panic!("read_all failed: {e:?}"));
    pin_mut!(stream);
    let mut out = Vec::new();
    while let Some(item) = stream.next().await {
        let (pos, env) = item.unwrap_or_else(|e| panic!("read_all item errored: {e:?}"));
        out.push((pos, env.payload().to_vec()));
    }
    out
}

/// Assert positions strictly increase (monotonic, no duplicate).
pub fn assert_strictly_increasing<P: Copy + Ord + fmt::Debug>(positions: &[(P, Vec<u8>)]) {
    for w in positions.windows(2) {
        assert!(
            w[1].0 > w[0].0,
            "$all positions must be strictly increasing: {:?} then {:?}",
            w[0].0,
            w[1].0,
        );
    }
}
```

NOTE: `append_event` uses `saturating_sub` deliberately — `Version::new(0)` is `None`, which IS the correct expectation for version 1. This is test-harness code; rule 2 targets production paths.

- [ ] **Step 1.4: Reshuffle `src/lib.rs`**

Keep the crate-level `#![allow]` block and BOTH legacy suites (`assert_event_stream_conformance`, `assert_all_stream_conformance` and their private helpers) exactly as they are — the three adapters still call them. Changes only:

1. Delete the old `ConformanceRow` definition (struct + impl) from lib.rs.
2. Add at the top (after the allows):

```rust
pub mod row;

pub use row::{ConformanceRow, SubId};
```

3. The legacy `drain` helper constructs `ConformanceRow` literally — add the new field to that one construction site: `metadata: env.metadata().map(<[u8]>::to_vec),`.

- [ ] **Step 1.5: Self-check skeleton `tests/self_check.rs`**

```rust
//! The kit's own red/green loop: every conformance check runs against
//! `InMemoryStore`, the reference adapter. A check that fails here is a kit
//! bug (or a real InMemoryStore contract violation — either way, a find).

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use mnesis_inmemory::InMemoryStore;

fn factory() -> impl std::future::Future<Output = (InMemoryStore, ())> + Send {
    async { (InMemoryStore::new(), ()) }
}

// Check modules land in Tasks 2-6; the macro invocation replaces these direct
// calls in Task 7. For now this file only proves the crate + factory compile.
#[tokio::test]
async fn factory_compiles() {
    let (_store, ()) = factory().await;
}
```

- [ ] **Step 1.6: Verify compile**

Run: `nix develop -c cargo check -p mnesis-store-testing --all-features && nix develop -c cargo nextest run -p mnesis-store-testing`
Expected: clean check; `factory_compiles` passes.

- [ ] **Step 1.7: Format + commit**

```bash
nix develop -c cargo fmt --all
git add crates/store-testing docs/superpowers/plans/2026-07-10-281-conformance-kit-pr1.md
git commit -m "feat(store-testing): row helpers + factory contract groundwork (#281)"
```

(The hook runs `nix flake check`; if it flags hakari, run `nix develop -c cargo hakari generate` and re-commit.)

---

### Task 2: `sequence.rs` — read/append protocol checks (part 1: per-stream + conflict)

**Files:**
- Create: `crates/store-testing/src/sequence.rs`
- Modify: `crates/store-testing/src/lib.rs` (add `pub mod sequence;`)
- Modify: `crates/store-testing/tests/self_check.rs`

- [ ] **Step 2.1: Write `src/sequence.rs` (part 1)**

Every check in this file shares one signature shape; `FactoryOutput` is the `(store, guard)` pair:

```rust
//! Sequence/Protocol conformance: multi-step interactions on one store —
//! append→read round-trips, optimistic-conflict protocol, `$all` ordering and
//! resume, and the subscription catch-up→live protocol.

use core::future::Future;

use futures::StreamExt;
use futures::pin_mut;
use mnesis::Version;
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::WakeSource;
use mnesis_store::{AppendError, StreamKey};

use crate::row::{ConformanceRow, append_rows, drain_stream, envelope_for};

// Task 3 extends this import block with: core::time::Duration,
// tokio::time::timeout, mnesis_store::{Step, Subscription}, and
// crate::row::{SubId, append_event, assert_strictly_increasing, drain_all} —
// unused imports are DENIED, so add them only when their checks land.

/// A fresh, empty stream reads back empty (absent stream = empty, not error).
pub async fn check_empty_read_yields_none<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let got = drain_stream(&store, &StreamKey::from_slice(b"missing"), Version::INITIAL).await;
    assert!(
        got.is_empty(),
        "reading an absent stream must yield an empty stream, got {} rows",
        got.len(),
    );
}

/// Mixed-shape rows round-trip byte-for-byte in insertion order: Unicode and
/// dotted event types, schema versions across the u32 range, payloads from
/// empty through 4 KiB, metadata absent and present.
pub async fn check_append_then_read_round_trips<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"round-trip");
    let rows = vec![
        ConformanceRow::new(1, "Created", vec![]),
        ConformanceRow::new(2, "user.signed_up", vec![0]).with_schema_version(7),
        ConformanceRow::new(3, "ÉvénementUTF8", vec![0; 64]).with_metadata(vec![1, 2, 3]),
        ConformanceRow::new(4, "with spaces 123", vec![0xff; 64]).with_schema_version(u32::MAX),
        ConformanceRow::new(5, "E", (0..=255u8).collect()),
        ConformanceRow::new(
            6,
            "E",
            (0..4096u32).map(|i| u8::try_from(i % 256).unwrap_or(0)).collect(),
        ),
    ];
    append_rows(&store, &id, &rows).await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(got, rows, "rows must round-trip byte-for-byte in insertion order");
}

/// Versions read back strictly monotonic and the stream is fused after `None`.
pub async fn check_versions_strictly_monotonic_and_fused<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"monotonic");
    let rows: Vec<_> = (1..=64u64).map(|v| ConformanceRow::new(v, "E", vec![])).collect();
    append_rows(&store, &id, &rows).await;

    let stream = store
        .read_stream(&id, Version::INITIAL)
        .await
        .unwrap_or_else(|e| panic!("read_stream failed: {e:?}"));
    pin_mut!(stream);
    let mut versions = Vec::new();
    while let Some(item) = stream.next().await {
        versions.push(item.unwrap_or_else(|e| panic!("item errored: {e:?}")).version().as_u64());
    }
    let want: Vec<u64> = (1..=64).collect();
    assert_eq!(versions, want, "versions must be exactly 1..=64, strictly increasing");
    for i in 0..8 {
        assert!(
            stream.next().await.is_none(),
            "fused-after-None violated on repeat #{i}",
        );
    }
}

/// A stream larger than any internal batch/refill size (1500 events) drains
/// completely with no gap or duplicate across the seams.
pub async fn check_large_stream_completes<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"large");
    let rows: Vec<_> = (1..=1500u64).map(|v| ConformanceRow::new(v, "E", vec![])).collect();
    append_rows(&store, &id, &rows).await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    let want: Vec<u64> = (1..=1500).collect();
    assert_eq!(versions, want, "1500-event stream must drain exactly 1..=1500");
}

/// `read_stream(from)` is INCLUSIVE: from=3 on a 5-event stream yields 3,4,5.
pub async fn check_read_stream_from_is_inclusive<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"inclusive");
    let rows: Vec<_> = (1..=5u64).map(|v| ConformanceRow::new(v, "E", vec![])).collect();
    append_rows(&store, &id, &rows).await;
    let got = drain_stream(&store, &id, Version::new(3).expect("v3")).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(versions, vec![3, 4, 5], "read_stream(from=3) is inclusive: yields 3,4,5");
}

/// A mismatched `expected_version` surfaces `AppendError::Conflict` carrying
/// the store's actual head, and the store is untouched.
pub async fn check_append_conflict_is_surfaced<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"conflict");
    append_rows(
        &store,
        &id,
        &[ConformanceRow::new(1, "E", vec![1]), ConformanceRow::new(2, "E", vec![2])],
    )
    .await;

    // Stale expectation: stream head is 2, we claim it's still fresh.
    let env = envelope_for(&ConformanceRow::new(1, "E", vec![9]));
    let err = store
        .append(&id, None, &[env])
        .await
        .expect_err("appending with a stale expected_version must fail");
    match err {
        AppendError::Conflict { actual, .. } => {
            assert_eq!(
                actual,
                Version::new(2),
                "Conflict must carry the actual head (2)",
            );
        }
        AppendError::Store(e) => panic!("expected Conflict, got Store({e:?})"),
    }

    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(got.len(), 2, "a conflicted append must not land any event");
    assert_eq!(got[0].payload, vec![1]);
    assert_eq!(got[1].payload, vec![2]);
}

/// After a conflict, retrying with the corrected expectation succeeds — the
/// standard optimistic-concurrency protocol completes.
pub async fn check_append_retry_after_conflict_succeeds<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"retry");
    append_rows(&store, &id, &[ConformanceRow::new(1, "E", vec![1])]).await;

    // Conflict first…
    let stale = envelope_for(&ConformanceRow::new(1, "E", vec![9]));
    store
        .append(&id, None, &[stale])
        .await
        .expect_err("stale append must conflict");

    // …then the corrected retry (head is 1, next event is v2).
    let retry = envelope_for(&ConformanceRow::new(2, "E", vec![2]));
    store
        .append(&id, Version::new(1), &[retry])
        .await
        .expect("retry with corrected expected_version must succeed");

    let got = drain_stream(&store, &id, Version::INITIAL).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(versions, vec![1, 2], "retry lands exactly one new event");
}
```

- [ ] **Step 2.2: Wire into lib.rs and self-check**

lib.rs: add `pub mod sequence;` next to `pub mod row;`.

self_check.rs: replace the `factory_compiles` test with direct calls (temporary until Task 7):

```rust
use mnesis_store_testing::sequence;

#[tokio::test]
async fn sequence_part1() {
    sequence::check_empty_read_yields_none(&factory).await;
    sequence::check_append_then_read_round_trips(&factory).await;
    sequence::check_versions_strictly_monotonic_and_fused(&factory).await;
    sequence::check_large_stream_completes(&factory).await;
    sequence::check_read_stream_from_is_inclusive(&factory).await;
    sequence::check_append_conflict_is_surfaced(&factory).await;
    sequence::check_append_retry_after_conflict_succeeds(&factory).await;
}
```

(`factory` stays the fn from Task 1; passing `&factory` to a `F: Fn() -> Fut` parameter works because fn items implement `Fn`.)

- [ ] **Step 2.3: Run**

Run: `nix develop -c cargo nextest run -p mnesis-store-testing`
Expected: `sequence_part1` PASSES. If a check fails against `InMemoryStore`, STOP — that is either a kit bug or a genuine reference-adapter contract violation; diagnose before proceeding (do not weaken the assertion).

- [ ] **Step 2.4: Format + commit**

```bash
nix develop -c cargo fmt --all
git add crates/store-testing
git commit -m "feat(store-testing): sequence checks — round-trip + optimistic-conflict protocol (#281)"
```

---

### Task 3: `sequence.rs` part 2 — `$all` checks (ported) + subscription protocol

**Files:**
- Modify: `crates/store-testing/src/sequence.rs`
- Modify: `crates/store-testing/tests/self_check.rs`

- [ ] **Step 3.1: Port the six `$all` checks**

Append to `sequence.rs`. First extend the file's top import block (all imports stay at the top — no inline `use`):

```rust
use core::time::Duration;

use mnesis_store::{Step, Subscription};
use tokio::time::timeout;

use crate::row::{SubId, append_event, assert_strictly_increasing, drain_all};

/// Upper bound on any single subscription wait — a hang here means a lost
/// wake, which is exactly what the check exists to catch.
const WAIT: Duration = Duration::from_secs(10);
```

These are the legacy `assert_all_stream_conformance` checks re-shaped onto the `(S, C)` factory — same assertions, helpers from `row.rs`:

```rust
/// Empty store: `read_all(None)` yields nothing.
pub async fn check_all_empty_store_yields_none<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let got = drain_all(&store, None).await;
    assert!(got.is_empty(), "empty store: read_all(None) must yield nothing");
}

/// `read_all(None)` yields every event across streams in append (position)
/// order, positions strictly increasing.
pub async fn check_all_global_order_across_streams<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &a, 2, b"a2").await;
    append_event(&store, &b, 1, b"b1").await;
    append_event(&store, &a, 3, b"a3").await;
    append_event(&store, &a, 4, b"a4").await;

    let got = drain_all(&store, None).await;
    let payloads: Vec<Vec<u8>> = got.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        payloads,
        vec![b"a1".to_vec(), b"a2".to_vec(), b"b1".to_vec(), b"a3".to_vec(), b"a4".to_vec()],
        "read_all(None) must yield every event across streams in append order",
    );
    assert_strictly_increasing(&got);
}

/// `read_all(Some(p))` is EXCLUSIVE: strictly after `p`.
pub async fn check_all_from_is_exclusive<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &a, 2, b"a2").await;
    append_event(&store, &a, 3, b"a3").await;

    let full = drain_all(&store, None).await;
    assert_eq!(full.len(), 3);
    let checkpoint = full[0].0;

    let rest = drain_all(&store, Some(checkpoint)).await;
    let payloads: Vec<Vec<u8>> = rest.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        payloads,
        vec![b"a2".to_vec(), b"a3".to_vec()],
        "read_all(Some(p)) is EXCLUSIVE",
    );
    assert!(rest[0].0 > checkpoint, "resumed position must be strictly after checkpoint");
}

/// Multi-resume cycles reconstruct the single-shot read exactly — no gap,
/// duplicate, or skip across the seams.
pub async fn check_all_multi_resume_cycles<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    let mut va = 0u64;
    let mut vb = 0u64;
    let mut expected: Vec<Vec<u8>> = Vec::new();
    for i in 0..10u64 {
        if i % 2 == 0 {
            va += 1;
            let p = format!("a{va}").into_bytes();
            append_event(&store, &a, va, &p).await;
            expected.push(p);
        } else {
            vb += 1;
            let p = format!("b{vb}").into_bytes();
            append_event(&store, &b, vb, &p).await;
            expected.push(p);
        }
    }

    let full = drain_all(&store, None).await;
    let full_payloads: Vec<Vec<u8>> = full.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(full_payloads, expected, "single-shot read_all(None) must match append order");

    let mut acc: Vec<(S::AllPosition, Vec<u8>)> = Vec::new();
    let mut checkpoint: Option<S::AllPosition> = None;
    loop {
        let stream = store
            .read_all(checkpoint)
            .await
            .unwrap_or_else(|e| panic!("open read_all cycle failed: {e:?}"));
        pin_mut!(stream);
        let mut taken = 0;
        let mut advanced = false;
        while let Some(item) = stream.next().await {
            let (pos, env) = item.unwrap_or_else(|e| panic!("cycle item errored: {e:?}"));
            acc.push((pos, env.payload().to_vec()));
            checkpoint = Some(pos);
            advanced = true;
            taken += 1;
            if taken == 3 {
                break;
            }
        }
        if !advanced {
            break;
        }
    }

    let acc_payloads: Vec<Vec<u8>> = acc.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        acc_payloads, full_payloads,
        "multi-resume cycles must reconstruct the full stream exactly",
    );
    assert_strictly_increasing(&acc);
}

/// `read_all(Some(last))` is empty at the boundary; a later append surfaces
/// exactly the new event from the same checkpoint.
pub async fn check_all_boundary_then_new_append<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &b, 1, b"b1").await;

    let full = drain_all(&store, None).await;
    assert_eq!(full.len(), 2);
    let last = full.last().expect("non-empty").0;

    let empty = drain_all(&store, Some(last)).await;
    assert!(empty.is_empty(), "nothing is strictly after the last position");

    append_event(&store, &a, 2, b"a2").await;
    let after = drain_all(&store, Some(last)).await;
    let payloads: Vec<Vec<u8>> = after.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(payloads, vec![b"a2".to_vec()], "same checkpoint surfaces exactly the new event");
    assert!(after[0].0 > last, "new position must be strictly after the prior last");
}

/// Inclusive `read_stream` and exclusive `read_all` coexist on one store —
/// the intentional asymmetry (CLAUDE rule 4).
pub async fn check_read_stream_inclusive_read_all_exclusive_coexist<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &a, 2, b"a2").await;
    append_event(&store, &a, 3, b"a3").await;

    let got = drain_stream(&store, &a, Version::new(2).expect("v2")).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(versions, vec![2, 3], "read_stream(from=2) is INCLUSIVE");

    let full = drain_all(&store, None).await;
    assert_eq!(full.len(), 3);
    let pos_of_a2 = full[1].0;
    let after = drain_all(&store, Some(pos_of_a2)).await;
    let payloads: Vec<Vec<u8>> = after.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(payloads, vec![b"a3".to_vec()], "read_all(from=pos(a2)) is EXCLUSIVE");
}
```

- [ ] **Step 3.2: Subscription protocol checks**

Append to `sequence.rs`:

```rust
/// Take the next subscription item within `WAIT`, panicking on hang, stream
/// end, or read error. Returns the `Step`.
async fn next_step<St, T, E>(stream: &mut core::pin::Pin<&mut St>, what: &str) -> Step<T>
where
    St: futures::Stream<Item = Result<Step<T>, E>>,
    E: core::fmt::Debug,
{
    timeout(WAIT, stream.next())
        .await
        .unwrap_or_else(|_| panic!("{what}: subscription hung (lost wake?)"))
        .unwrap_or_else(|| panic!("{what}: subscription ended (must never return None)"))
        .unwrap_or_else(|e| panic!("{what}: subscription item errored: {e:?}"))
}

/// Per-stream subscription protocol: backlog in order, then `CaughtUp`
/// exactly once, then live events.
pub async fn check_subscription_backlog_then_caught_up_then_live<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("sub-proto");
    for v in 1..=3u64 {
        append_event(&store, &id.key(), v, format!("p{v}").as_bytes()).await;
    }

    let sub = Subscription::new(&store);
    let stream = sub.subscribe(&id, None).unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    for want in 1..=3u64 {
        match next_step(&mut stream, "backlog").await {
            Step::Event(env) => assert_eq!(
                env.version().as_u64(),
                want,
                "backlog must replay in version order",
            ),
            Step::CaughtUp => panic!("CaughtUp before the backlog drained (at v{want})"),
        }
    }
    match next_step(&mut stream, "boundary").await {
        Step::CaughtUp => {}
        Step::Event(env) => panic!("expected CaughtUp after backlog, got Event v{}", env.version()),
    }

    // Live phase: an append after CaughtUp is delivered.
    append_event(&store, &id.key(), 4, b"p4").await;
    match next_step(&mut stream, "live").await {
        Step::Event(env) => assert_eq!(env.version().as_u64(), 4, "live event must be v4"),
        Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
    }
}

/// `subscribe(Some(v))` resumes STRICTLY AFTER `v` — no duplicate of the
/// checkpointed event.
pub async fn check_subscription_resume_strict_after<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("sub-resume");
    for v in 1..=5u64 {
        append_event(&store, &id.key(), v, b"p").await;
    }

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe(&id, Some(Version::new(3).expect("v3")))
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    match next_step(&mut stream, "resume").await {
        Step::Event(env) => assert_eq!(
            env.version().as_u64(),
            4,
            "resume from Some(3) must deliver v4 first (strict-after, no dup)",
        ),
        Step::CaughtUp => panic!("expected v4 before CaughtUp"),
    }
}

/// `$all` subscription protocol: cross-stream backlog in position order, then
/// `CaughtUp` exactly once, then live events with strictly increasing tags.
pub async fn check_subscription_all_backlog_then_caught_up_then_live<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &b, 1, b"b1").await;
    append_event(&store, &a, 2, b"a2").await;

    let sub = Subscription::new(&store);
    let stream = sub
        .subscribe_all(None)
        .unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    let mut backlog: Vec<(S::AllPosition, Vec<u8>)> = Vec::new();
    loop {
        match next_step(&mut stream, "all backlog").await {
            Step::Event((pos, env)) => backlog.push((pos, env.payload().to_vec())),
            Step::CaughtUp => break,
        }
    }
    let payloads: Vec<Vec<u8>> = backlog.iter().map(|(_, p)| p.clone()).collect();
    assert_eq!(
        payloads,
        vec![b"a1".to_vec(), b"b1".to_vec(), b"a2".to_vec()],
        "$all backlog must replay in position order",
    );
    assert_strictly_increasing(&backlog);
    let last = backlog.last().expect("non-empty").0;

    append_event(&store, &b, 2, b"b2").await;
    match next_step(&mut stream, "all live").await {
        Step::Event((pos, env)) => {
            assert_eq!(env.payload(), b"b2", "live $all event must be the new append");
            assert!(pos > last, "live position must be strictly after the backlog");
        }
        Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
    }
}

/// A backlog larger than the catch-up chunk (1024) crosses the internal
/// rescan seams with no gap or duplicate, and `CaughtUp` still arrives.
pub async fn check_subscription_large_backlog_crosses_chunk_seam<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    const N: u64 = 2500; // > 2 × CATCHUP_CHUNK (1024)
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("sub-chunk");
    let rows: Vec<_> = (1..=N).map(|v| ConformanceRow::new(v, "E", vec![])).collect();
    append_rows(&store, &id.key(), &rows).await;

    let sub = Subscription::new(&store);
    let stream = sub.subscribe(&id, None).unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    let mut versions = Vec::with_capacity(usize::try_from(N).unwrap_or(usize::MAX));
    loop {
        match next_step(&mut stream, "chunk backlog").await {
            Step::Event(env) => versions.push(env.version().as_u64()),
            Step::CaughtUp => break,
        }
    }
    let want: Vec<u64> = (1..=N).collect();
    assert_eq!(
        versions, want,
        "backlog across chunk seams must be exactly 1..=N — no gap, no duplicate",
    );
}
```

- [ ] **Step 3.3: Extend self-check**

Add to `tests/self_check.rs`:

```rust
#[tokio::test]
async fn sequence_all_reads() {
    sequence::check_all_empty_store_yields_none(&factory).await;
    sequence::check_all_global_order_across_streams(&factory).await;
    sequence::check_all_from_is_exclusive(&factory).await;
    sequence::check_all_multi_resume_cycles(&factory).await;
    sequence::check_all_boundary_then_new_append(&factory).await;
    sequence::check_read_stream_inclusive_read_all_exclusive_coexist(&factory).await;
}

#[tokio::test]
async fn sequence_subscription() {
    sequence::check_subscription_backlog_then_caught_up_then_live(&factory).await;
    sequence::check_subscription_resume_strict_after(&factory).await;
    sequence::check_subscription_all_backlog_then_caught_up_then_live(&factory).await;
    sequence::check_subscription_large_backlog_crosses_chunk_seam(&factory).await;
}
```

- [ ] **Step 3.4: Run**

Run: `nix develop -c cargo nextest run -p mnesis-store-testing`
Expected: all three self-check tests PASS.

- [ ] **Step 3.5: Format + commit**

```bash
nix develop -c cargo fmt --all
git add crates/store-testing
git commit -m "feat(store-testing): sequence checks — \$all reads + subscription protocol (#281)"
```

---

### Task 4: `boundary.rs` — Defensive Boundary checks

**Files:**
- Create: `crates/store-testing/src/boundary.rs`
- Modify: `crates/store-testing/src/lib.rs` (add `pub mod boundary;`)
- Modify: `crates/store-testing/tests/self_check.rs`

- [ ] **Step 4.1: Write `src/boundary.rs`**

```rust
//! Defensive Boundary conformance: inputs that violate the append protocol
//! must be rejected cleanly and completely — nothing lands, nothing corrupts —
//! and legal-but-extreme values must round-trip.

use core::future::Future;

use bytes::Bytes;
use mnesis::Version;
use mnesis_store::envelope::pending_envelope;
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::WakeSource;
use mnesis_store::{AppendError, StreamKey};

use crate::row::{ConformanceRow, append_rows, drain_all, drain_stream, envelope_for};

/// A rejected append leaves the store byte-identical — per-stream AND `$all`.
pub async fn check_conflict_leaves_store_unchanged<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"unchanged");
    let rows = vec![
        ConformanceRow::new(1, "E", vec![1]).with_metadata(vec![7]),
        ConformanceRow::new(2, "E", vec![2]),
    ];
    append_rows(&store, &id, &rows).await;
    let before_stream = drain_stream(&store, &id, Version::INITIAL).await;
    let before_all = drain_all(&store, None).await;

    let stale = envelope_for(&ConformanceRow::new(1, "E", vec![9]));
    store
        .append(&id, None, &[stale])
        .await
        .expect_err("stale append must be rejected");

    let after_stream = drain_stream(&store, &id, Version::INITIAL).await;
    let after_all = drain_all(&store, None).await;
    assert_eq!(after_stream, before_stream, "per-stream contents changed by a REJECTED append");
    assert_eq!(
        after_all.len(),
        before_all.len(),
        "$all grew by a REJECTED append",
    );
}

/// Envelope versions must be sequential from `expected + 1`: a gap inside the
/// batch is rejected as a Conflict and nothing lands.
pub async fn check_version_gap_batch_rejected<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"gap");
    // Fresh stream, expected None ⇒ envelopes must be v1, v2, … — send v1, v3.
    let envs = vec![
        envelope_for(&ConformanceRow::new(1, "E", vec![1])),
        envelope_for(&ConformanceRow::new(3, "E", vec![3])),
    ];
    let err = store
        .append(&id, None, &envs)
        .await
        .expect_err("a version gap inside the batch must be rejected");
    assert!(
        matches!(err, AppendError::Conflict { .. }),
        "gap rejection must be the Conflict domain, got: {err:?}",
    );
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert!(got.is_empty(), "a rejected batch must land NOTHING (got {} rows)", got.len());
}

/// First envelope version must equal `expected + 1` — starting a fresh stream
/// at v3 is rejected.
pub async fn check_wrong_first_version_rejected<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"first");
    let envs = vec![envelope_for(&ConformanceRow::new(3, "E", vec![3]))];
    let err = store
        .append(&id, None, &envs)
        .await
        .expect_err("fresh stream starting at v3 must be rejected");
    assert!(matches!(err, AppendError::Conflict { .. }), "got: {err:?}");
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert!(got.is_empty(), "nothing may land");
}

/// Metadata `None` and `Some(empty)` are DISTINCT values and round-trip as
/// such (the wire absent-sentinel edge).
pub async fn check_metadata_absent_vs_empty_distinct<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"meta");
    let rows = vec![
        ConformanceRow::new(1, "E", vec![1]),                         // absent
        ConformanceRow::new(2, "E", vec![2]).with_metadata(vec![]),   // present, empty
        ConformanceRow::new(3, "E", vec![3]).with_metadata(vec![1, 2, 3]),
    ];
    append_rows(&store, &id, &rows).await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(got[0].metadata, None, "absent metadata must read back as None");
    assert_eq!(
        got[1].metadata,
        Some(vec![]),
        "empty metadata must read back as Some(empty), NOT None",
    );
    assert_eq!(got[2].metadata, Some(vec![1, 2, 3]));
}

/// A maximum-length event type (u16::MAX bytes) round-trips.
pub async fn check_max_length_event_type_round_trips<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"maxtype");
    let long = "a".repeat(usize::from(u16::MAX));
    let env = pending_envelope(Version::INITIAL)
        .event_type_bytes(Bytes::from(long.clone().into_bytes()))
        .expect("u16::MAX event type is within the cap")
        .payload(vec![1])
        .build()
        .expect("valid envelope");
    store
        .append(&id, None, &[env])
        .await
        .unwrap_or_else(|e| panic!("append max-len event type failed: {e:?}"));
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(got[0].event_type, long, "u16::MAX event type must round-trip");
}

/// Stream ids sharing a byte prefix ("a", "ab") are fully isolated — a
/// prefix-collision in the adapter's key encoding would leak events across.
pub async fn check_prefix_stream_ids_isolated<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let a = StreamKey::from_slice(b"a");
    let ab = StreamKey::from_slice(b"ab");
    let unicode = StreamKey::from_slice("поток-流".as_bytes());
    append_rows(&store, &a, &[ConformanceRow::new(1, "E", vec![1])]).await;
    append_rows(
        &store,
        &ab,
        &[ConformanceRow::new(1, "E", vec![2]), ConformanceRow::new(2, "E", vec![3])],
    )
    .await;
    append_rows(&store, &unicode, &[ConformanceRow::new(1, "E", vec![4])]).await;

    let got_a = drain_stream(&store, &a, Version::INITIAL).await;
    assert_eq!(got_a.len(), 1, "stream 'a' must not see 'ab' events");
    assert_eq!(got_a[0].payload, vec![1]);

    let got_ab = drain_stream(&store, &ab, Version::INITIAL).await;
    assert_eq!(got_ab.len(), 2, "stream 'ab' must not see 'a' events");

    let got_u = drain_stream(&store, &unicode, Version::INITIAL).await;
    assert_eq!(got_u.len(), 1, "unicode stream id must round-trip");
    assert_eq!(got_u[0].payload, vec![4]);
}
```

- [ ] **Step 4.2: Wire in (lib.rs `pub mod boundary;`; self_check test)**

```rust
use mnesis_store_testing::boundary;

#[tokio::test]
async fn boundary_checks() {
    boundary::check_conflict_leaves_store_unchanged(&factory).await;
    boundary::check_version_gap_batch_rejected(&factory).await;
    boundary::check_wrong_first_version_rejected(&factory).await;
    boundary::check_metadata_absent_vs_empty_distinct(&factory).await;
    boundary::check_max_length_event_type_round_trips(&factory).await;
    boundary::check_prefix_stream_ids_isolated(&factory).await;
}
```

- [ ] **Step 4.3: Run**

Run: `nix develop -c cargo nextest run -p mnesis-store-testing`
Expected: PASS. If `check_version_gap_batch_rejected` or `check_wrong_first_version_rejected` fails on `InMemoryStore`, the reference behavior diverges from the pinned facts — STOP and re-read `adapters/inmemory/src/lib.rs:402`; report the divergence to the user rather than weakening the check.

- [ ] **Step 4.4: Format + commit**

```bash
nix develop -c cargo fmt --all
git add crates/store-testing
git commit -m "feat(store-testing): defensive boundary checks (#281)"
```

---

### Task 5: `linearizability.rs` — concurrency checks

**Files:**
- Create: `crates/store-testing/src/linearizability.rs`
- Modify: `crates/store-testing/src/lib.rs` (add `pub mod linearizability;`)
- Modify: `crates/store-testing/tests/self_check.rs`

These checks REQUIRE a multi-thread runtime for real overlap (`#[tokio::test(flavor = "multi_thread")]` in self-check and in the macro).

- [ ] **Step 5.1: Write `src/linearizability.rs`**

```rust
//! Linearizability/Isolation conformance: genuinely-overlapping writers and a
//! parked subscriber. Real overlap via `tokio::spawn` + `Barrier` (CLAUDE
//! rule 8 — never sequential-then-check).

use core::future::Future;
use core::time::Duration;
use std::sync::Arc;

use futures::pin_mut;
use mnesis::Version;
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::WakeSource;
use mnesis_store::{AppendError, Step, StreamKey, Subscription};
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
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        let id = id.clone();
        handles.push(tokio::spawn(async move {
            let payload = vec![u8::try_from(i).unwrap_or(0)];
            let env = envelope_for(&ConformanceRow::new(1, "E", payload));
            barrier.wait().await;
            store.append(&id, None, &[env]).await
        }));
    }

    let mut winners = Vec::new();
    let mut conflicts = 0;
    for (i, h) in handles.into_iter().enumerate() {
        match h.await.expect("writer task panicked") {
            Ok(()) => winners.push(i),
            Err(AppendError::Conflict { .. }) => conflicts += 1,
            Err(AppendError::Store(e)) => panic!("writer {i} hit a Store error: {e:?}"),
        }
    }
    assert_eq!(winners.len(), 1, "exactly one concurrent appender must win, got {winners:?}");
    assert_eq!(conflicts, WRITERS - 1, "every loser must see Conflict");

    let got = drain_stream(&store, &id, Version::INITIAL).await;
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
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let id = StreamKey::from_slice(format!("s{i}").as_bytes());
            barrier.wait().await;
            for v in 1..=PER_STREAM {
                append_event(store.as_ref(), &id, v, format!("s{i}v{v}").as_bytes()).await;
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
        let want: Vec<Vec<u8>> =
            (1..=PER_STREAM).map(|v| format!("s{i}v{v}").into_bytes()).collect();
        let payloads: Vec<Vec<u8>> = got.iter().map(|r| r.payload.clone()).collect();
        assert_eq!(payloads, want, "stream s{i} must hold its own events in order");
    }
}

/// Wake-after-idle: a subscriber parked at `CaughtUp` is woken by a later
/// append from another task — the lost-wakeup race the arm-before-rescan
/// discipline exists to prevent.
pub async fn check_wake_after_idle<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("wake-idle");
    append_event(&store, &id.key(), 1, b"p1").await;

    let sub = Subscription::new(&store);
    let stream = sub.subscribe(&id, None).unwrap_or_else(|e| panic!("register failed: {e:?}"));
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
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    const BACKLOG: u64 = 100;
    const LIVE: u64 = 100;
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("boundary-race");
    let rows: Vec<_> = (1..=BACKLOG).map(|v| ConformanceRow::new(v, "E", vec![])).collect();
    append_rows(&store, &id.key(), &rows).await;

    let sub = Subscription::new(&store);
    let stream = sub.subscribe(&id, None).unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    // Writer races the reader's catch-up.
    let writer_store = store.clone();
    let key = id.key();
    let writer = tokio::spawn(async move {
        for v in (BACKLOG + 1)..=(BACKLOG + LIVE) {
            append_event(&writer_store, &key, v, b"live").await;
        }
    });

    let total = BACKLOG + LIVE;
    let mut versions = Vec::with_capacity(usize::try_from(total).unwrap_or(usize::MAX));
    let mut caught_up = 0u32;
    while versions.len() < usize::try_from(total).unwrap_or(usize::MAX) {
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

    assert_eq!(caught_up, 1, "CaughtUp must be emitted exactly once, got {caught_up}");
    let want: Vec<u64> = (1..=total).collect();
    assert_eq!(
        versions, want,
        "all {total} events must arrive exactly once, in order, across the boundary",
    );
}
```

- [ ] **Step 5.2: Wire in (lib.rs; self_check with multi_thread flavor)**

```rust
use mnesis_store_testing::linearizability;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn linearizability_checks() {
    linearizability::check_concurrent_same_stream_single_winner(&factory).await;
    linearizability::check_concurrent_distinct_streams_all_land(&factory).await;
    linearizability::check_wake_after_idle(&factory).await;
    linearizability::check_caught_up_boundary_race(&factory).await;
}
```

- [ ] **Step 5.3: Run** (same nextest command; expect PASS)

- [ ] **Step 5.4: Format + commit**

```bash
nix develop -c cargo fmt --all
git add crates/store-testing
git commit -m "feat(store-testing): linearizability checks — races, single-winner, wake-after-idle (#281)"
```

---

### Task 6: `lifecycle.rs` + `atomic.rs` + `snapshot.rs`

**Files:**
- Create: `crates/store-testing/src/lifecycle.rs`
- Create: `crates/store-testing/src/atomic.rs` (cfg `atomic-append`)
- Create: `crates/store-testing/src/snapshot.rs` (cfg `snapshot`)
- Modify: `crates/store-testing/src/lib.rs`
- Modify: `crates/store-testing/tests/self_check.rs`

- [ ] **Step 6.1: Write `src/lifecycle.rs`**

Factory shape here: `open() -> (S, C)` plus `reopen(S, C) -> (S, C)` — reopen consumes the old handle (drop closes it) and the context (tempdir path, connection url) and yields a store over the SAME storage.

```rust
//! Lifecycle conformance (opt-in): close → reopen must preserve events,
//! versions, and the `$all` position watermark. In-memory adapters have
//! nothing to reopen and skip this module.

use core::future::Future;

use mnesis::Version;
use mnesis_store::store::RawEventStore;
use mnesis_store::wake::WakeSource;
use mnesis_store::StreamKey;

use crate::row::{ConformanceRow, append_event, append_rows, drain_all, drain_stream};

/// Everything written before close reads back identically after reopen.
pub async fn check_reopen_preserves_events<S, C, O, OFut, R, RFut>(open: &O, reopen: &R)
where
    S: RawEventStore + WakeSource,
    C: Send,
    O: Fn() -> OFut + Send + Sync,
    OFut: Future<Output = (S, C)> + Send,
    R: Fn(S, C) -> RFut + Send + Sync,
    RFut: Future<Output = (S, C)> + Send,
{
    let (store, ctx) = open().await;
    let id = StreamKey::from_slice(b"persist");
    let rows = vec![
        ConformanceRow::new(1, "Created", vec![1]).with_metadata(vec![7]),
        ConformanceRow::new(2, "Updated", vec![2]).with_schema_version(3),
    ];
    append_rows(&store, &id, &rows).await;
    let before = drain_stream(&store, &id, Version::INITIAL).await;

    let (store, _ctx) = reopen(store, ctx).await;
    let after = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(after, before, "reopen must preserve every event byte-for-byte");
}

/// The `$all` position watermark survives reopen: a post-reopen append lands
/// strictly after every pre-close position (a reset counter would violate
/// resume and corrupt projections).
pub async fn check_reopen_preserves_position_watermark<S, C, O, OFut, R, RFut>(open: &O, reopen: &R)
where
    S: RawEventStore + WakeSource,
    C: Send,
    O: Fn() -> OFut + Send + Sync,
    OFut: Future<Output = (S, C)> + Send,
    R: Fn(S, C) -> RFut + Send + Sync,
    RFut: Future<Output = (S, C)> + Send,
{
    let (store, ctx) = open().await;
    let a = StreamKey::from_slice(b"a");
    let b = StreamKey::from_slice(b"b");
    append_event(&store, &a, 1, b"a1").await;
    append_event(&store, &b, 1, b"b1").await;
    let before = drain_all(&store, None).await;
    let last = before.last().expect("non-empty").0;

    let (store, _ctx) = reopen(store, ctx).await;
    append_event(&store, &a, 2, b"a2").await;

    let resumed = drain_all(&store, Some(last)).await;
    assert_eq!(resumed.len(), 1, "resume from the pre-close watermark must yield only the new event");
    assert_eq!(resumed[0].1, b"a2".to_vec());
    assert!(
        resumed[0].0 > last,
        "post-reopen position {:?} must be strictly after the pre-close last {:?} — the watermark must survive reopen",
        resumed[0].0,
        last,
    );
}

/// Optimistic-concurrency state survives reopen: a stale expectation still
/// conflicts against the persisted head.
pub async fn check_reopen_conflict_state_intact<S, C, O, OFut, R, RFut>(open: &O, reopen: &R)
where
    S: RawEventStore + WakeSource,
    C: Send,
    O: Fn() -> OFut + Send + Sync,
    OFut: Future<Output = (S, C)> + Send,
    R: Fn(S, C) -> RFut + Send + Sync,
    RFut: Future<Output = (S, C)> + Send,
{
    let (store, ctx) = open().await;
    let id = StreamKey::from_slice(b"head");
    append_event(&store, &id, 1, b"p1").await;
    append_event(&store, &id, 2, b"p2").await;

    let (store, _ctx) = reopen(store, ctx).await;
    let stale = crate::row::envelope_for(&ConformanceRow::new(1, "E", vec![9]));
    store
        .append(&id, None, &[stale])
        .await
        .expect_err("the persisted head must still conflict after reopen");
    // The corrected append succeeds.
    append_event(&store, &id, 3, b"p3").await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    let versions: Vec<u64> = got.iter().map(|r| r.version).collect();
    assert_eq!(versions, vec![1, 2, 3]);
}

/// A subscription opened after reopen catches up over the pre-close backlog.
pub async fn check_reopen_subscription_catches_up<S, C, O, OFut, R, RFut>(open: &O, reopen: &R)
where
    S: RawEventStore + WakeSource,
    C: Send,
    O: Fn() -> OFut + Send + Sync,
    OFut: Future<Output = (S, C)> + Send,
    R: Fn(S, C) -> RFut + Send + Sync,
    RFut: Future<Output = (S, C)> + Send,
{
    use core::time::Duration;

    use futures::pin_mut;
    use mnesis_store::{Step, Subscription};
    use tokio::time::timeout;

    let (store, ctx) = open().await;
    let id = crate::row::SubId::new("reopen-sub");
    for v in 1..=3u64 {
        append_event(&store, &id.key(), v, b"p").await;
    }

    let (raw, _ctx) = reopen(store, ctx).await;
    let store = raw.into_store();
    let sub = Subscription::new(&store);
    let stream = sub.subscribe(&id, None).unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    let mut versions = Vec::new();
    loop {
        match timeout(Duration::from_secs(10), futures::StreamExt::next(&mut stream))
            .await
            .expect("catch-up after reopen must not hang")
            .expect("subscription must not end")
            .unwrap_or_else(|e| panic!("item errored: {e:?}"))
        {
            Step::Event(env) => versions.push(env.version().as_u64()),
            Step::CaughtUp => break,
        }
    }
    assert_eq!(versions, vec![1, 2, 3], "reopen backlog must replay fully");
}
```

NOTE (violates "no inline use" only apparently): the `use` items inside `check_reopen_subscription_catches_up` must move to the top of the file with the rest — write them at the top; shown inline here only to keep the snippet self-contained. Final file has ALL imports at the top.

- [ ] **Step 6.2: Write `src/atomic.rs`**

```rust
//! `AtomicAppend` capability conformance: several per-stream runs commit in
//! ONE transaction — all land or none do.

use core::future::Future;

use mnesis::Version;
use mnesis_store::wake::WakeSource;
use mnesis_store::{AtomicAppend, AtomicAppendError, PlannedAppend, StreamKey};
// NOTE: RawEventStore is NOT imported — `AtomicAppend: RawEventStore` is a
// supertrait, and nothing here names the trait directly (unused imports deny).

use crate::row::{ConformanceRow, append_rows, assert_strictly_increasing, drain_all, drain_stream, envelope_for};

/// Three runs across three streams (two fresh, one existing) commit together.
pub async fn check_atomic_multi_stream_commits_all<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let existing = StreamKey::from_slice(b"existing");
    append_rows(&store, &existing, &[ConformanceRow::new(1, "E", vec![0])]).await;

    let writes = vec![
        PlannedAppend {
            target: StreamKey::from_slice(b"fresh-a"),
            expected_version: None,
            events: vec![envelope_for(&ConformanceRow::new(1, "E", vec![1]))],
        },
        PlannedAppend {
            target: StreamKey::from_slice(b"fresh-b"),
            expected_version: None,
            events: vec![
                envelope_for(&ConformanceRow::new(1, "E", vec![2])),
                envelope_for(&ConformanceRow::new(2, "E", vec![3])),
            ],
        },
        PlannedAppend {
            target: existing.clone(),
            expected_version: Version::new(1),
            events: vec![envelope_for(&ConformanceRow::new(2, "E", vec![4]))],
        },
    ];
    store
        .atomic_append_many(&writes)
        .await
        .unwrap_or_else(|e| panic!("atomic append must succeed: {e:?}"));

    assert_eq!(drain_stream(&store, &StreamKey::from_slice(b"fresh-a"), Version::INITIAL).await.len(), 1);
    assert_eq!(drain_stream(&store, &StreamKey::from_slice(b"fresh-b"), Version::INITIAL).await.len(), 2);
    assert_eq!(drain_stream(&store, &existing, Version::INITIAL).await.len(), 2);
    let all = drain_all(&store, None).await;
    assert_eq!(all.len(), 5, "$all must hold every committed event");
    assert_strictly_increasing(&all);
}

/// A conflict in ONE run aborts the WHOLE batch: no stream changes, the error
/// names the offending write index and the actual head.
pub async fn check_atomic_conflict_aborts_all<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let existing = StreamKey::from_slice(b"existing");
    append_rows(&store, &existing, &[ConformanceRow::new(1, "E", vec![0])]).await;
    let all_before = drain_all(&store, None).await;

    let writes = vec![
        PlannedAppend {
            target: StreamKey::from_slice(b"fresh-a"),
            expected_version: None,
            events: vec![envelope_for(&ConformanceRow::new(1, "E", vec![1]))],
        },
        PlannedAppend {
            // WRONG: head is 1, we claim fresh.
            target: existing.clone(),
            expected_version: None,
            events: vec![envelope_for(&ConformanceRow::new(1, "E", vec![9]))],
        },
    ];
    let err = store
        .atomic_append_many(&writes)
        .await
        .expect_err("a conflicting run must abort the batch");
    match err {
        AtomicAppendError::Conflict { index, actual } => {
            assert_eq!(index, 1, "the error must name the offending write");
            assert_eq!(actual, Version::new(1), "the error must carry the actual head");
        }
        AtomicAppendError::Store(e) => panic!("expected Conflict, got Store({e:?})"),
    }

    let fresh = drain_stream(&store, &StreamKey::from_slice(b"fresh-a"), Version::INITIAL).await;
    assert!(fresh.is_empty(), "NOTHING may land on any stream of an aborted batch");
    let all_after = drain_all(&store, None).await;
    assert_eq!(all_after.len(), all_before.len(), "$all must be untouched by an aborted batch");
}

/// An empty batch is a no-op `Ok`.
pub async fn check_atomic_empty_batch_is_noop<S, C, F, Fut>(factory: &F)
where
    S: AtomicAppend + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    store
        .atomic_append_many(&[])
        .await
        .unwrap_or_else(|e| panic!("empty atomic batch must be Ok: {e:?}"));
    assert!(drain_all(&store, None).await.is_empty(), "empty batch must write nothing");
}
```

Verify the import paths (`AtomicAppend`, `AtomicAppendError`, `PlannedAppend` re-exported at `mnesis_store::` root?) with `rg "pub use.*import" crates/store/src/lib.rs` — adjust to the actual re-export paths.

- [ ] **Step 6.3: Write `src/snapshot.rs`**

```rust
//! `SnapshotStore` capability conformance: state and position commit and
//! hydrate together; a schema change reads back as `Stale`, never as decode
//! garbage.

use core::fmt::Debug;
use core::future::Future;
use std::num::NonZeroU32;

use mnesis_store::state::{Hydrated, SnapshotStore};

use crate::row::SubId;

const SCHEMA_1: NonZeroU32 = NonZeroU32::new(1).expect("1 is non-zero");
const SCHEMA_2: NonZeroU32 = NonZeroU32::new(2).expect("2 is non-zero");

/// Fresh store hydrates `Absent`; after commit the same `(position, state)`
/// pair comes back `Found`.
pub async fn check_snapshot_absent_then_commit_then_found<SS, P, C, F, Fut>(
    factory: &F,
    p1: P,
    _p2: P,
) where
    SS: SnapshotStore<Vec<u8>, P>,
    P: Copy + Ord + Debug + Send + Sync + 'static,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (SS, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = SubId::new("snap");
    match store.hydrate(&id, SCHEMA_1).await.unwrap_or_else(|e| panic!("hydrate failed: {e:?}")) {
        Hydrated::Absent => {}
        other => panic!("fresh store must hydrate Absent, got {other:?}"),
    }

    let state = vec![1u8, 2, 3];
    store
        .commit(&id, SCHEMA_1, p1, &state)
        .await
        .unwrap_or_else(|e| panic!("commit failed: {e:?}"));
    match store.hydrate(&id, SCHEMA_1).await.unwrap_or_else(|e| panic!("hydrate failed: {e:?}")) {
        Hydrated::Found { position, state: got } => {
            assert_eq!(position, p1, "position must hydrate exactly as committed");
            assert_eq!(got, state, "state must hydrate byte-for-byte");
        }
        other => panic!("expected Found after commit, got {other:?}"),
    }
}

/// Hydrating under a different schema version yields `Stale` carrying the
/// stored schema — never `Found` with undecodable bytes, never `Absent`.
pub async fn check_snapshot_stale_on_schema_change<SS, P, C, F, Fut>(factory: &F, p1: P, _p2: P)
where
    SS: SnapshotStore<Vec<u8>, P>,
    P: Copy + Ord + Debug + Send + Sync + 'static,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (SS, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = SubId::new("snap-stale");
    store
        .commit(&id, SCHEMA_1, p1, &vec![1u8])
        .await
        .unwrap_or_else(|e| panic!("commit failed: {e:?}"));
    match store.hydrate(&id, SCHEMA_2).await.unwrap_or_else(|e| panic!("hydrate failed: {e:?}")) {
        Hydrated::Stale { stored_schema } => {
            assert_eq!(stored_schema, SCHEMA_1, "Stale must carry the stored schema version");
        }
        other => panic!("schema mismatch must hydrate Stale, got {other:?}"),
    }
}

/// A second commit fully replaces the first — latest `(position, state)` wins.
pub async fn check_snapshot_overwrite_latest_wins<SS, P, C, F, Fut>(factory: &F, p1: P, p2: P)
where
    SS: SnapshotStore<Vec<u8>, P>,
    P: Copy + Ord + Debug + Send + Sync + 'static,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (SS, C)> + Send,
{
    assert!(p1 < p2, "kit misuse: positions must be supplied in ascending order");
    let (store, _guard) = factory().await;
    let id = SubId::new("snap-latest");
    store
        .commit(&id, SCHEMA_1, p1, &vec![1u8])
        .await
        .unwrap_or_else(|e| panic!("first commit failed: {e:?}"));
    store
        .commit(&id, SCHEMA_1, p2, &vec![2u8])
        .await
        .unwrap_or_else(|e| panic!("second commit failed: {e:?}"));
    match store.hydrate(&id, SCHEMA_1).await.unwrap_or_else(|e| panic!("hydrate failed: {e:?}")) {
        Hydrated::Found { position, state } => {
            assert_eq!(position, p2, "latest committed position must win");
            assert_eq!(state, vec![2u8], "latest committed state must win");
        }
        other => panic!("expected Found, got {other:?}"),
    }
}
```

NOTE: `NonZeroU32::new(..).expect(..)` in `const` requires the const-stable `expect`; if the pinned toolchain rejects it, use `match NonZeroU32::new(1) { Some(v) => v, None => unreachable!() }` — verify at compile.

- [ ] **Step 6.4: Wire into lib.rs**

```rust
pub mod boundary;
pub mod lifecycle;
pub mod linearizability;
pub mod row;
pub mod sequence;

#[cfg(feature = "atomic-append")]
pub mod atomic;
#[cfg(feature = "snapshot")]
pub mod snapshot;
```

- [ ] **Step 6.5: Extend self-check**

```rust
use mnesis_store_testing::{atomic, lifecycle, snapshot};

// Lifecycle against InMemoryStore is a self-check ONLY: "reopen" hands back
// the same store (in-memory has nothing to close). It validates the kit's
// closure plumbing, not real persistence — fjall/postgres prove that part.
#[tokio::test]
async fn lifecycle_plumbing() {
    let open = || async { (InMemoryStore::new(), ()) };
    let reopen = |store: InMemoryStore, (): ()| async move { (store, ()) };
    lifecycle::check_reopen_preserves_events(&open, &reopen).await;
    lifecycle::check_reopen_preserves_position_watermark(&open, &reopen).await;
    lifecycle::check_reopen_conflict_state_intact(&open, &reopen).await;
    lifecycle::check_reopen_subscription_catches_up(&open, &reopen).await;
}

#[tokio::test]
async fn atomic_checks() {
    atomic::check_atomic_multi_stream_commits_all(&factory).await;
    atomic::check_atomic_conflict_aborts_all(&factory).await;
    atomic::check_atomic_empty_batch_is_noop(&factory).await;
}

#[tokio::test]
async fn snapshot_checks() {
    use mnesis::Version;
    use mnesis_inmemory::InMemorySnapshotStore;
    let sfactory = || async { (InMemorySnapshotStore::<Vec<u8>, Version>::new(), ()) };
    let p1 = Version::new(5).unwrap();
    let p2 = Version::new(9).unwrap();
    snapshot::check_snapshot_absent_then_commit_then_found(&sfactory, p1, p2).await;
    snapshot::check_snapshot_stale_on_schema_change(&sfactory, p1, p2).await;
    snapshot::check_snapshot_overwrite_latest_wins(&sfactory, p1, p2).await;
}
```

(Verify `InMemorySnapshotStore::new()`'s actual constructor signature with `rg "pub fn new" adapters/inmemory/src/snapshot.rs` and adjust.)

- [ ] **Step 6.6: Run**

Run: `nix develop -c cargo nextest run -p mnesis-store-testing`
Expected: PASS (self dev-dep unifies the `snapshot`/`atomic-append` features into the test build).

- [ ] **Step 6.7: Format + commit**

```bash
nix develop -c cargo fmt --all
git add crates/store-testing
git commit -m "feat(store-testing): lifecycle, atomic-append, and snapshot conformance checks (#281)"
```

---

### Task 7: Macro entry points

**Files:**
- Modify: `crates/store-testing/src/lib.rs` (add the macros + crate docs)
- Modify: `crates/store-testing/tests/self_check.rs` (replace direct-call tests with macro invocations)

- [ ] **Step 7.1: Add the hidden per-case helper + four public macros to lib.rs**

```rust
/// One generated conformance test: skip-guard, factory, check call.
#[doc(hidden)]
#[macro_export]
macro_rules! __conformance_case {
    ($module:ident, $check:ident, $factory:expr, $skip:expr) => {
        #[tokio::test]
        async fn $check() {
            if !($skip)() {
                return;
            }
            $crate::$module::$check(&$factory).await;
        }
    };
    (multi_thread: $module:ident, $check:ident, $factory:expr, $skip:expr) => {
        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn $check() {
            if !($skip)() {
                return;
            }
            $crate::$module::$check(&$factory).await;
        }
    };
}

/// Run the full core conformance matrix (sequence + boundary +
/// linearizability) against a store factory.
///
/// The factory returns `(store, guard)`: the guard keeps any backing resource
/// (a temp dir, a pool) alive for the check's duration — use `()` when the
/// store owns everything.
///
/// ```ignore
/// mnesis_store_testing::conformance! {
///     factory: || async { (InMemoryStore::new(), ()) },
/// }
/// ```
///
/// Requires `tokio` (with `macros`, `rt-multi-thread`) as a dev-dependency of
/// the invoking crate.
#[macro_export]
macro_rules! conformance {
    (factory: $factory:expr $(,)?) => {
        $crate::conformance! { factory: $factory, skip_unless: || true }
    };
    (factory: $factory:expr, skip_unless: $skip:expr $(,)?) => {
        mod conformance_sequence {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            $crate::__conformance_case!(sequence, check_empty_read_yields_none, $factory, $skip);
            $crate::__conformance_case!(sequence, check_append_then_read_round_trips, $factory, $skip);
            $crate::__conformance_case!(sequence, check_versions_strictly_monotonic_and_fused, $factory, $skip);
            $crate::__conformance_case!(sequence, check_large_stream_completes, $factory, $skip);
            $crate::__conformance_case!(sequence, check_read_stream_from_is_inclusive, $factory, $skip);
            $crate::__conformance_case!(sequence, check_append_conflict_is_surfaced, $factory, $skip);
            $crate::__conformance_case!(sequence, check_append_retry_after_conflict_succeeds, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_empty_store_yields_none, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_global_order_across_streams, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_from_is_exclusive, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_multi_resume_cycles, $factory, $skip);
            $crate::__conformance_case!(sequence, check_all_boundary_then_new_append, $factory, $skip);
            $crate::__conformance_case!(sequence, check_read_stream_inclusive_read_all_exclusive_coexist, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_backlog_then_caught_up_then_live, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_resume_strict_after, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_all_backlog_then_caught_up_then_live, $factory, $skip);
            $crate::__conformance_case!(sequence, check_subscription_large_backlog_crosses_chunk_seam, $factory, $skip);
        }

        mod conformance_boundary {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            $crate::__conformance_case!(boundary, check_conflict_leaves_store_unchanged, $factory, $skip);
            $crate::__conformance_case!(boundary, check_version_gap_batch_rejected, $factory, $skip);
            $crate::__conformance_case!(boundary, check_wrong_first_version_rejected, $factory, $skip);
            $crate::__conformance_case!(boundary, check_metadata_absent_vs_empty_distinct, $factory, $skip);
            $crate::__conformance_case!(boundary, check_max_length_event_type_round_trips, $factory, $skip);
            $crate::__conformance_case!(boundary, check_prefix_stream_ids_isolated, $factory, $skip);
        }

        mod conformance_linearizability {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            $crate::__conformance_case!(multi_thread: linearizability, check_concurrent_same_stream_single_winner, $factory, $skip);
            $crate::__conformance_case!(multi_thread: linearizability, check_concurrent_distinct_streams_all_land, $factory, $skip);
            $crate::__conformance_case!(multi_thread: linearizability, check_wake_after_idle, $factory, $skip);
            $crate::__conformance_case!(multi_thread: linearizability, check_caught_up_boundary_race, $factory, $skip);
        }
    };
}

/// Run the `AtomicAppend` capability conformance (feature `atomic-append`).
#[cfg(feature = "atomic-append")]
#[macro_export]
macro_rules! conformance_atomic_append {
    (factory: $factory:expr $(,)?) => {
        $crate::conformance_atomic_append! { factory: $factory, skip_unless: || true }
    };
    (factory: $factory:expr, skip_unless: $skip:expr $(,)?) => {
        mod conformance_atomic {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            $crate::__conformance_case!(atomic, check_atomic_multi_stream_commits_all, $factory, $skip);
            $crate::__conformance_case!(atomic, check_atomic_conflict_aborts_all, $factory, $skip);
            $crate::__conformance_case!(atomic, check_atomic_empty_batch_is_noop, $factory, $skip);
        }
    };
}

/// Run the `SnapshotStore` capability conformance (feature `snapshot`).
/// `positions` are two ascending sample positions of the store's `P`.
#[cfg(feature = "snapshot")]
#[macro_export]
macro_rules! conformance_snapshot {
    (factory: $factory:expr, positions: ($p1:expr, $p2:expr) $(,)?) => {
        $crate::conformance_snapshot! { factory: $factory, positions: ($p1, $p2), skip_unless: || true }
    };
    (factory: $factory:expr, positions: ($p1:expr, $p2:expr), skip_unless: $skip:expr $(,)?) => {
        mod conformance_snapshot {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            #[tokio::test]
            async fn check_snapshot_absent_then_commit_then_found() {
                if !($skip)() { return; }
                $crate::snapshot::check_snapshot_absent_then_commit_then_found(&$factory, $p1, $p2).await;
            }
            #[tokio::test]
            async fn check_snapshot_stale_on_schema_change() {
                if !($skip)() { return; }
                $crate::snapshot::check_snapshot_stale_on_schema_change(&$factory, $p1, $p2).await;
            }
            #[tokio::test]
            async fn check_snapshot_overwrite_latest_wins() {
                if !($skip)() { return; }
                $crate::snapshot::check_snapshot_overwrite_latest_wins(&$factory, $p1, $p2).await;
            }
        }
    };
}

/// Run the lifecycle conformance (persistent adapters only): `open` yields a
/// fresh `(store, ctx)`; `reopen` consumes both and reopens the SAME storage.
#[macro_export]
macro_rules! conformance_lifecycle {
    (open: $open:expr, reopen: $reopen:expr $(,)?) => {
        $crate::conformance_lifecycle! { open: $open, reopen: $reopen, skip_unless: || true }
    };
    (open: $open:expr, reopen: $reopen:expr, skip_unless: $skip:expr $(,)?) => {
        mod conformance_lifecycle {
            #[allow(clippy::wildcard_imports, reason = "re-import the invocation scope")]
            use super::*;

            #[tokio::test]
            async fn check_reopen_preserves_events() {
                if !($skip)() { return; }
                $crate::lifecycle::check_reopen_preserves_events(&$open, &$reopen).await;
            }
            #[tokio::test]
            async fn check_reopen_preserves_position_watermark() {
                if !($skip)() { return; }
                $crate::lifecycle::check_reopen_preserves_position_watermark(&$open, &$reopen).await;
            }
            #[tokio::test]
            async fn check_reopen_conflict_state_intact() {
                if !($skip)() { return; }
                $crate::lifecycle::check_reopen_conflict_state_intact(&$open, &$reopen).await;
            }
            #[tokio::test]
            async fn check_reopen_subscription_catches_up() {
                if !($skip)() { return; }
                $crate::lifecycle::check_reopen_subscription_catches_up(&$open, &$reopen).await;
            }
        }
    };
}
```

`#[cfg(feature)]` on `#[macro_export]` gates the macro's *availability* correctly (the macro only exists when the feature is on).

- [ ] **Step 7.2: Replace self-check direct calls with macro invocations**

`tests/self_check.rs` becomes (keep the lifecycle_plumbing and snapshot direct-call tests — they need custom closures/positions the core macro doesn't take... snapshot DOES have a macro; use it):

```rust
//! The kit's own red/green loop against `InMemoryStore`, driven through the
//! same macros adapters use — so the macros themselves are under test.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use mnesis::Version;
use mnesis_inmemory::{InMemorySnapshotStore, InMemoryStore};

mnesis_store_testing::conformance! {
    factory: || async { (InMemoryStore::new(), ()) },
}

mnesis_store_testing::conformance_atomic_append! {
    factory: || async { (InMemoryStore::new(), ()) },
}

mnesis_store_testing::conformance_snapshot! {
    factory: || async { (InMemorySnapshotStore::<Vec<u8>, Version>::new(), ()) },
    positions: (Version::new(5).unwrap(), Version::new(9).unwrap()),
}

// In-memory "reopen" hands back the same store — validates the kit's closure
// plumbing only; fjall/postgres prove real persistence.
mnesis_store_testing::conformance_lifecycle! {
    open: || async { (InMemoryStore::new(), ()) },
    reopen: |store: InMemoryStore, (): ()| async move { (store, ()) },
}
```

- [ ] **Step 7.3: Run**

Run: `nix develop -c cargo nextest run -p mnesis-store-testing`
Expected: ~30 individually-named tests, e.g. `self_check::conformance_sequence::check_append_conflict_is_surfaced`, all PASS.

- [ ] **Step 7.4: Format + commit**

```bash
nix develop -c cargo fmt --all
git add crates/store-testing
git commit -m "feat(store-testing): conformance! macro entry points (#281)"
```

---

### Task 8: Port `mnesis-inmemory`

**Files:**
- Modify: `adapters/inmemory/tests/inmemory_conformance.rs` (full rewrite)
- Modify: `adapters/inmemory/Cargo.toml` (dev-dep features if needed)

- [ ] **Step 8.1: Rewrite the conformance test file**

```rust
//! `InMemoryStore` conformance against the executable store contract —
//! every check delegated to the `mnesis-store-testing` kit (#281).

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use mnesis::Version;
use mnesis_inmemory::{InMemorySnapshotStore, InMemoryStore};

mnesis_store_testing::conformance! {
    factory: || async { (InMemoryStore::new(), ()) },
}

mnesis_store_testing::conformance_atomic_append! {
    factory: || async { (InMemoryStore::new(), ()) },
}

mnesis_store_testing::conformance_snapshot! {
    factory: || async { (InMemorySnapshotStore::<Vec<u8>, Version>::new(), ()) },
    positions: (Version::new(5).unwrap(), Version::new(9).unwrap()),
}
```

Update the dev-dep: `mnesis-store-testing = { path = "../../crates/store-testing", features = ["snapshot", "atomic-append"] }` — match the existing dev-dep form (path-only vs versioned) already in the file. If inmemory's `AtomicAppend` impl is behind its own `import` feature, the atomic macro invocation needs `#![cfg(feature = "import")]`-style gating consistent with how `inmemory_store_tests.rs` handles it AND a self dev-dep unifying the feature (check `rg "features" adapters/inmemory/Cargo.toml` first; mirror the existing pattern).

- [ ] **Step 8.2: Run**

Run: `nix develop -c cargo nextest run -p mnesis-inmemory`
Expected: the macro-generated matrix passes alongside the existing `inmemory_store_tests`. The old `inmemory_event_stream_conforms`/`inmemory_all_stream_conforms` tests are gone (superseded by the matrix).

- [ ] **Step 8.3: Format + commit**

```bash
nix develop -c cargo fmt --all
git add adapters/inmemory
git commit -m "refactor(inmemory): consume the conformance kit matrix (#281)"
```

---

### Task 9: Port `mnesis-fjall`

**Files:**
- Modify: `adapters/fjall/tests/fjall_conformance.rs` (full rewrite — the `OwnedFjallStream` wrapper dies)
- Modify: `adapters/fjall/Cargo.toml` (dev-dep features)

- [ ] **Step 9.1: Check fjall's builder/open call shape**

Run: `rg -n "FjallStore::builder|\.open\(\)" adapters/fjall/tests/fjall_conformance.rs adapters/fjall/tests/subscription_tests.rs | head -5`
Use exactly that call shape below (async vs sync `open`, error handling).

- [ ] **Step 9.2: Rewrite the conformance test file**

```rust
//! `FjallStore` conformance against the executable store contract (#281).
//! The factory hands the kit `(store, TempDir)` — the guard keeps the on-disk
//! directory alive for the check's duration.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use mnesis_fjall::FjallStore;
use tempfile::TempDir;

async fn open_fresh() -> (FjallStore, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FjallStore::builder(dir.path()).open().expect("open fjall");
    (store, dir)
}

mnesis_store_testing::conformance! {
    factory: open_fresh,
}

mnesis_store_testing::conformance_lifecycle! {
    open: open_fresh,
    reopen: |store: FjallStore, dir: TempDir| async move {
        drop(store); // release the keyspace lock before reopening the same path
        let reopened = FjallStore::builder(dir.path()).open().expect("reopen fjall");
        (reopened, dir)
    },
}
```

Add `conformance_atomic_append!`/`conformance_snapshot!` invocations gated exactly like fjall's existing feature-gated test files (`rg -l "cfg(feature" adapters/fjall/tests` and mirror: fjall's `AtomicAppend` sits behind its `import` feature, `SnapshotStore<Vec<u8>, Version>` behind `snapshot`). If fjall's dev-deps don't already unify those features for its test targets, follow the workspace's self-dev-dep pattern (`mnesis-fjall = { path = ".", features = [...] }`) — check whether `snapshot_tests.rs`/`export_import_tests.rs` already established it and copy that mechanism. Snapshot positions: `(Version::new(5).unwrap(), Version::new(9).unwrap())`.

If `FjallStore::builder(...).open()` is async (`rg` from Step 9.1 tells you), add `.await`.

- [ ] **Step 9.3: Run**

Run: `nix develop -c cargo nextest run -p mnesis-fjall --test fjall_conformance`
Expected: full matrix + lifecycle PASS. Any failure here is a REAL FINDING (fjall diverging from the contract InMemoryStore satisfies) — report it, do not weaken the kit.

- [ ] **Step 9.4: Format + commit**

```bash
nix develop -c cargo fmt --all
git add adapters/fjall
git commit -m "refactor(fjall): consume the conformance kit matrix + lifecycle (#281)"
```

---

### Task 10: Port `mnesis-postgres`

**Files:**
- Modify: `adapters/postgres/tests/conformance_tests.rs` (rewrite onto the macros; keep the `setup` truncate discipline)
- Modify: `adapters/postgres/Cargo.toml` (dev-dep on store-testing if features change)

- [ ] **Step 10.1: Check what postgres implements**

Run: `rg -n "impl (AtomicAppend|SnapshotStore|WakeSource) for" adapters/postgres/src/*.rs`
Invoke only the capability macros postgres actually implements.

- [ ] **Step 10.2: Rewrite the conformance test file**

```rust
//! `PostgresStore` conformance against the executable store contract (#281).
//!
//! Skips (passes vacuously) when `DATABASE_URL` is unset — the nixosTest
//! supplies a real URL in CI and runs serially (`--test-threads=1`), so each
//! test's fresh-store factory TRUNCATEs for isolation.

#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::missing_panics_doc, reason = "tests")]

use mnesis_postgres::PostgresStore;

fn have_db() -> bool {
    std::env::var("DATABASE_URL").is_ok()
}

/// Fresh store over a truncated events table. Only called when `have_db()`.
async fn open_fresh() -> (PostgresStore, ()) {
    let url = std::env::var("DATABASE_URL").expect("guarded by skip_unless");
    let pool = sqlx::postgres::PgPoolOptions::new().connect(&url).await.expect("connect");
    let store = PostgresStore::from_pool(pool.clone()).await.expect("from_pool");
    sqlx::query("TRUNCATE events RESTART IDENTITY")
        .execute(&pool)
        .await
        .expect("truncate");
    (store, ())
}

mnesis_store_testing::conformance! {
    factory: open_fresh,
    skip_unless: have_db,
}

// "Reopen" = a brand-new pool + store over the same database (no truncate!).
mnesis_store_testing::conformance_lifecycle! {
    open: open_fresh,
    reopen: |store: PostgresStore, (): ()| async move {
        drop(store);
        let url = std::env::var("DATABASE_URL").expect("guarded by skip_unless");
        let pool = sqlx::postgres::PgPoolOptions::new().connect(&url).await.expect("reconnect");
        let reopened = PostgresStore::from_pool(pool).await.expect("from_pool");
        (reopened, ())
    },
    skip_unless: have_db,
}
```

Add `conformance_atomic_append!` (with `skip_unless: have_db`) if Step 10.1 shows the impl exists. Keep `all_noskip_tests.rs` and `subscription_tests.rs` untouched (PR2's dedupe decides their fate).

- [ ] **Step 10.3: Compile-check (no local DB)**

Run: `nix develop -c cargo nextest run -p mnesis-postgres`
Expected: all conformance tests PASS vacuously (skip). CI's nixosTest runs them for real.

- [ ] **Step 10.4: Format + commit**

```bash
nix develop -c cargo fmt --all
git add adapters/postgres
git commit -m "refactor(postgres): consume the conformance kit matrix + lifecycle (#281)"
```

---

### Task 11: Delete the legacy shim + final gate + PR

**Files:**
- Modify: `crates/store-testing/src/lib.rs` (delete `assert_event_stream_conformance`, `assert_all_stream_conformance`, and their now-unused private helpers; crate docs point at the macros)

- [ ] **Step 11.1: Delete the legacy suites**

Remove from lib.rs: `assert_event_stream_conformance`, `assert_all_stream_conformance`, and every private helper only they used (`drain`, `assert_remains_none`, `check_empty_stream_yields_none` … `check_envelope_accessors_consistent`, `all_append`, the module-local `drain_all`/`assert_strictly_increasing` duplicates). Confirm nothing references them:

Run: `rg -n "assert_event_stream_conformance|assert_all_stream_conformance" --type rust`
Expected: no hits outside lib.rs history.

Rewrite the crate-level doc comment: what the kit is (the executable store contract), the factory contract, a `conformance!` usage example, the capability macros, and the pinned ambiguities (read-visibility under concurrent append is unspecified; GlobalSeq monotonic-not-gapless; empty stream ids are a permitted adapter limitation). Keep it tight — the full "writing a store adapter" guide is PR3.

- [ ] **Step 11.2: Workspace-wide verification**

```bash
nix develop -c cargo hakari generate
nix develop -c cargo fmt --all
nix develop -c cargo clippy --workspace --all-features --all-targets
```

Expected: clippy clean (the flake gate is `--lib` only; the workspace must ALSO be clean under `--all-features --all-targets` — fix anything it finds, never `#[allow]` without reason).

- [ ] **Step 11.3: Commit (hook runs the full flake gate)**

```bash
git add -A
git commit -m "refactor(store-testing)!: retire legacy assert_* suites — the macro matrix is the entry point (#281)"
```

- [ ] **Step 11.4: Push + PR**

```bash
git push -u origin feat/281-conformance-kit
gh pr create --title "feat(store-testing): adapter conformance kit — executable store contract (PR1 of #281)" --body "$(cat <<'EOF'
## Summary
- Expands mnesis-store-testing into the 4-category conformance kit (#281 PR1 of 3): sequence/protocol, defensive boundary, linearizability, lifecycle + atomic-append & snapshot capability modules
- `conformance!` / `conformance_atomic_append!` / `conformance_snapshot!` / `conformance_lifecycle!` macros generate one named test per contract rule
- All three adapters (inmemory, fjall, postgres) now run the matrix; legacy assert_* suites retired
- Kit self-checks against InMemoryStore via its own test target

Refs #281. PR2 dedupes adapter-local tests; PR3 adds the adapter-guide rustdoc + toy-adapter acceptance.

## Test plan
- [ ] nix flake check (pre-commit hook, every commit)
- [ ] cargo nextest run -p mnesis-store-testing / -p mnesis-inmemory / -p mnesis-fjall
- [ ] postgres matrix runs in the nixosTest CI attribute
- [ ] cargo clippy --workspace --all-features --all-targets clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Deviation log

Record every divergence from this plan here (what, why, impact) as work proceeds — per the repo's multi-PR refactor discipline. Clippy-driven micro-fixes are NOT deviations; contract findings (an adapter failing a check, an API mismatch against the pinned facts) ARE.

| # | Task | Deviation | Why | Impact |
|---|------|-----------|-----|--------|
| 1 | 1 | `mnesis-inmemory` dev-dep features are `["export","import"]`, no `snapshot` | inmemory has no `snapshot` feature — `InMemorySnapshotStore` is unconditional | none |
| 2 | 1 | self-check factory is `async fn` + `#[allow(clippy::unused_async)]`, not the plan's `impl Future` fn | plan's form trips deny-level `clippy::manual_async_fn` | none; shape matches later adapter factories |
| 3 | 2 | `envelope_for` uses `SchemaVersion::from_u32` (one step) | Task 1 quality review: don't re-derive validation the value type provides | none |
| 4 | 2 | conflict checks match `AppendError` with a wildcard non-Conflict arm | `AppendError` gained `#[non_exhaustive]` after the plan's facts were pinned (freeze rule: error enums) | none; same panic behavior |
| 5 | 3 | subscription checks add `S::Stream: Unpin` / `S::AllStream: Unpin` where-bounds not spelled out in the plan snippet | `Subscription::subscribe`/`subscribe_all` require these bounds on the real signature (verified in `crates/store/src/subscription.rs`) — the plan's check signatures omitted them | none; required for compilation, satisfied by `InMemoryStore` |
| 6 | 4 | **CONTRACT FINDING**: `check_metadata_absent_vs_empty_distinct` becomes `check_metadata_absent_vs_present_distinct` | `Some(empty)` metadata is unrepresentable by construction (`Metadata::from_bytes` rejects empty with `ValueError::MetadataEmpty`; wire reserves `u32::MAX` as absent sentinel) — the spec's absent-vs-empty premise was false; the type layer already prevents the confusion | check asserts the real adapter contract: `None` reads back `None`, present metadata (incl. the 1-byte minimum) byte-faithful; design spec boundary section updated |
| 7 | 5 | single-winner check's non-Conflict arm is a wildcard; `Arc<S>` call sites use `.as_ref()` | `AppendError` non_exhaustive (as #4); `Arc<S>` is not itself `RawEventStore` (only `Store<S>` is) | none |
| 8 | 5 | **PLAN BUG FIX**: `check_caught_up_boundary_race` loop waits on `versions.len() < total \|\| caught_up == 0` (plan's version waited on event count alone) | when the writer outpaces catch-up, all events arrive as backlog and `CaughtUp` lands after them — the plan's loop exited without consuming it (deterministic 5/5 repro); production loop verified correct against `subscription_cursor.rs` | check is strictly stronger: genuinely awaits the boundary marker |
| 9 | 6 | `atomic.rs` imports `AtomicAppend`/`AtomicAppendError`/`PlannedAppend` from `mnesis_store::import::*`, not the crate root | these three are not re-exported at `mnesis_store::` root (only `AbortReason`/`Atomicity`/`EventImporter`/`ImportBlock`/`ImportError`/`ImportReport`/`StreamOutcome`/`StreamReport`/`StreamSection` are) — verified against `crates/store/src/lib.rs` | none; same types, correct import path |
| 10 | 6 | `check_atomic_conflict_aborts_all`'s error match uses a wildcard non-Conflict arm | `AtomicAppendError` is `#[non_exhaustive]` (same freeze rule as #4/#7) | none |
| 11 | 6 | `lifecycle.rs` renames post-reopen bindings (`opened`/`reopened` instead of reusing `store`) and the plan's inline `use` block (in `check_reopen_subscription_catches_up`) moved to the file top | strict `clippy::shadow_reuse`/`clippy::shadow_unrelated` (deny) reject rebinding `store`/`ctx` across the reopen call; "no inline use" is a standing repo rule the plan's own NOTE flagged | none; same logic, different local names |
| 12 | 6 | `check_reopen_subscription_catches_up`'s catch-up loop is `while let Step::Event(env) = ...` instead of the plan's `loop { match .. { Event => .., CaughtUp => break } }` | `clippy::while_let_loop` (deny, part of `all`) flags the loop-with-only-a-match shape; `sequence.rs`'s equivalent `$all` backlog check already uses this same pattern | none; identical control flow |
| 13 | 6 | `snapshot.rs`'s three `Hydrated` matches keep the plan's `other => panic!` wildcard arm even though `Hydrated` is NOT `#[non_exhaustive]` (confirmed by reading `state.rs`) | consistency with `atomic.rs`'s mandatory wildcard is harmless; the plan's own text already used wildcards here | none; exhaustiveness was never relied on |
| 14 | 8 | inmemory adds a path-only self dev-dep `mnesis-inmemory = { path = ".", features = ["import"] }` | `AtomicAppend for InMemoryStore` is `#[cfg(feature = "import")]` and no existing dev-dep unified it into test targets (flake nextest = default features); `export` deliberately omitted (unused by the kit macros) | atomic macro compiles in CI; established mnesis-store self-dev-dep pattern |
| 15 | final review | binary (non-UTF-8) stream id `[0x00, 0xff, 0x42]` added to `check_prefix_stream_ids_isolated`; postgres file gained a comment stating atomic/snapshot macros are absent by design | whole-branch review found the spec's binary-id bullet uncovered and unlogged | spec §boundary fully covered; passes on all three adapters |
```
