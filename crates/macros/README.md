# mnesis-macros

Procedural macros for [`mnesis`](../mnesis). Three macros, zero boilerplate.

## `#[derive(DomainEvent)]`

Derive on a nonempty enum, preserving type, lifetime and const parameters and where clauses. The enum must satisfy `Message` bounds (`Send + Sync + Debug + 'static`). Generates `Message` + `DomainEvent` impls with `name()` returning variant names as `&'static str`.

```rust
#[derive(Debug, Clone, DomainEvent)]
enum AccountEvent {
    Opened(AccountOpened),
    Deposited(MoneyDeposited),
    Closed(AccountClosed),
}
```

## `#[mnesis::aggregate]`

Attribute macro on a nongeneric unit struct without a where clause. Generates `impl Aggregate` plus a convenience `BankAccount::new(id) -> AggregateRoot<Self>` constructor; the struct stays a bare marker. Implement `Handle<C>` on the marker as `handle(state, cmd) -> events`.

```rust
#[mnesis::aggregate(state = AccountState, error = AccountError, id = AccountId)]
struct BankAccount;

impl Handle<Withdraw> for BankAccount {
    fn handle(
        state: &AccountState,
        cmd: Withdraw,
    ) -> Result<Option<Events<AccountEvent>>, AccountError> {
        // pure decision: read state, return decided events — or `Ok(None)`
        // when the command changes nothing
    }
}
```

## `#[mnesis::transforms]`

Attribute macro on a nongeneric inherent impl of a single marker name. Generates `upcast` and `current_version` functions for schema evolution. Transform functions are annotated with `#[transform(event = "...", from = N, to = N+1)]`, optionally with `rename = "NewName"`. Schemas fit nonzero u32 and every step advances by one. Rename destinations participate in chain validation; every declared node must reach the latest declared schema for its final name.

`current_version` returns the latest schema declared for that exact name. A source renamed from `Old@1` to `New@2` remains schema 1 under `Old`; `New` has its own schema. `upcast` returns `TransformError<UserError>`: undeclared schemas of known names are rejected, user errors retain their source, and unknown names pass through for the codec to handle. Earlier writers that stamped a source name with a rename destination's schema require an explicit, validated migration; these bytes are not silently reinterpreted.

Annotated transforms must be ordinary synchronous safe Rust functions with one payload argument, no receiver, generics or where clause, and an explicit result type. Helpers and constants are preserved; `upcast` and `current_version` are reserved. Duplicate arguments/attributes and conditional individual transforms are rejected. Conditional compilation applies to the whole impl. `aggregate` is a parsed documentation label, not an aggregate or payload compatibility check.

```rust
#[mnesis::transforms(aggregate = BankAccount, error = MyUpcastError)]
impl BankAccountTransforms {
    #[transform(event = "Deposited", from = 1, to = 2)]
    fn add_currency(payload: &[u8]) -> Result<Vec<u8>, MyUpcastError> {
        // migrate v1 → v2
    }
}
```

## MSRV & stability

MSRV **1.99** (pinned stable). **1.0 tier**, version-locked to `mnesis` with an exact `=` pin. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
