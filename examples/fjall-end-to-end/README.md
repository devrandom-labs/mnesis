# fjall-end-to-end

The executable proof that the freeze-ready public API **composes** on the real
persistent adapter. Every other example uses `InMemoryStore`; this one runs the
complete path on `mnesis-fjall`, exercising three surfaces that previously had
zero example coverage — composed over one bank-account domain.

```bash
cargo run -p mnesis-example-fjall-end-to-end     # narrate the three phases
cargo test -p mnesis-example-fjall-end-to-end    # the gate-checked proofs
```

## What it proves

1. **Persistence lifecycle** (`run_persistence`) — persist an aggregate to an
   on-disk `FjallStore` via the typed repository, **close** the keyspace,
   **reopen** it, and rehydrate. The rehydrated state equals the pre-close
   state.
2. **Subscription** (`run_subscription`) — open the catch-up + live-tail cursor,
   drain existing history, then observe a **genuinely live** append on the same
   stream (spawned writer + `Barrier`, bounded by a timeout because the cursor
   never returns `None`). Plus **strict-after resume**: a cursor reopened from
   `Some(v3)` starts at v4, no redelivery.
3. **Export / import** (`run_export_import`) — back several streams up through
   the CBOR box to a **file on disk**, restore into a fresh store, and assert the
   restored aggregates rehydrate **identical** to the originals. Plus the
   normal corruption rejection and explicit salvage with per-block corrupt
   markers. Malformed framing retains its separate decode error.

The real assertions live in `#[tokio::test]`s (run by the gate's nextest);
`main.rs` drives the same three functions for a human.

## Aggregate binding (#243)

The aggregate is named once, at `store.repository::<BankAccount>()`; the facade
then implements `Repository<BankAccount>` for exactly that aggregate, so
`repo.load(id)` / `repo.save(..)` infer it with no per-call annotation. This
example was the canary for #243 — with that landed, the former
`let acct: AggregateRoot<BankAccount> = repo.load(id)…` annotations are gone.

## Deciding and persisting (#227, #251)

The sanctioned one-call command path is `repo.execute(&mut root, cmd)` — it
fuses `AggregateRoot::handle` (decide) and `Repository::save` (persist) into
one call, so the decided events can never be forgotten or misthreaded between
the two steps. The manual two-step `root.handle(cmd)?` +
`repo.save(&mut root, &decided).await?` remains available as the escape hatch
when a caller needs to inspect the decided events before they land.

## Arithmetic and invalid historical records

Deposit decisions use checked addition and return `AccountError::BalanceOverflow`
before emitting an event. Withdrawal decisions reject insufficient funds. Zero
amounts remain accepted events for open accounts; closed accounts reject them.

The kernel's `AggregateState::apply` interface is infallible. This example stores
`balance: Result<u64, AccountError>` so invalid historical arithmetic has an
explicit typed result instead of a panic, wrapped value or substitute balance.
The first overflowing deposit or underflowing withdrawal replaces the balance
with its exact cause. Subsequent folds leave that cause and the other state
fields unchanged; every command rejects that cause, including zero amounts and
attempts to reopen. Use `state.balance()?` before consuming the balance. This is
distinct from kernel poisoning after an application panic: repository replay
returns a root whose version records the consumed history, while the domain
balance is unusable. No storage rollback, implicit repair or deletion occurs.
The subscription fold uses the same checked event transition and returns its
error to the driver.

`tests/audit_repros.rs` retains the original decision-overflow regression and
covers exact bounds, zero/closed behavior, both historical arithmetic failures,
sticky errors, rejected-command raw rows/state/global counters, and a real
invalid persisted record surviving close/reopen. Run both arithmetic profiles:

```sh
nix develop -c cargo test -p mnesis-example-fjall-end-to-end --all-features --locked
nix develop -c cargo test -p mnesis-example-fjall-end-to-end --release --all-features --locked
```

The standalone `inmemory`, `store-and-kernel` and `closing-the-books` examples
also preserve typed arithmetic failures and retain their own regressions against
their actual domain implementations. The shift example checks opening float
plus registered total before accepting another transaction; closing computes
checked expected tender and uses absolute differences for overage/shortage.
