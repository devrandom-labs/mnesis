# Conformance Kit PR2 Implementation Plan (#281) — adapter-test dedupe

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Delete adapter-local tests the conformance kit now covers; promote the genuine kit gaps to kit checks FIRST, then delete the local copies. Adapter-specific tests stay.

**Architecture:** Two audits (2026-07-11) classified all 141 adapter-local tests as A (delete — kit covers at equal/greater strength), B (promote to kit, then delete local), or C (keep — adapter-specific). The controller corrected the audits per the repo's testing rules: proptest/state-machine/DST tests are NEVER A (they explore input space fixed vectors can't); white-box inline tests at non-default configs are C; checks the kit cannot express generically (opaque position ceilings, snapshot/stream independence) are C.

**Branch:** `feat/281-conformance-kit-pr2` (created off origin/main at `b432f15`).

**Rules recap:** every commit passes the pre-commit `nix flake check` (never bypass); `nix develop -c cargo fmt --all` before staging; deletions are `git rm`/edits with NO behavioral rewrites of surviving tests; imports pruned after deletions (deny-level unused lints); deviation log at the bottom of this file.

---

## Task 1: New kit checks (the B-promotions)

**Files:**
- Modify: `crates/store-testing/src/sequence.rs` (2 new checks)
- Modify: `crates/store-testing/src/boundary.rs` (1 new check)
- Modify: `crates/store-testing/src/snapshot.rs` (2 new checks + positions-contract doc)
- Modify: `crates/store-testing/src/lib.rs` (wire the 5 into the macros)
- Modify: `crates/store-testing/tests/self_check.rs` + `adapters/inmemory/tests/inmemory_conformance.rs` + `adapters/fjall/tests/fjall_conformance.rs` (snapshot macro invocations gain the extremes pair)

- [ ] **Step 1.1: `sequence.rs` — subscription edge checks**

```rust
/// Subscribing to a stream that does not exist yet parks (after `CaughtUp`)
/// and is woken by the stream's FIRST event — the producer-after-consumer
/// startup order must work.
pub async fn check_subscription_absent_stream_waits_then_delivers<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("ghost");

    let sub = Subscription::new(&store);
    let stream = sub.subscribe(&id, None).unwrap_or_else(|e| panic!("register failed: {e:?}"));
    pin_mut!(stream);

    // An absent stream has an empty backlog: CaughtUp arrives first.
    match next_step(&mut stream, "absent-stream boundary").await {
        Step::CaughtUp => {}
        Step::Event(env) => panic!("absent stream must have no backlog, got v{}", env.version()),
    }

    // The FIRST event ever written to the stream wakes the parked subscriber.
    append_event(&store, &id.key(), 1, b"first").await;
    match next_step(&mut stream, "absent-stream first event").await {
        Step::Event(env) => {
            assert_eq!(env.version().as_u64(), 1, "the first event must be v1");
            assert_eq!(env.payload(), b"first");
        }
        Step::CaughtUp => panic!("CaughtUp must be emitted exactly once"),
    }
}

/// Two simultaneous subscribers on ONE stream each receive the full event
/// sequence — subscriptions are fan-out, never competing-consumer queues.
pub async fn check_two_subscribers_same_stream_both_receive<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    S::Stream: Unpin,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (raw, _guard) = factory().await;
    let store = raw.into_store();
    let id = SubId::new("fanout");
    append_event(&store, &id.key(), 1, b"p1").await;

    let sub = Subscription::new(&store);
    let stream_a = sub.subscribe(&id, None).unwrap_or_else(|e| panic!("register a failed: {e:?}"));
    let stream_b = sub.subscribe(&id, None).unwrap_or_else(|e| panic!("register b failed: {e:?}"));
    pin_mut!(stream_a);
    pin_mut!(stream_b);

    // Both drain the backlog and reach CaughtUp independently.
    for (stream, name) in [(&mut stream_a, "a"), (&mut stream_b, "b")] {
        match next_step(stream, "fanout backlog").await {
            Step::Event(env) => assert_eq!(env.version().as_u64(), 1, "subscriber {name} backlog"),
            Step::CaughtUp => panic!("subscriber {name}: CaughtUp before backlog"),
        }
        match next_step(stream, "fanout boundary").await {
            Step::CaughtUp => {}
            Step::Event(env) => panic!("subscriber {name}: expected CaughtUp, got v{}", env.version()),
        }
    }

    // One live append reaches BOTH subscribers.
    append_event(&store, &id.key(), 2, b"p2").await;
    for (stream, name) in [(&mut stream_a, "a"), (&mut stream_b, "b")] {
        match next_step(stream, "fanout live").await {
            Step::Event(env) => assert_eq!(
                env.version().as_u64(),
                2,
                "subscriber {name} must receive the live event — fan-out, not a queue",
            ),
            Step::CaughtUp => panic!("subscriber {name}: CaughtUp must be emitted exactly once"),
        }
    }
}
```

