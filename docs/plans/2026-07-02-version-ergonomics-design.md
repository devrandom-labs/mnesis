# `version!` literal + checked version runs

**Issue:** #253 — construct/advance `Version` with no `.unwrap()` on literals and no hand-rolled `base + i + 1` math.
**Status:** design — research settled, build two small kernel additions.
**Date:** 2026-07-02

---

## The two problems

Every example pays one of these:

```rust
// 1. Literal: a known-nonzero constant, forced through Option → unwrap.
Version::new(3).unwrap()
Version::new(3).ok_or("v3 is nonzero")?

// 2. Sequence: hand-assigning contiguous versions to a batch.
let ver = Version::new(base + u64::try_from(i).unwrap() + 1).unwrap();   // per event, in a loop
let new_version = Version::new(base + u64::try_from(decided.len()).unwrap()).unwrap();
```

Bucket 1 is everywhere (tests, asserts, `commit_persisted` calls). Bucket 2 lives only on the **manual-drive path** — examples calling `commit_persisted`/`replay` by hand for no-store event sourcing. The real `save`/`append` path already stamps contiguous versions internally (`nexus-store`'s `first_persisted_version` + `next()`), so a normal app never writes bucket-2 math. That scopes bucket 2 as a genuine but **narrow** API gap, not a universal tax.

## Fix 1 — `version!(N)`: a compile-checked literal

A declarative macro that evaluates at compile time and rejects `0` as a **compile error**, zero runtime cost even in debug. This is settled prior art — `nonzero_lit`, `nonzero_ext::nonzero!` ([nonzero_lit](https://docs.rs/nonzero_lit/latest/nonzero_lit/)) — built on stable inline `const {}` blocks (RFC 2920; stable since 1.79, we pin 1.95).

```rust
/// A compile-checked `Version` literal. `version!(3)` is `Version` (not
/// `Option<Version>`), evaluated at compile time; `version!(0)` is a
/// *compile error*, not a runtime panic. Zero runtime cost.
#[macro_export]
macro_rules! version {
    ($n:expr) => {
        const {
            match $crate::Version::new($n) {
                ::core::option::Option::Some(v) => v,
                ::core::option::Option::None => ::core::panic!("version literal must be non-zero"),
            }
        }
    };
}
```

Why a `macro_rules!`, not a proc-macro or a const-generic `Version::lit::<3>()`:
- **No proc-macro** — it's a one-liner over the existing `const fn new`; `nexus-macros` stays untouched. `no_std`-clean, IoT-friendly.
- **A const-generic `lit::<N>()` doesn't force the check.** A `const fn` called in a *non-const* position runs at runtime, so `Version::lit::<0>()` would panic at runtime, not compile time. The `const {}` block is what guarantees compile-time rejection. The macro wraps that; a bare fn can't.
- **`match`, not `.unwrap()`** — avoids any dependency on `const Option::unwrap` stability and gives a clear message.

Call site: `version!(3)` replaces `Version::new(3).unwrap()` / `.ok_or(...)?` everywhere.

## Fix 2 — `Version::run`: a checked contiguous run

A length-checked iterator over consecutive versions. **Checked once, up front** — not a lazy infinite iterator, because a lazy one silently ends on overflow, and zipping it against a batch would drop events (a silent-truncation correctness bug, rule 2/3). Pre-validating the whole run makes the iterator infallible and the overflow explicit.

```rust
/// Iterator over `len` consecutive versions starting at `start`.
/// Named (not RPIT) so it is a nameable return type and can carry the
/// checked-run invariant. Infallible: the overflow check happened in `run`.
pub struct VersionRun { /* next: Option<Version>, remaining: usize */ }

impl Iterator for VersionRun {
    type Item = Version;
    fn next(&mut self) -> Option<Version> { /* yields start, start.next(), … len times */ }
}
impl ExactSizeIterator for VersionRun { /* remaining */ }

impl Version {
    /// `len` consecutive versions `start, start+1, …, start+len-1`.
    /// `None` if the run would overflow past `u64::MAX` (consistent with
    /// `new`/`next` returning `Option`). `len == 0` yields an empty run.
    #[must_use]
    pub const fn run(start: Version, len: usize) -> Option<VersionRun> { /* checked add up front */ }
}
```

The `base + i + 1` loop collapses to a `zip`:

```rust
// before
let base = account.version().map_or(0, |v| v.as_u64());
for (i, event) in decided.iter().enumerate() {
    let ver = Version::new(base + u64::try_from(i).unwrap() + 1).unwrap();
    stream.push(VersionedEvent::new(ver, event.clone()));
}
let new_version = Version::new(base + u64::try_from(decided.len()).unwrap()).unwrap();
account.commit_persisted(new_version, decided);

// after
let first = account.version().map_or(Version::INITIAL, |v| v.next().expect("version overflow"));
let run = Version::run(first, decided.len()).expect("version overflow");
let last = run.clone().last().unwrap_or(first);
for (ver, event) in run.zip(decided.iter()) {
    stream.push(VersionedEvent::new(ver, event.clone()));
}
account.commit_persisted(last, decided);
```

No cast, no `+ 1`, no per-event unwrap. The one remaining `first` line (INITIAL-or-`next`) is the honest "where does the run start" question, not arithmetic noise.

> Open refinement for implementation: if every manual site computes `first` the same way, add `Version::run_after(current: Option<Version>, len)` folding the INITIAL/`next` step in. Decide from the actual call sites — don't add it speculatively.

## Placement

Both land in `crates/nexus/src/version.rs` (kernel — `Version` is kernel). `version!` is `#[macro_export]` at the crate root; re-export/confirm it's reachable as `nexus::version!`. `VersionRun` + `Version::run` are inherent. No new module, no proc-macro, no `nexus-store` change.

## Scope — and where to stop

- `version!(N)` literal macro.
- `Version::run(start, len) -> Option<VersionRun>` + `VersionRun` iterator.
- Migrate the example/test sites that hand-roll the math: `store-and-kernel`, `inmemory`, `closing-the-books`, `projection-tokio`, `fjall-end-to-end`.
- **Not** in scope: touching the `save`/`append` internal stamping (already correct), or a `run_after` unless the call sites prove it earns its place.

## Tests (rule 7 first)

1. **Sequence/protocol:** `version!(1)`/`version!(u64::MAX)` equal their `Version::new(_).unwrap()` counterparts; `Version::run(v, 3)` yields exactly `[v, v+1, v+2]` in order; `run` then `commit_persisted` advances a root identically to the old loop.
2. **Boundary/defensive:** `Version::run(near_max, 2)` where the run overflows → `None` (no panic, no silent short run); `Version::run(v, 0)` → empty run; `version!` at `1` and `u64::MAX` (min/max literals).
3. **Compile-fail:** `version!(0)` must **fail to compile** — add a `trybuild` compile-fail case (the whole point is a compile error, so a runtime test can't prove it). Confirm `nexus` already has a `trybuild`/`compile_fail` harness; if not, this is the one place to add a minimal one.
4. **Equivalence:** an example migrated to `run`/`version!` produces byte-identical persisted state / versions to the pre-migration loop.

## What this does not do

- No proc-macro; `nexus-macros` untouched.
- No `Result` (stays `Option`, consistent with `new`/`next`).
- No silent-truncating lazy iterator — overflow is surfaced up front.
- No change to the store's internal version stamping.
