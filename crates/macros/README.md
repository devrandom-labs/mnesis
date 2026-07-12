# mnesis-macros

Procedural macros for [`mnesis`](../mnesis). Three macros, zero boilerplate.

## `#[derive(DomainEvent)]`

Derive on an enum. Generates `Message` + `DomainEvent` impls with `name()` returning variant names as `&'static str`.

```rust
#[derive(Debug, Clone, DomainEvent)]
enum AccountEvent {
    Opened(AccountOpened),
    Deposited(MoneyDeposited),
    Closed(AccountClosed),
}
```

## `#[mnesis::aggregate]`

Attribute macro on a unit struct. Generates `impl Aggregate` plus a convenience `BankAccount::new(id) -> AggregateRoot<Self>` constructor; the struct stays a bare marker. Implement `Handle<C>` on the marker as `handle(state, cmd) -> events`.

```rust
#[mnesis::aggregate(state = AccountState, error = AccountError, id = AccountId)]
struct BankAccount;

impl Handle<Withdraw> for BankAccount {
    fn handle(state: &AccountState, cmd: Withdraw) -> Result<Events<AccountEvent>, AccountError> {
        // pure decision: read state, return decided events
    }
}
```

## `#[mnesis::transforms]`

Attribute macro on an impl block. Generates an `Upcaster` impl for schema evolution. Transform functions are annotated with `#[transform(event = "...", from = N, to = N+1)]`.

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

MSRV **1.95** (pinned stable). **1.0 tier**, version-locked to `mnesis` with an exact `=` pin. See [STABILITY.md](../../STABILITY.md).

## License

Licensed under your choice of [MIT](../../LICENSE-MIT) or [Apache-2.0](../../LICENSE-APACHE).