NOTE: `for (stream, name) in [(&mut stream_a, ...)]` re-borrows a `Pin<&mut _>` — if the borrow checker objects, unroll the loop into a-then-b blocks (same assertions). `next_step`'s signature takes `&mut Pin<&mut St>`; match whatever shape landed in PR1.

- [ ] **Step 1.2: `boundary.rs` — large payload**

```rust
/// A 1 MiB payload round-trips byte-for-byte — well beyond any internal
/// buffer/batch size (the round-trip check tops out at 4 KiB).
pub async fn check_large_payload_round_trips<S, C, F, Fut>(factory: &F)
where
    S: RawEventStore + WakeSource,
    C: Send,
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = (S, C)> + Send,
{
    let (store, _guard) = factory().await;
    let id = StreamKey::from_slice(b"large-payload");
    let payload: Vec<u8> = (0..1_048_576u32)
        .map(|i| u8::try_from(i % 251).unwrap_or(0)) // prime modulus: no 256-aligned repeats
        .collect();
    append_rows(&store, &id, &[ConformanceRow::new(1, "E", payload.clone())]).await;
    let got = drain_stream(&store, &id, Version::INITIAL).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].payload, payload, "1 MiB payload must round-trip byte-for-byte");
}
```

- [ ] **Step 1.3: `snapshot.rs` — empty state + extreme positions**

Re-document the macro's `positions` contract: `(p1, p2)` MUST be `p1 < p2`; ADD a second pair `extremes: (pmin, pmax)` = the smallest representable position and a position at/near the type's ceiling (`pmin < pmax`). Two new checks:

```rust
/// Empty state (`vec![]`) commits and hydrates as `Found` with empty bytes —
/// an empty projection fold result is a legal snapshot.
pub async fn check_snapshot_empty_state_round_trips<SS, P, C, F, Fut>(factory: &F, p1: P, _p2: P)
where /* same bounds as the existing checks */
{
    let (store, _guard) = factory().await;
    let id = SubId::new("snap-empty");
    store.commit(&id, SCHEMA_1, p1, &vec![]).await.unwrap_or_else(|e| panic!("commit failed: {e:?}"));
    match store.hydrate(&id, SCHEMA_1).await.unwrap_or_else(|e| panic!("hydrate failed: {e:?}")) {
        Hydrated::Found { position, state } => {
            assert_eq!(position, p1);
            assert_eq!(state, Vec::<u8>::new(), "empty state must hydrate as Found(empty), not Absent");
        }
        other => panic!("expected Found(empty), got {other:?}"),
    }
}

/// Positions at the representable extremes (minimum and ceiling) encode and
/// decode exactly — the position codec has no off-by-one at either edge.
pub async fn check_snapshot_extreme_positions_round_trip<SS, P, C, F, Fut>(
    factory: &F,
    pmin: P,
    pmax: P,
) where /* same bounds */
{
    assert!(pmin < pmax, "kit misuse: extremes must be (min, max) with min < max");
    let (store, _guard) = factory().await;
    for (label, p) in [("min", pmin), ("max", pmax)] {
        let id = SubId::new(&format!("snap-extreme-{label}"));
        store.commit(&id, SCHEMA_1, p, &vec![1u8]).await.unwrap_or_else(|e| panic!("commit at {label} failed: {e:?}"));
        match store.hydrate(&id, SCHEMA_1).await.unwrap_or_else(|e| panic!("hydrate at {label} failed: {e:?}")) {
            Hydrated::Found { position, .. } => assert_eq!(position, p, "{label} position must round-trip exactly"),
            other => panic!("expected Found at {label}, got {other:?}"),
        }
    }
}
```

- [ ] **Step 1.4: wire into macros + invocations**

`conformance!` gains the two sequence checks (sequence module) and the boundary check. `conformance_snapshot!` grammar becomes:

