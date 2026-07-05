# Typed Subscription View — Implementation Plan (#249)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a consumer-side typed layer over the raw subscription / `read_stream` / `read_all`
streams that reuses the configured codec — `.decoded(codec)` (owning codecs → a stream of
carry-away `Decoded<E>` items) and `.for_each_decoded(codec, f)` (owning **and** zero-copy → an
internal-iteration fold that hands the borrowed window to a closure) — with **no** hand-rolled
`from_slice`/`access`.

**Architecture:** One new module `crates/nexus-store/src/decoded.rs`. A `Decoded<T>` box generic over
the payload (owned `E` on the stream path, borrowed window `Decode::Output<'a>` in the fold). A
sealed `RawItem` trait unifies the two raw item shapes (`PersistedEnvelope` and
`(AllPosition, PersistedEnvelope)`) so **one** method covers per-stream and `$all`, preserving the
`$all` position tag beside the box. `futures::Stream` stays non-lending: owning codecs get a real
stream; zero-copy gets internal iteration (no lending stream, no `yoke`). Read / decode / handler
errors are distinct enum variants.

**Tech Stack:** Rust 2024, GATs (stable 1.65), RPITIT + AFIT (stable 1.75), HRTB equality bound on a
GAT (`for<'a> C: Decode<E, Output<'a> = E>` — the exact spelling `repository.rs` already uses),
`futures::StreamExt`, `thiserror`.

**Design doc:** `docs/plans/2026-07-03-typed-subscription-view-design.md`.

---

## Locked facts from the codebase (verified before writing this plan)

- `Decode<E: ?Sized>`: `type Output<'a>`, `type Error: std::error::Error + Send + Sync + 'static`,
  `fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<Self::Output<'a>, Self::Error>`.
  Owning serde: `Output<'a> = E`, `decode = self.format.deserialize(env.payload())` (ignores
  `event_type`). (`crates/nexus-store/src/codec.rs`.)
- `PersistedEnvelope::version(&self) -> Version` (line 416), `payload(&self) -> &[u8]`,
  `metadata_bytes(&self) -> Option<Bytes>`, `event_type(&self) -> &str`.
- `RawEventStore::AllPosition: AllPosition`; `pub trait AllPosition: Copy + Ord + Send + Sync +
  Debug + 'static` → **`Copy`**, so the tuple `retag` copies the tag (no `Clone`).
- `Version`: `INITIAL`, `new(u64) -> Option`, `next() -> Option`, `as_u64()`.
- `InMemoryStore` (feature `testing`) impls `RawEventStore` **and** `WakeSource` → usable for
  subscription + concurrency tests. `InMemoryStore::new()`, wrapped by `Store::new(..)`.
- `JsonCodec::default()` (= `SerdeCodec<Json>`, feature `json`); `Encode::encode(&self, &E) ->
  Result<Bytes, _>`.
- `pending_envelope(Version).event_type(&'static str).payload(impl Into<Bytes>).build() ->
  Result<PendingEnvelope, EnvelopeError>`; `store.append(&StreamKey::from_slice(id), expected,
  &[env])`.
- `Subscription::new(&store).subscribe(&id, from) -> Result<impl Stream<Item = Result<
  PersistedEnvelope, RawErr>>, WakeErr>`; `.subscribe_all(from) -> Result<impl Stream<Item =
  Result<(AllPosition, PersistedEnvelope), RawErr>>, WakeErr>`.
- `futures` is a normal dep **and** dev-dep; `futures::StreamExt` is already used in
  `subscription.rs`. Re-exported `Stream` is `futures_core::Stream`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/nexus-store/src/decoded.rs` | **New.** `Decoded<T>`; `DecodeStreamError`, `FoldDecodedError`; sealed `RawItem` + 2 impls; `DecodedStreamExt` (`.decoded`, `.for_each_decoded`). |
| `crates/nexus-store/src/lib.rs` | `pub mod decoded;` + `pub use` the public items. |
| `crates/nexus-store/tests/decoded_view_tests.rs` | **New.** The 4 mandatory categories. |
| `examples/fjall-end-to-end/src/lib.rs` | Delete `fold_balance`; consume via `.decoded()`. |
| `CLAUDE.md` | Document `decoded.rs` in the store-crate map. |

---

## Task 0: Confirm branch

- [ ] **Step 1: Verify the feature branch is checked out**

Run: `git branch --show-current`
Expected: `feat/249-typed-subscription-view` (already created; the design doc commit `23fdc87` is on it).

---

## Task 1: `Decoded<T>`, error enums, and the sealed `RawItem` trait

**Files:**
- Create: `crates/nexus-store/src/decoded.rs`
- Modify: `crates/nexus-store/src/lib.rs`
- Test: unit tests inline in `decoded.rs`

- [ ] **Step 1: Create `decoded.rs` with the types (no adapter methods yet)**

```rust
//! Consumer-side typed view over the raw envelope streams (#249).
//!
//! [`Subscription`](crate::Subscription), [`read_stream`](crate::RawEventStore::read_stream)
//! and [`read_all`](crate::RawEventStore::read_all) yield **raw**
//! [`PersistedEnvelope`]s by design (the distributed multi-consumer contract:
//! each consumer holds its own codec). This module adds an ergonomic layer on
//! top — it does **not** make the core subscription typed.
//!
//! - [`DecodedStreamExt::decoded`] — for **owning** codecs (JSON, bincode): a
//!   stream of carry-away [`Decoded<E>`] items.
//! - [`DecodedStreamExt::for_each_decoded`] — for **owning and zero-copy**
//!   codecs (rkyv, bytemuck): an internal-iteration fold that hands the borrowed
//!   window to a closure, so no lending stream is needed.

