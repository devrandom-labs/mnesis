# Version Ergonomics (`version!` + `Version::run`) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans. Steps use `- [ ]` checkboxes.

**Goal:** Kill `Version::new(n).unwrap()` on literals and `base + i + 1` sequence math (#253) via a compile-checked `version!(n)` macro and a checked `Version::run(start, len)` iterator.

**Architecture:** Two additions to `crates/nexus/src/version.rs`: a `#[macro_export] macro_rules! version` built on a stable inline `const {}` block (compile-time nonzero check, zero runtime cost), and an inherent `Version::run(start, len) -> Option<VersionRun>` returning a named, length-checked `VersionRun: Iterator`. No proc-macro, no store change.

**Design doc:** `docs/plans/2026-07-02-version-ergonomics-design.md`

**Conventions:** run one test `nix develop -c cargo test -p nexus -- <name>`. Do NOT run `nix flake check` by hand — the pre-commit hook runs it (pass Bash `timeout: 600000` to `git commit`). `git add` new files before committing. All `use` at top. Strict clippy (pedantic/nursery denied). Toolchain is 1.95.0 (inline `const{}` and const `Option` are available).

---

## Task 1: `version!(N)` compile-checked literal macro

**Files:** Modify `crates/nexus/src/version.rs`; verify export in `crates/nexus/src/lib.rs`.

- [ ] **Step 1: Add the macro** near the top of `version.rs` (after the imports, before or after `struct Version` — a `#[macro_export]` macro is visible crate-wide regardless of position):

```rust
/// A compile-checked [`Version`] literal.
///
/// `version!(3)` evaluates to a `Version` (not `Option<Version>`) at compile
/// time. `version!(0)` is a **compile error**, not a runtime panic — the check
/// runs in an inline `const {}` block, so there is zero runtime cost even in
/// debug builds.
///
/// ```
/// use nexus::{version, Version};
/// assert_eq!(version!(3), Version::new(3).unwrap());
/// ```
#[macro_export]
macro_rules! version {
    ($n:expr) => {
        const {
            match $crate::Version::new($n) {
                ::core::option::Option::Some(v) => v,
                ::core::option::Option::None => {
                    ::core::panic!("version literal must be non-zero")
                }
            }
        }
    };
}
```

- [ ] **Step 2: Confirm it's reachable as `nexus::version!`.** `#[macro_export]` puts it at the crate root automatically. Check `crates/nexus/src/lib.rs` — if the crate uses `pub use` grouping for macros or has `#![no_std]` concerns, ensure nothing shadows it. Run: `nix develop -c grep -n "macro_export\|pub use.*version\|no_std" crates/nexus/src/lib.rs crates/nexus/src/version.rs`.

- [ ] **Step 3: Add unit tests** in `version.rs`'s test module:

```rust
#[test]
fn version_macro_matches_new_unwrap() {
    assert_eq!(version!(1), Version::new(1).unwrap());
    assert_eq!(version!(3), Version::new(3).unwrap());
    assert_eq!(version!(u64::MAX), Version::new(u64::MAX).unwrap());
}

#[test]
fn version_macro_equals_initial_at_one() {
    assert_eq!(version!(1), Version::INITIAL);
}
```
(Import the macro in the test module if needed — `use crate::version;` or rely on `#[macro_export]` crate-root visibility; adjust to what compiles.)

- [ ] **Step 4: Run tests.** `nix develop -c cargo test -p nexus -- version_macro` → expect PASS. `nix develop -c cargo clippy -p nexus --lib` → clean.

- [ ] **Step 5: Commit** (Bash timeout 600000):
```bash
git add crates/nexus/src/version.rs crates/nexus/src/lib.rs
git commit -m "feat(kernel): version! compile-checked Version literal (#253)"
```

---

## Task 2: `Version::run` + `VersionRun` checked iterator

**Files:** Modify `crates/nexus/src/version.rs`.

- [ ] **Step 1: Write the failing test first** in the test module:

```rust
#[test]
fn run_yields_exactly_len_consecutive_versions() {
    let run = Version::run(version!(5), 3).expect("no overflow");
    let got: Vec<u64> = run.map(Version::as_u64).collect();
    assert_eq!(got, vec![5, 6, 7]);
}

#[test]
fn run_len_zero_is_empty() {
    let run = Version::run(version!(1), 0).expect("no overflow");
    assert_eq!(run.count(), 0);
}

#[test]
fn run_overflow_past_max_is_none() {
    // start at MAX, asking for 2 versions overflows.
    let max = Version::new(u64::MAX).unwrap();
    assert!(Version::run(max, 2).is_none());
    // exactly 1 at MAX is fine (just the start).
    assert_eq!(Version::run(max, 1).map(|r| r.count()), Some(1));
}

#[test]
fn run_is_exact_size() {
    let run = Version::run(version!(1), 4).unwrap();
    assert_eq!(run.len(), 4);
}
```

- [ ] **Step 2: Run to confirm it fails.** `nix develop -c cargo test -p nexus -- run_` → FAIL (`Version::run` not found).

- [ ] **Step 3: Implement `VersionRun` + `Version::run`:**

```rust
/// Iterator over a checked run of consecutive [`Version`]s.
///
/// Constructed by [`Version::run`], which validates up front that the whole
/// run fits — so iteration is infallible and never silently truncates on
/// overflow (a lazy infinite iterator zipped against a batch could drop
/// items; this cannot).
#[derive(Debug, Clone)]
pub struct VersionRun {
    next: Version,
    remaining: usize,
}

impl Iterator for VersionRun {
    type Item = Version;

    fn next(&mut self) -> Option<Version> {
        if self.remaining == 0 {
            return None;
        }
        let current = self.next;
        self.remaining -= 1;
        // Only advance when more remain; on the last item `next()` might
        // overflow past MAX, but we never use that successor (run was
        // length-checked at construction), so guard it.
        if self.remaining > 0 {
            // SAFETY of unwrap avoided: run() guaranteed `remaining-1` more
            // successors exist, so `next()` is Some here.
            if let Some(n) = self.next.next() {
                self.next = n;
            }
        }
        Some(current)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for VersionRun {}
impl core::iter::FusedIterator for VersionRun {}

impl Version {
    /// `len` consecutive versions `start, start+1, …, start+len-1`.
    ///
    /// Returns `None` if the run would overflow past `u64::MAX` (consistent
    /// with [`Version::new`]/[`Version::next`] returning `Option`). `len == 0`
    /// yields an empty run.
    #[must_use]
    pub fn run(start: Version, len: usize) -> Option<VersionRun> {
        if len == 0 {
            return Some(VersionRun { next: start, remaining: 0 });
        }
        // Last version is start + (len - 1). Check it fits.
        let last_offset = u64::try_from(len - 1).ok()?;
        let last_raw = start.as_u64().checked_add(last_offset)?;
        // last_raw >= start.as_u64() >= 1, so it's a valid Version.
        Version::new(last_raw)?;
        Some(VersionRun { next: start, remaining: len })
    }
}
```

> Note: `run` is not `const fn` because `u64::try_from`/`checked_add` chains and struct construction are fine in a normal fn, and no caller needs it const. If a const need appears, revisit — don't force it now (YAGNI).

- [ ] **Step 4: Run tests.** `nix develop -c cargo test -p nexus -- run_` → PASS (4 tests). `nix develop -c cargo clippy -p nexus --lib` → clean. Fix any nursery lint (e.g. `checked_add` is already rule-2-correct; no bare arithmetic — note `len - 1` is guarded by the `len == 0` early return above).

- [ ] **Step 5: Commit** (Bash timeout 600000):
```bash
git add crates/nexus/src/version.rs
git commit -m "feat(kernel): Version::run — checked contiguous version iterator (#253)"
```

---

## Task 3: Compile-fail test — `version!(0)` must not compile

**Files:** Create/extend a `trybuild` compile-fail harness for `nexus`.

- [ ] **Step 1: Check for an existing harness.** Run: `nix develop -c bash -c 'ls crates/nexus/tests/ 2>/dev/null; grep -rn "trybuild" crates/nexus/Cargo.toml'`. `nexus-store` has `tests/compile_fail_tests.rs` + `tests/compile_fail/` — mirror that pattern. If `trybuild` isn't a `nexus` dev-dep, add it: `nix develop -c cargo add --dev trybuild -p nexus` (never hand-write the version).

- [ ] **Step 2: Add the compile-fail case.** Create `crates/nexus/tests/compile_fail/version_zero.rs`:
```rust
fn main() {
    let _ = nexus::version!(0);
}
```
And the harness `crates/nexus/tests/compile_fail_tests.rs` (or extend the existing one):
```rust
#[test]
fn compile_fail_cases() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/version_zero.rs");
}
```

- [ ] **Step 3: Generate the expected stderr.** Run: `nix develop -c cargo test -p nexus --test compile_fail_tests` once; trybuild writes a `.stderr` on first run (or run with `TRYBUILD=overwrite`). Confirm the error mentions the non-zero panic / const-eval failure. Commit the `.stderr` alongside.

- [ ] **Step 4: Verify it actually fails to compile** (the test PASSES because compilation FAILED as expected). Re-run: `nix develop -c cargo test -p nexus --test compile_fail_tests` → PASS.

- [ ] **Step 5: Commit** (Bash timeout 600000):
```bash
git add crates/nexus/tests/compile_fail/ crates/nexus/tests/compile_fail_tests.rs crates/nexus/Cargo.toml Cargo.lock
git commit -m "test(kernel): version!(0) is a compile error (#253)"
```

---

## Task 4: Migrate the examples/tests off the hand-rolled math

**Files:** `examples/inmemory/src/main.rs`, `examples/store-and-kernel/src/main.rs`, `examples/closing-the-books/src/main.rs`, `examples/projection-tokio/tests/loop_tests.rs`, `examples/fjall-end-to-end/src/lib.rs`.

- [ ] **Step 1: Replace literal unwraps.** Across the listed files, replace `Version::new(N).unwrap()` and `Version::new(N).ok_or(...)?` (for **literal** `N`) with `version!(N)`. Add `version` (and keep `Version`) to each file's `nexus` import. Do NOT touch `Version::new(x)` where `x` is a runtime variable — that stays fallible.

- [ ] **Step 2: Replace the sequence loops.** In `inmemory`, `store-and-kernel`, `closing-the-books`, and `projection-tokio` (the `base + u64::try_from(i).unwrap() + 1` sites), rewrite using `Version::run`:
```rust
let first = account.version().map_or(Version::INITIAL, |v| v.next().expect("version overflow"));
let run = Version::run(first, decided.len()).expect("version overflow");
let last = run.clone().last().unwrap_or(first);
for (ver, event) in run.zip(decided.iter()) {
    stream.push(VersionedEvent::new(ver, event.clone()));
}
account.commit_persisted(last, decided);
```
Adapt to each site's exact locals (some compute `base` as `u64`, some track `new_version` separately — the `run`/`last` pair replaces both). These are examples, so `.expect(...)` on the provably-unreachable overflow is acceptable (matches existing example style), but prefer surfacing over silent defaults.

- [ ] **Step 3: Build + test each crate.** For each: `nix develop -c cargo test -p <crate>` (foreground, timeout 600000). Expect PASS — behavior is identical. Confirm `examples/inmemory`, `store-and-kernel`, `closing-the-books`, `projection-tokio`, `fjall-end-to-end` all green.

- [ ] **Step 4: Clippy the examples.** `nix develop -c cargo clippy --all-targets` (the examples aren't covered by the `--lib` flake gate). Fix any lint in non-test code.

- [ ] **Step 5: Commit** (Bash timeout 600000):
```bash
git add examples/
git commit -m "refactor(examples): use version! and Version::run, delete manual version math (#253)"
```

---

## Task 5: Final review + PR

- [ ] **Step 1: Whole-branch review** — dispatch a reviewer over `git diff origin/main..HEAD`: check the macro is genuinely compile-time (const block), `run`'s overflow is checked not silent, `VersionRun::next` never panics, examples produce identical versions, no `Version::new(literal).unwrap()` survives (grep: `nix develop -c grep -rn "Version::new([0-9]" examples/ crates/` should be empty for literals).
- [ ] **Step 2: Open PR** (`joeldsouzax` account) titled `feat(kernel): version! literal + Version::run (#253)`, body summarizing both additions + the compile-fail proof, `Closes #253`. Squash-merge after green.

---

## Self-Review Notes (author checklist — done)
- **Coverage:** `version!` macro (T1) ✓, compile-fail proof (T3) ✓, `Version::run`/`VersionRun` checked (T2) ✓, examples migrated (T4) ✓, both buckets from the design addressed ✓.
- **Type consistency:** `version!`, `Version::run(start, len) -> Option<VersionRun>`, `VersionRun: Iterator + ExactSizeIterator + FusedIterator` used consistently T1-T4.
- **Rule adherence:** `run` uses `checked_add`/`try_from` (rule 2, no bare arithmetic); `len - 1` guarded by the `len == 0` early return; overflow → `None` not silent truncation (rule 3); `Option` not `Result` (API consistency); `#[must_use]` on `run`.
- **Watch-items:** (1) confirm `nexus` has/needs a `trybuild` dev-dep (T3 S1); (2) `VersionRun::next` last-item overflow guard — the `if self.remaining > 0` prevents calling `.next()` past the checked range; (3) macro visibility as `nexus::version!` (T1 S2).