```rust
conformance_snapshot! {
    factory: ...,
    positions: ($p1, $p2),
    extremes: ($pmin, $pmax),
    // optional skip_unless
}
```

generating the two new tests alongside the existing three. Update the three invocation sites:
- self_check + inmemory: `extremes: (Version::new(1).unwrap(), Version::new(u64::MAX).unwrap())`
- fjall: same `Version` pair.

- [ ] **Step 1.5: run + commit**

`nix develop -c cargo nextest run -p nexus-store-testing -p nexus-inmemory -p nexus-fjall` — everything green (a failure of a NEW check against any adapter is a real finding → BLOCKED, report, never weaken). fmt, `git add`, commit:
`feat(store-testing): promote adapter-test gaps into kit checks (#281)`

---

## Task 2: fjall deletions

**Files (all under `adapters/fjall/tests/`):**

- [x] **Step 2.1: whole-file deletions**
- `git rm metadata_roundtrip_tests.rs` (all 3 tests kit-covered: metadata distinct + reopen preservation)
- `git rm execute_lifecycle_tests.rs` (facade-through-fjall reopen; kit lifecycle + nexus-store facade tests cover both halves — READ the file first and confirm it holds nothing else; if it tests facade behavior not covered anywhere, reclassify to C and report)

- [x] **Step 2.2: `projection_tests.rs` — delete these fns** (kit snapshot module covers them; the GlobalSeq `P` instantiation adds nothing — the kit runs `SnapshotStore<_, Version>` and the value codec is P-generic):
`commit_then_hydrate_roundtrips`, `commit_overwrites_previous_projection`, `projection_persists_across_reopen`, `hydrate_unknown_id_is_absent`, `hydrate_schema_mismatch_is_stale_not_absent`, `different_projections_are_independent`, `commit_at_initial_global_seq_roundtrips`, `commit_at_max_global_seq_roundtrips`, `commit_empty_state_roundtrips` (last three now kit checks via Task 1).
KEEP: `snapshot_and_projection_with_same_id_do_not_collide` (fjall partition split — with a comment noting it's fjall-specific if one isn't there).