use bytes::Bytes;
use futures::{Stream, StreamExt};

use crate::codec::Decode;
use crate::envelope::PersistedEnvelope;
use nexus::Version;

/// A raw envelope, un-packed: the decoded event plus its resume bookmark and
/// metadata. `T` is the owned event (`E`) on the stream path, or the borrowed
/// window ([`Decode::Output`]) inside a fold closure.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded<T> {
    /// The decoded event (owned `E`, or a borrowed window).
    pub event: T,
    /// The per-stream version — the per-stream resume bookmark.
    pub version: Version,
    /// The event's metadata, if any (cheap Arc-shared bytes).
    pub metadata: Option<Bytes>,
}

/// A read from the raw stream failed, or the codec failed to decode the payload.
/// The two failure domains never share a variant (CLAUDE rule 3).
#[derive(Debug, thiserror::Error)]
pub enum DecodeStreamError<R, D> {
    /// The underlying raw stream yielded an error item.
    #[error("subscription stream read failed")]
    Read(#[source] R),
    /// The codec rejected the payload.
    #[error("event decode failed")]
    Decode(#[source] D),
}

/// [`DecodedStreamExt::for_each_decoded`] failure: a raw read, a decode, or the
/// consumer's own fold closure. Three distinct domains (CLAUDE rule 3).
#[derive(Debug, thiserror::Error)]
pub enum FoldDecodedError<R, D, H> {
    /// The underlying raw stream yielded an error item.
    #[error("subscription stream read failed")]
    Read(#[source] R),
    /// The codec rejected the payload.
    #[error("event decode failed")]
    Decode(#[source] D),
    /// The consumer's fold closure returned an error.
    #[error("decoded-event handler failed")]
    Handler(#[source] H),
}

mod sealed {
    pub trait Sealed {}
}

/// A raw stream item that carries a [`PersistedEnvelope`] and can be re-tagged
/// with its decoded counterpart.
///
/// Implemented for the two raw shapes the store yields:
/// - `PersistedEnvelope` (per-stream / `read_stream`) → typed item `Decoded<T>`;
///   the bookmark (`version`) lives **inside** the box.
/// - `(P, PersistedEnvelope)` (`$all` / `read_all`) → typed item
///   `(P, Decoded<T>)`; the `$all` position bookmark stays **beside** the box.
///
/// The asymmetry is intentional (CLAUDE rule 4): each bookmark rides exactly
/// where it lives in the raw layer. Sealed — not implementable downstream.
pub trait RawItem: sealed::Sealed {
    /// The typed item once the envelope is decoded to `Decoded<T>`.
    type Typed<T>;
    /// Borrow the carried envelope (to decode it).
    fn envelope(&self) -> &PersistedEnvelope;
    /// Re-attach the decoded box into this item's shape (copying any tag).
    fn retag<T>(&self, decoded: Decoded<T>) -> Self::Typed<T>;
}

impl sealed::Sealed for PersistedEnvelope {}
impl RawItem for PersistedEnvelope {
    type Typed<T> = Decoded<T>;
    fn envelope(&self) -> &PersistedEnvelope {
        self
    }
    fn retag<T>(&self, decoded: Decoded<T>) -> Decoded<T> {
        decoded
    }
}

impl<P: Copy> sealed::Sealed for (P, PersistedEnvelope) {}
impl<P: Copy> RawItem for (P, PersistedEnvelope) {
    type Typed<T> = (P, Decoded<T>);
    fn envelope(&self) -> &PersistedEnvelope {
        &self.1
    }
    fn retag<T>(&self, decoded: Decoded<T>) -> (P, Decoded<T>) {
        (self.0, decoded)
    }
}
```

- [ ] **Step 2: Add the module + re-exports to `lib.rs`**

In `crates/nexus-store/src/lib.rs`, add the module declaration next to the other `pub mod`s
(alphabetical block around line 92–124):

```rust
pub mod decoded;
```

And in the `pub use` block (around line 126+), add:

```rust
pub use decoded::{Decoded, DecodeStreamError, DecodedStreamExt, FoldDecodedError, RawItem};
```

(`DecodedStreamExt` does not exist yet — it is added in Task 2. To keep this task compiling on its
own, **temporarily** list only `pub use decoded::{Decoded, DecodeStreamError, FoldDecodedError,
RawItem};` and add `DecodedStreamExt` in Task 2, Step 4.)

- [ ] **Step 3: Add inline unit tests at the bottom of `decoded.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending_envelope;

    fn env(version: u64, meta: Option<&[u8]>) -> PersistedEnvelope {
        let mut b = pending_envelope(Version::new(version).unwrap())
            .event_type("E")
            .payload(b"payload".to_vec());
        if let Some(m) = meta {
            b = b.metadata(m.to_vec());
        }
        b.build().unwrap().into_persisted_for_test()
    }

    #[test]
    fn retag_on_bare_envelope_is_identity_shape() {
        let e = env(3, Some(b"m"));
        let decoded = Decoded {
            event: 42u64,
            version: e.version(),
            metadata: e.metadata_bytes(),
        };
        let typed: Decoded<u64> = e.retag(decoded);
        assert_eq!(typed.event, 42);
        assert_eq!(typed.version, Version::new(3).unwrap());
        assert_eq!(typed.metadata.as_deref(), Some(b"m".as_ref()));
    }

    #[test]
    fn retag_on_tagged_item_copies_the_position_beside_the_box() {
        let e = env(1, None);
        let item = (99u64, e);
        let decoded = Decoded {
            event: 7u64,
            version: item.envelope().version(),
            metadata: None,
        };
        let (pos, typed): (u64, Decoded<u64>) = item.retag(decoded);
        assert_eq!(pos, 99);
        assert_eq!(typed.event, 7);
        assert_eq!(typed.version, Version::new(1).unwrap());
    }

    #[test]
    fn error_variants_render_distinct_messages() {
        #[derive(Debug, thiserror::Error)]
        #[error("boom")]
        struct Boom;
        let read: DecodeStreamError<Boom, Boom> = DecodeStreamError::Read(Boom);
        let decode: DecodeStreamError<Boom, Boom> = DecodeStreamError::Decode(Boom);
        assert_eq!(read.to_string(), "subscription stream read failed");
        assert_eq!(decode.to_string(), "event decode failed");
    }
}
```

> **Note on `into_persisted_for_test`:** the unit test needs a `PersistedEnvelope` from a
> `PendingEnvelope`. If no such test constructor exists, replace the `env` helper body with the
> store round-trip used in the integration tests (append to `InMemoryStore`, `read_stream`, take the
> first item) — see Task 2 Step 1 for that exact pattern — and gate this `mod tests` with
> `#![cfg(feature = "testing")]`. Prefer whichever a 30-second `grep 'fn into_persisted\|for_decode'
> crates/nexus-store/src/envelope.rs` shows already exists; `PersistedEnvelope::for_decode(name,
> payload)` exists per CLAUDE.md but does not carry a version, so it is unsuitable here.

- [ ] **Step 4: Verify it compiles and the unit tests pass**

Run: `nix develop -c cargo test -p nexus-store --features testing,json decoded::tests -- --nocapture`
Expected: 3 tests pass. (If `into_persisted_for_test` is unavailable, use the round-trip helper per
the note above.)

- [ ] **Step 5: Commit**

```bash
git add crates/nexus-store/src/decoded.rs crates/nexus-store/src/lib.rs
git commit -m "feat(store): Decoded<T> box + RawItem seam for typed subscription view (#249)"
```

---

## Task 2: `.decoded(codec)` — the owning stream adapter

**Files:**
- Modify: `crates/nexus-store/src/decoded.rs`
- Create: `crates/nexus-store/tests/decoded_view_tests.rs`

- [ ] **Step 1: Write the failing integration test (sequence: catch-up then live)**

Create `crates/nexus-store/tests/decoded_view_tests.rs`:

```rust
#![cfg(all(feature = "testing", feature = "json"))]
#![allow(clippy::unwrap_used, reason = "tests")]
#![allow(clippy::expect_used, reason = "tests")]
#![allow(clippy::panic, reason = "tests")]

use std::time::Duration;

use futures::StreamExt;
use nexus::{DomainEvent, Message, Version};
use nexus_store::store::RawEventStore;
use nexus_store::testing::InMemoryStore;
use nexus_store::{
    Decode, DecodeStreamError, DecodedStreamExt, Encode, JsonCodec, PersistedEnvelope, Store,
    StreamKey, Subscription, pending_envelope,
};
use serde::{Deserialize, Serialize};

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Money {
    Deposited { amount: u64 },
    Withdrew { amount: u64 },
}
impl Message for Money {}
impl DomainEvent for Money {
    fn name(&self) -> &'static str {
        match self {
            Self::Deposited { .. } => "Deposited",
            Self::Withdrew { .. } => "Withdrew",
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct AcctId(String);
impl std::fmt::Display for AcctId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl AsRef<[u8]> for AcctId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

fn money_envelope(version: u64, event: &Money) -> nexus_store::PendingEnvelope {
    let bytes = JsonCodec::default().encode(event).unwrap();
    pending_envelope(Version::new(version).unwrap())
        .event_type(event.name())
        .payload(bytes)
        .build()
        .expect("valid envelope")
}

async fn append(store: &Store<InMemoryStore>, id: &AcctId, version: u64, ev: &Money) {
    let expected = Version::new(version - 1);
    store
        .append(&StreamKey::from_slice(id.as_ref()), expected, &[money_envelope(version, ev)])
        .await
        .unwrap();
}

// ═══ 1. Sequence/Protocol ═══
#[tokio::test]
async fn decoded_catchup_then_live_reuses_the_codec() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("acct-1".to_owned());

    append(&store, &id, 1, &Money::Deposited { amount: 1000 }).await;
    append(&store, &id, 2, &Money::Withdrew { amount: 400 }).await;

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    // Catch-up: exactly the two seeded events, decoded, with their versions.
    let d1 = tokio::time::timeout(TIMEOUT, stream.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(d1.event, Money::Deposited { amount: 1000 });
    assert_eq!(d1.version, Version::new(1).unwrap());
    let d2 = tokio::time::timeout(TIMEOUT, stream.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(d2.event, Money::Withdrew { amount: 400 });
    assert_eq!(d2.version, Version::new(2).unwrap());

    // Live: append a third; the parked cursor must decode it too.
    append(&store, &id, 3, &Money::Deposited { amount: 250 }).await;
    let d3 = tokio::time::timeout(TIMEOUT, stream.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(d3.event, Money::Deposited { amount: 250 });
    assert_eq!(d3.version, Version::new(3).unwrap());
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `nix develop -c cargo test -p nexus-store --features testing,json --test decoded_view_tests decoded_catchup_then_live_reuses_the_codec`
Expected: FAIL — `no method named 'decoded' found` (`DecodedStreamExt` not written yet).

- [ ] **Step 3: Implement `DecodedStreamExt::decoded`**

Append to `crates/nexus-store/src/decoded.rs`:

```rust
/// Extension methods that add a typed, codec-reusing view over any stream of
/// raw envelope items ([`Subscription`](crate::Subscription),
/// [`read_stream`](crate::RawEventStore::read_stream),
/// [`read_all`](crate::RawEventStore::read_all)).
pub trait DecodedStreamExt<I, R>: Stream<Item = Result<I, R>> + Sized
where
    I: RawItem,
{
    /// Decode each item with `codec`, reusing the codec configured elsewhere.
    ///
    /// Owning codecs only — the `for<'a> Output<'a> = E` bound is unsatisfiable
    /// for a zero-copy codec (whose `Output` borrows the envelope), so the
    /// compiler steers zero-copy consumers to [`for_each_decoded`](Self::for_each_decoded).
    /// Per-stream items become `Decoded<E>`; `$all` items become
    /// `(AllPosition, Decoded<E>)` (the tag is preserved beside the box).
    fn decoded<E, C>(
        self,
        codec: C,
    ) -> impl Stream<Item = Result<I::Typed<E>, DecodeStreamError<R, C::Error>>> + Send
    where
        C: Decode<E> + Send + Sync + 'static,
        for<'a> C: Decode<E, Output<'a> = E>,
        E: Send + 'static,
        I: Send + 'static,
        R: Send + 'static,
        Self: Send,
    {
        self.map(move |res| {
            let item = res.map_err(DecodeStreamError::Read)?;
            let event: E = codec.decode(item.envelope()).map_err(DecodeStreamError::Decode)?;
            let env = item.envelope();
            let decoded = Decoded {
                event,
                version: env.version(),
                metadata: env.metadata_bytes(),
            };
            Ok(item.retag(decoded))
        })
    }
}

impl<St, I, R> DecodedStreamExt<I, R> for St
where
    St: Stream<Item = Result<I, R>>,
    I: RawItem,
{
}
```

- [ ] **Step 4: Add `DecodedStreamExt` to the `lib.rs` re-export**

Update the Task 1 re-export line to the final form:

```rust
pub use decoded::{Decoded, DecodeStreamError, DecodedStreamExt, FoldDecodedError, RawItem};
```

- [ ] **Step 5: Run the sequence test to verify it passes**

Run: `nix develop -c cargo test -p nexus-store --features testing,json --test decoded_view_tests decoded_catchup_then_live_reuses_the_codec`
Expected: PASS.

- [ ] **Step 6: Add the boundary + `$all` tests**

Append to `crates/nexus-store/tests/decoded_view_tests.rs`:

```rust
// ═══ 3. Defensive Boundary ═══
#[tokio::test]
async fn corrupt_payload_surfaces_decode_not_panic_not_read() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("acct-bad".to_owned());
    // Append a raw envelope whose payload is NOT valid JSON for `Money`.
    let bad = pending_envelope(Version::INITIAL)
        .event_type("Deposited")
        .payload(b"not json".to_vec())
        .build()
        .unwrap();
    store
        .append(&StreamKey::from_slice(id.as_ref()), None, &[bad])
        .await
        .unwrap();

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    let item = tokio::time::timeout(TIMEOUT, stream.next()).await.unwrap().unwrap();
    assert!(matches!(item, Err(DecodeStreamError::Decode(_))), "got {item:?}");
}

#[tokio::test]
async fn read_error_item_surfaces_read_variant() {
    // Craft a raw stream with a read error to exercise the Read arm against the
    // real adapter method (SUT = `.decoded`), no store needed.
    #[derive(Debug, thiserror::Error)]
    #[error("adapter boom")]
    struct Boom;

    let good = money_envelope(1, &Money::Deposited { amount: 5 });
    let good = {
        // Round-trip through the store to obtain a PersistedEnvelope item.
        let store = Store::new(InMemoryStore::new());
        let id = AcctId("x".to_owned());
        store
            .append(&StreamKey::from_slice(id.as_ref()), None, &[good])
            .await
            .unwrap();
        store
            .read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL)
            .next()
            .await
            .unwrap()
            .unwrap()
    };

    let raw = futures::stream::iter(vec![
        Ok::<PersistedEnvelope, Boom>(good),
        Err(Boom),
    ]);
    let typed = raw.decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(typed);

    let first = typed.next().await.unwrap();
    assert!(matches!(first, Ok(_)), "got {first:?}");
    let second = typed.next().await.unwrap();
    assert!(matches!(second, Err(DecodeStreamError::Read(Boom))), "got {second:?}");
}

#[tokio::test]
async fn decoded_all_preserves_the_position_tag_beside_the_box() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("acct-all".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;

    let stream = Subscription::new(&store)
        .subscribe_all(None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    let (_pos, d) = tokio::time::timeout(TIMEOUT, stream.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(d.event, Money::Deposited { amount: 1 });
    assert_eq!(d.version, Version::new(1).unwrap());
}
```

> `read_stream` returns the adapter's concrete `Stream`; `.next()` needs `futures::StreamExt`
> (already imported) and the stream to be `Unpin` or pinned. `InMemoryStore`'s stream is `Unpin`; if
> the compiler complains, wrap with `let mut s = std::pin::pin!(store.read_stream(...));`.

- [ ] **Step 7: Run all `.decoded` tests**

Run: `nix develop -c cargo test -p nexus-store --features testing,json --test decoded_view_tests`
Expected: all passing (4 tests so far).

- [ ] **Step 8: Commit**

```bash
git add crates/nexus-store/src/decoded.rs crates/nexus-store/src/lib.rs crates/nexus-store/tests/decoded_view_tests.rs
git commit -m "feat(store): .decoded(codec) owning typed subscription stream (#249)"
```

---

## Task 3: `.for_each_decoded(codec, f)` — the borrowing fold (serves zero-copy)

**Files:**
- Modify: `crates/nexus-store/src/decoded.rs`
- Modify: `crates/nexus-store/tests/decoded_view_tests.rs`

- [ ] **Step 1: Write the failing tests (owning fold, borrowing/zero-copy fold, handler error)**

Append to `crates/nexus-store/tests/decoded_view_tests.rs`:

```rust
use nexus_store::{Decoded, FoldDecodedError};

// ═══ for_each_decoded: owning codec folds typed state ═══
#[tokio::test]
async fn for_each_decoded_folds_owning_events() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("fe-1".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1000 }).await;
    append(&store, &id, 2, &Money::Withdrew { amount: 400 }).await;

    // Bounded: read the finite history via read_stream (terminates), not the
    // never-ending subscription.
    let raw = std::pin::pin!(store.read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL));
    let mut balance: i64 = 0;
    let mut last = Version::INITIAL;
    raw.for_each_decoded::<Money, _, _, std::convert::Infallible>(JsonCodec::default(), |d: Decoded<Money>| {
        match d.event {
            Money::Deposited { amount } => balance += i64::try_from(amount).unwrap(),
            Money::Withdrew { amount } => balance -= i64::try_from(amount).unwrap(),
        }
        last = d.version;
        Ok(())
    })
    .await
    .unwrap();

    assert_eq!(balance, 600);
    assert_eq!(last, Version::new(2).unwrap());
}

// ═══ for_each_decoded: BORROWING codec (zero-copy path, no rkyv feature) ═══
// A codec whose Output borrows the envelope — the KERI rkyv shape, proven with
// a dependency-free stand-in.
struct RawBytesCodec;
impl Decode<[u8]> for RawBytesCodec {
    type Output<'a> = &'a [u8];
    type Error = std::convert::Infallible;
    fn decode<'a>(&'a self, env: &'a PersistedEnvelope) -> Result<&'a [u8], Self::Error> {
        Ok(env.payload())
    }
}

#[tokio::test]
async fn for_each_decoded_folds_borrowed_windows() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("fe-zc".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1000 }).await;

    let raw = std::pin::pin!(store.read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL));
    let mut seen_len = 0usize;
    raw.for_each_decoded::<[u8], _, _, std::convert::Infallible>(RawBytesCodec, |d: Decoded<&[u8]>| {
        // `d.event` is a window borrowing the envelope — zero copy.
        seen_len = d.event.len();
        assert_eq!(d.version, Version::new(1).unwrap());
        Ok(())
    })
    .await
    .unwrap();

    assert!(seen_len > 0);
}

// ═══ Handler error maps to the Handler variant (not Decode, not Read) ═══
#[tokio::test]
async fn for_each_decoded_surfaces_handler_error() {
    #[derive(Debug, thiserror::Error)]
    #[error("stop")]
    struct Stop;

    let store = Store::new(InMemoryStore::new());
    let id = AcctId("fe-h".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;

    let raw = std::pin::pin!(store.read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL));
    let out = raw
        .for_each_decoded::<Money, _, _, Stop>(JsonCodec::default(), |_d: Decoded<Money>| Err(Stop))
        .await;
    assert!(matches!(out, Err(FoldDecodedError::Handler(Stop))), "got {out:?}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test -p nexus-store --features testing,json --test decoded_view_tests for_each_decoded`
Expected: FAIL — `no method named 'for_each_decoded'`.

- [ ] **Step 3: Implement `for_each_decoded` as a provided method on `DecodedStreamExt`**

Inside the `trait DecodedStreamExt` block in `decoded.rs`, add:

```rust
    /// Fold each decoded event by handing your closure the borrowed window —
    /// works for **owning and zero-copy** codecs, because the window lives only
    /// for the call and never escapes (internal iteration; no lending stream).
    ///
    /// `f` receives the typed item (`Decoded<Output<'a>>`, or
    /// `(AllPosition, Decoded<Output<'a>>)` for `$all`) valid only for that
    /// call. On a never-ending [`Subscription`](crate::Subscription) this runs
    /// until the first `Err`; over a finite
    /// [`read_stream`](crate::RawEventStore::read_stream) it runs to completion.
    fn for_each_decoded<E, C, F, H>(
        self,
        codec: C,
        mut f: F,
    ) -> impl Future<Output = Result<(), FoldDecodedError<R, C::Error, H>>>
    where
        E: ?Sized,
        C: Decode<E>,
        F: for<'a> FnMut(I::Typed<<C as Decode<E>>::Output<'a>>) -> Result<(), H>,
    {
        async move {
            let stream = self;
            futures::pin_mut!(stream);
            while let Some(res) = stream.next().await {
                let item = res.map_err(FoldDecodedError::Read)?;
                let env = item.envelope();
                let window = codec.decode(env).map_err(FoldDecodedError::Decode)?;
                let decoded = Decoded {
                    event: window,
                    version: env.version(),
                    metadata: env.metadata_bytes(),
                };
                f(item.retag(decoded)).map_err(FoldDecodedError::Handler)?;
            }
            Ok(())
        }
    }
```

Add the `Future` import at the top of `decoded.rs`:

```rust
use core::future::Future;
```

> **Borrow note (why this compiles):** `env` borrows `item` (shared); `window` borrows `env`;
> `decoded` owns `window` (still a shared borrow of `item`); `item.retag(&self, decoded)` takes
> `&self` (another shared borrow) and copies the `Copy` position. All borrows of `item` are shared
> and end when the closure returns. This is the exact discipline the design doc §6.3 describes.

- [ ] **Step 4: Run the `for_each_decoded` tests**

Run: `nix develop -c cargo test -p nexus-store --features testing,json --test decoded_view_tests for_each_decoded`
Expected: 3 tests pass (owning fold, borrowed-window fold, handler error).

- [ ] **Step 5: Commit**

```bash
git add crates/nexus-store/src/decoded.rs crates/nexus-store/tests/decoded_view_tests.rs
git commit -m "feat(store): .for_each_decoded borrowing fold — zero-copy typed view (#249)"
```

---

## Task 4: Lifecycle + linearizability tests

**Files:**
- Modify: `crates/nexus-store/tests/decoded_view_tests.rs`

- [ ] **Step 1: Add the lifecycle test (write → resume from checkpoint → decode)**

Append:

```rust
// ═══ 2. Lifecycle ═══
#[tokio::test]
async fn decoded_resume_from_checkpoint_decodes_the_tail() {
    let store = Store::new(InMemoryStore::new());
    let id = AcctId("life-1".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;
    append(&store, &id, 2, &Money::Deposited { amount: 2 }).await;
    append(&store, &id, 3, &Money::Deposited { amount: 3 }).await;

    // Resume strictly after v2 → the typed view must begin at v3.
    let stream = Subscription::new(&store)
        .subscribe(&id, Version::new(2))
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    let d = tokio::time::timeout(TIMEOUT, stream.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(d.version, Version::new(3).unwrap());
    assert_eq!(d.event, Money::Deposited { amount: 3 });
}
```

- [ ] **Step 2: Add the linearizability test (concurrent writer + decoded fold, real overlap)**

Append:

```rust
// ═══ 4. Linearizability/Isolation ═══
#[tokio::test]
async fn decoded_observes_concurrent_writes_in_order_no_dup() {
    use std::sync::Arc;
    use tokio::sync::Barrier;

    let store = Store::new(InMemoryStore::new());
    let id = AcctId("lin-1".to_owned());
    append(&store, &id, 1, &Money::Deposited { amount: 1 }).await;

    let stream = Subscription::new(&store)
        .subscribe(&id, None)
        .unwrap()
        .decoded::<Money, _>(JsonCodec::default());
    tokio::pin!(stream);

    // Drain v1, then rendezvous with a writer that appends v2..=v6 concurrently.
    let first = tokio::time::timeout(TIMEOUT, stream.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(first.version, Version::new(1).unwrap());

    let barrier = Arc::new(Barrier::new(2));
    let wb = Arc::clone(&barrier);
    let ws = store.clone();
    let wid = id.clone();
    let writer = tokio::spawn(async move {
        wb.wait().await;
        for v in 2..=6u64 {
            append(&ws, &wid, v, &Money::Deposited { amount: v }).await;
        }
    });

    barrier.wait().await;
    let mut versions = Vec::new();
    for _ in 0..5 {
        let d = tokio::time::timeout(TIMEOUT, stream.next()).await.unwrap().unwrap().unwrap();
        versions.push(d.version.as_u64());
    }
    writer.await.unwrap();

    // Strictly monotonic, no duplicates, exact set.
    assert_eq!(versions, vec![2, 3, 4, 5, 6]);
}
```

- [ ] **Step 3: Run the full test file**

Run: `nix develop -c cargo test -p nexus-store --features testing,json --test decoded_view_tests`
Expected: all tests pass (9 total).

- [ ] **Step 4: Commit**

```bash
git add crates/nexus-store/tests/decoded_view_tests.rs
git commit -m "test(store): lifecycle + linearizability for typed subscription view (#249)"
```

---

## Task 5: Migrate the #227 example and update docs (acceptance)

**Files:**
- Modify: `examples/fjall-end-to-end/src/lib.rs`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Read the current consumption sites**

Run: `sed -n '120,135p;198,246p' examples/fjall-end-to-end/src/lib.rs`
Expected: see `fn fold_balance(balance, env) { serde_json::from_slice(env.payload()) … }` and the
three `fold_balance(balance, &env)?` call sites plus the `subscribe(...)` loops.

- [ ] **Step 2: Replace the hand-decode with `.decoded()`**

Delete the `fold_balance` fn (lines ~125–130). Add `use nexus_store::{DecodedStreamExt, Decoded};`
to the imports (top of file, alongside the existing `nexus_store::{…}` line — no mid-file `use`, per
project rule). Change the catch-up loop from folding raw envelopes to folding decoded events. The
existing loop:

```rust
let stream = subscription.subscribe(&id, None)?;
tokio::pin!(stream);
// … balance = fold_balance(balance, &env)?; env.version().as_u64();
```

becomes:

```rust
let stream = subscription
    .subscribe(&id, None)?
    .decoded::<AccountEvent, _>(codec.clone());
tokio::pin!(stream);
// for each: let d = stream.next().await…?; d.version.as_u64(); apply d.event to balance
```

Fold `d.event` into `balance` inline where `fold_balance` was called (the example's `AccountEvent`
arithmetic moves into the loop body / a small `apply(balance, &AccountEvent) -> u64` pure helper if
the same fold is needed at the live and resume sites — DRY). Use `d.version` where
`env.version()` was used. The `codec` is the same `JsonCodec` the repository was built with (clone
it, or construct a fresh `JsonCodec::default()` — codecs are stateless).

> **Exact edits depend on the current example body.** Read it (Step 1) and preserve behavior: the
> `SubscriptionOutcome` fields (`catchup_versions`, `catchup_balance`, `live_version`,
> `resumed_first_version`) must be computed identically, now via `d.version` / `d.event` instead of
> `env.version()` / `fold_balance`.

- [ ] **Step 3: Verify the example still builds and its tests pass**

Run: `nix develop -c cargo test -p fjall-end-to-end`
Expected: PASS, and `grep -n fold_balance examples/fjall-end-to-end/src/lib.rs` returns nothing.

- [ ] **Step 4: Document `decoded.rs` in `CLAUDE.md`**

In the "Store Crate (`nexus-store`)" module list, add a bullet after the `stream.rs` entry:

```markdown
- **`decoded.rs`** — Consumer-side typed view over the raw subscription / `read_stream` / `read_all`
  streams (#249). Keeps the deliberate raw multi-consumer contract; adds an ergonomic layer that
  **reuses the configured codec** instead of a hand-rolled `from_slice`. `Decoded<T>` is one box
  (event + `version` + `metadata`) generic over the payload — owned `E` on the stream path, the
  borrowed window `Decode::Output<'a>` inside a fold. `DecodedStreamExt` adds two methods over any
  `Stream<Item = Result<I, R>>` where `I: RawItem` (sealed; impl'd for `PersistedEnvelope` →
  `Decoded<E>` and `(AllPosition, PersistedEnvelope)` → `(AllPosition, Decoded<E>)`, tag preserved):
  `.decoded(codec)` — **owning codecs only** (`for<'a> Output<'a> = E`, so a zero-copy codec cannot
  satisfy it and the compiler steers to the fold) → a stream of carry-away `Decoded<E>`; and
  `.for_each_decoded(codec, f)` — **owning and zero-copy** (rkyv/bytemuck) → internal-iteration fold
  handing the borrowed window to a closure, so no lending stream is reintroduced (the ~1370-line GAT
  layer stays deleted). Error domains stay distinct (rule 3): `DecodeStreamError` (`Read`/`Decode`),
  `FoldDecodedError` (`Read`/`Decode`/`Handler`). Not subscription-gated — also sugars finite reads.
```

- [ ] **Step 5: Commit**

```bash
git add examples/fjall-end-to-end/src/lib.rs CLAUDE.md
git commit -m "feat(store): consume #227 example via .decoded — delete hand-rolled fold_balance (#249)"
```

- [ ] **Step 6: Open the PR**

```bash
git push -u origin feat/249-typed-subscription-view
gh pr create --title "feat(store): typed subscription view — reuse the configured codec (#249)" \
  --body "Closes #249. Consumer-side typed layer over the raw subscription/read streams; keeps the raw contract. \`.decoded(codec)\` (owning) + \`.for_each_decoded(codec, f)\` (owning + zero-copy). See docs/plans/2026-07-03-typed-subscription-view-design.md."
```

---

## Self-Review

**1. Spec coverage:**
- Design §3 decision 1–2 (event + bookmark, labeled box) → `Decoded<T>` (Task 1). ✓
- §3 decision 3 (snap-on adapter, `subscribe` untouched) → `DecodedStreamExt` blanket impl (Task 2). ✓
- §3 decision 4 / §5 (two entry points; owning stream + borrowing fold) → Task 2 + Task 3. ✓
- §6.1 `Decoded<T>` generic over payload → one struct, used with `E` (Task 2) and `&[u8]` window (Task 3). ✓
- §6.2 owning-only `for<'a> Output<'a> = E` bound → Task 2 Step 3. ✓
- §6.3 borrowing fold, window never escapes → Task 3 Step 3 (`for<'a> FnMut(...Output<'a>...)`). ✓
- §6.4 per-stream vs `$all`, tag preserved → `RawItem` + `decoded_all` test (Task 2 Step 6). ✓
- §6.5 distinct error domains → `DecodeStreamError` / `FoldDecodedError` (Task 1). ✓
- §7 four mandatory test categories → sequence (T2), boundary (T2), lifecycle + linearizability (T4), plus for_each (T3). ✓
- §8 acceptance (delete `fold_balance`, raw stays, zero-copy expressible) → Task 5 + the borrowed-window test (T3). ✓

**2. Placeholder scan:** No "TBD/TODO". The two `> Note` callouts (Task 1 Step 3 test constructor;
Task 5 Step 2 exact edits) are **explicit fallback instructions with the exact grep/read to run**,
not deferred work — they exist because the precise line numbers depend on the current file, and both
give the concrete alternative. Acceptable.

**3. Type consistency:** `Decoded<T>` fields (`event`/`version`/`metadata`) identical across Tasks
1–5. `RawItem::{Typed,envelope,retag}` names match every call site. `decoded::<Money, _>` /
`for_each_decoded::<Money, _, _, H>` turbofish arity matches the method generics (`<E, C>` and
`<E, C, F, H>`). Error variants (`Read`/`Decode`/`Handler`) match between definition and `matches!`
assertions. ✓

**Known risks flagged for execution (not placeholders):**
- `async fn`/RPITIT `+ Send`: if the `.decoded` return fails a `+ Send` bound in a real spawn
  context, drop `+ Send` from the RPITIT (consumers pin+await inline, as the example does) or add
  the missing `Send` bound the compiler names. The tests await inline, so they pass either way.
- Task 1 unit-test envelope constructor (`into_persisted_for_test`) may not exist — the note gives
  the store-round-trip fallback used everywhere else in the suite.