- [x] **Step 2.3: `snapshot_tests.rs` — delete:** `commit_then_hydrate_roundtrips`, `commit_overwrites_previous_snapshot`, `snapshot_persists_across_reopen`, `hydrate_unknown_id_returns_none`, `different_streams_have_separate_snapshots`.
KEEP: `hydrate_id_without_snapshot_returns_none`, `commit_without_event_stream_is_persisted` (fjall partition-independence properties — the SnapshotStore interface can't express them generically).

- [x] **Step 2.4: `subscription_tests.rs` — delete:** `subscribe_catchup_then_live`, `subscribe_from_checkpoint`, `drop_and_resubscribe`, `write_close_reopen_subscribe`, `subscribe_to_nonexistent_stream_waits` (now a kit check), `subscribe_from_beyond_head`, `append_during_catchup_no_loss`, `subscribe_all_sees_concurrent_appends_across_streams`, `multiple_subscribers_same_stream` (now a kit check), `subscribe_close_reopen_replays_backlog_then_one_caughtup`, `subscribe_exactly_one_caughtup_across_a_large_backlog_on_fjall`.
KEEP: `subscription_cursor_is_static`, `concurrent_append_and_subscribe` (50-task multi-writer drain stress exceeds the kit's boundary-race shape).

- [x] **Step 2.5: `property_tests.rs` — delete ONLY the deterministic 1:1 duplicates** (every `proptest!` fn STAYS — input-space exploration is not replaced by fixed vectors):
`attack_append_read_roundtrip_fixed_evil_payloads`, `attack_version_boundaries_basic`, `attack_concurrent_appends_one_wins`, `attack_read_during_write`, `attack_persistence_across_many_reopens`, `attack_stream_recovery_across_reopen`, `attack_many_streams_recovery`, `attack_many_streams_isolation`, `attack_large_event_stream`, `attack_many_small_events`, `attack_schema_version_min_valid`, `attack_schema_version_u32_max` (kit round-trip already carries a u32::MAX row), `attack_read_from_initial_version`, `attack_stream_fused_after_exhaustion`, `attack_multiple_sequential_appends`, `attack_read_with_from_version_filters`, `attack_event_type_preserved`, `attack_append_after_empty_batch`, `attack_large_payloads` (now a kit check), `attack_sequential_version_enforcement`.
KEEP: every `proptest!` block (`attack_append_read_roundtrip_any_payloads`, `attack_append_read_roundtrip_evil_payloads`, `attack_evil_stream_ids`, `attack_stream_id_isolation_never_leaks`, `attack_version_conflict_detection`, `attack_random_versions_rejected_unless_sequential`, `attack_model_based_shadow_store`, `attack_schema_version_round_trip_any`, `attack_stream_versions_always_monotonic`) and the type-level guards (`attack_very_long_stream_id`, `attack_version_boundaries_near_max`, `attack_schema_version_zero_clamped_by_builder`, `attack_empty_string_stream_id_rejected_by_stream_id_type`).

- [x] **Step 2.6: `resilience_tests.rs` — delete:** `attack_read_snapshot_isolation`, `attack_version_u64_max_read`, `attack_read_from_version_zero`, `attack_read_nonexistent_stream`, `attack_read_past_end_of_stream`, `attack_recovery_torture_100_streams_reopen_5_times`, `attack_concurrent_append_storm_50_tasks`, `attack_concurrent_append_same_stream_conflict`, `attack_version_gap_in_batch`, `attack_version_duplicate_in_batch`, `attack_version_backwards_in_batch`, `attack_empty_batch_does_not_advance_version`, `attack_empty_batch_to_nonexistent_stream`, `attack_large_batch_1000_events`, `attack_reopen_10_times_with_appends`, `attack_concurrent_reads_and_writes`.
KEEP: the overflow-arithmetic white-box quartet, `attack_dual_instance_same_path`, `attack_deterministic_simulation` (DST), `attack_crash_simulation_forget_store`, `attack_recovery_stream_id_counter_correctness`, `attack_empty_batch_wrong_version_to_nonexistent`, `attack_schema_version_zero_rejected_by_type_system`, `attack_custom_builder_config_still_works`, `append_assigns_monotonic_all_position_across_streams` (asserts fjall's CONTIGUOUS GlobalSeq — stronger than the contract's monotonic-not-gapless; keep as a documented fjall implementation property with a comment saying exactly that).

- [x] **Step 2.7: `state_machine_tests.rs` — KEEP whole file** (proptest state-machine methodology).

- [x] **Step 2.8:** prune unused imports/helpers orphaned by the deletions in each touched file; `nix develop -c cargo nextest run -p nexus-fjall` green; fmt; commit `refactor(fjall): drop tests the conformance kit now owns (#281)`.

---

## Task 3: inmemory deletions

- [x] **Step 3.1: `tests/inmemory_store_tests.rs` — delete:** `append_and_read_back`, `read_from_version_filters_correctly`, `append_assigns_monotonic_all_position_across_batches_and_streams`. KEEP `append_conflict_truncates_overlong_stream_id_with_ellipsis` (ErrorId truncation UX).
- [x] **Step 3.2: inline mods in `src/lib.rs`:**
  - `global_read_tests`: delete `read_all_yields_global_order_across_streams`, `read_all_from_is_exclusive_and_resumes`, `subscribe_all_catches_up_then_sees_live_event`, `subscribe_all_sees_concurrent_appends_across_streams`. KEEP `read_all_from_max_yields_empty` (ceiling position — not generically expressible in the kit).
  - `bounded_subscription_tests`: delete `subscription_drains_many_batches_then_sees_new_event` ONLY IF it runs at the default batch size — read it first; if it forces a small batch size, KEEP (white-box seam test the kit can't reach).
  - KEEP whole: `batch_config_tests`, `bounded_read_tests` (batch(4) white-box seams), `wake_source_tests`, `all_pos_tests`.
- [x] **Step 3.3:** prune imports; `nix develop -c cargo nextest run -p nexus-inmemory` green; fmt; commit `refactor(inmemory): drop tests the conformance kit now owns (#281)`.

---

## Task 4: postgres deletions

- [ ] **Step 4.1: `tests/all_noskip_tests.rs` — delete:** `all_noskip_empty_store_yields_nothing`, `all_noskip_via_store_api_ordering`. KEEP the two out-of-order-commit watermark proofs (the #213 design's raison d'être).
- [ ] **Step 4.2: `tests/subscription_tests.rs` — delete:** `subscribe_per_stream_catchup_then_live`, `subscribe_per_stream_from_checkpoint`, `subscribe_per_stream_nonexistent_blocks_then_live` (now a kit check), `subscribe_all_catchup_then_live`, `subscribe_all_concurrent_writers_strictly_increasing`. If the file ends up empty, `git rm` it; if shared helpers remain used by all_noskip_tests, relocate minimally.
- [ ] **Step 4.3: KEEP all inline src tests** (`prepare_inserts` unit tests run WITHOUT a database — the kit's postgres coverage skips locally; deleting them would leave the SQL batch-validation logic with zero local coverage — plus error/position/hex internals).
- [ ] **Step 4.4:** `nix develop -c cargo nextest run -p nexus-postgres` green (vacuous conformance skips + surviving tests); fmt; commit `refactor(postgres): drop tests the conformance kit now owns (#281)`.

---

## Task 5: follow-up issue + final gate + PR

- [ ] **Step 5.1:** file a follow-up GitHub issue: "conformance kit: export/import capability macro (`conformance_export_import!`)" referencing #281 and the fjall `export_import_tests.rs` round-trip as the seed — deliberately deferred from PR2 (the kit's export checks need the StreamLister+EventImporter capability pair; spec listed it out of PR1 scope).
- [ ] **Step 5.2:** `nix develop -c cargo clippy --workspace --all-features --all-targets` clean; `cargo hakari generate` no-op check.
- [ ] **Step 5.3:** push branch; `gh pr create` — title `refactor: adapters consume the conformance kit — dedupe local suites (PR2 of #281)`; body summarizes: N tests deleted (A), 5 kit checks added (B), keeps itemized (C) with the audit rationale; test-plan checklist; ends with the Claude Code attribution.

---

## Deviation log

| # | Task | Deviation | Why | Impact |
|---|------|-----------|-----|--------|
| 1 | 1.1 | Unrolled the two-subscriber checks (`check_two_subscribers_same_stream_both_receive`) into explicit a-then-b blocks instead of `for (stream, name) in [(&mut stream_a, "a"), ...]` | The plan's own NOTE pre-approved this: looping over `[(&mut Pin<&mut St>, &str)]` risks both a borrow-checker fight (two mutable pins in one array) and deny-level `clippy::similar_names` (`stream`/`name` shadow patterns across iterations). Unrolling avoids both with identical assertions. | None — same coverage, same panic messages, just written twice instead of looped. |
| 2 | 2.8 | Pruned `count_events` (`property_tests.rs`) and `read_all_versions` + `AppendError` + `Arc` import (`resilience_tests.rs`) as orphaned after the listed deletions, beyond the plan's explicit "prune unused imports" instruction | Deny-level unused-item lints (`nix develop -c cargo clippy -p nexus-fjall --all-targets --all-features`) would fail the build otherwise; these were dead weight left behind only by helper fns/imports, not by any surviving test body | None — no test coverage lost, clean clippy confirmed |
| 3 | 2.8 | `nix flake check` required 5 retries: first run hit a genuine environment ENOSPC (disk at 99–100% full throughout the session) that killed `nexus-store-nostd`/`nexus-clippy`/`nexus-nostd` mid-build (once on a real target-lib eviction, `E0463 can't find crate for core`; once during the nostd artifact's zstd install-phase write) and cascade-cancelled the other checks in the same parallel batch; retried the same unmodified command until disk headroom recovered (69G freed as prior derivations' scratch space was reclaimed) and all 11 checks passed clean | Disk-space exhaustion is an environment condition, not a code defect — the same command with no code changes between retries eventually went green, confirming the fjall test deletions were never the cause | None on code; documented per the "never bypass the gate" rule — the gate was never skipped, only retried until the environment could sustain the build |
| 4 | 3.2 | `bounded_subscription_tests::subscription_drains_many_batches_then_sees_new_event` was KEPT (not deleted) | Read the test first per the plan's instruction: it constructs `InMemoryStore::with_batch_size(BatchSize::new(4).unwrap())` — a forced non-default batch size of 4 against 40 pre-seeded events (10 full refills) plus one live append. That is exactly the "forces a small batch size" case the plan calls out as a white-box seam the kit can't reach (the kit factory doesn't expose an adapter-specific batch-size override). | None — whole `bounded_subscription_tests` mod left untouched, matching `batch_config_tests`/`bounded_read_tests`/`wake_source_tests`/`all_pos_tests`. |
