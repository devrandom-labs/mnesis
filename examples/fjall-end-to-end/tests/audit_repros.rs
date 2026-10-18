//! A14 in todo.md: reject overflow before recording an event.
#![allow(
    clippy::unwrap_used,
    reason = "exact bank arithmetic regression assertions"
)]
use futures::TryStreamExt;
use mnesis::{AggregateState, DecisionError, DomainEvent, Handle, Version};
use mnesis_example_fjall_end_to_end::domain::{
    AccountError, AccountEvent, AccountId, AccountOpened, AccountState, BankAccount, Deposit,
    MoneyDeposited, MoneyWithdrawn, OpenAccount, Withdraw,
};
use mnesis_fjall::{FjallStore, GlobalSeq};
use mnesis_store::{
    CommandRepository, ExecuteError, Execution, PendingBatch, RawEventStore, Repository, StreamKey,
    pending_envelope,
};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn deposit_rejects_balance_overflow() {
    let state = AccountState {
        balance: Ok(u64::MAX),
        is_open: true,
        owner: String::new(),
    };
    assert_eq!(
        <BankAccount as Handle<Deposit>>::handle(&state, Deposit { amount: 1 }).unwrap_err(),
        AccountError::BalanceOverflow {
            balance: u64::MAX,
            amount: 1
        }
    );
}

#[test]
fn invalid_historical_arithmetic_never_panics() {
    for invalid in [
        AccountEvent::Deposited(MoneyDeposited { amount: 1 }),
        AccountEvent::Withdrawn(MoneyWithdrawn { amount: u64::MAX }),
    ] {
        let mut root = BankAccount::new(AccountId("invalid".to_owned()));
        root.replay(
            Version::INITIAL,
            &AccountEvent::Opened(AccountOpened {
                owner: "owner".to_owned(),
            }),
        )
        .unwrap();
        let deposited = if matches!(invalid, AccountEvent::Deposited(_)) {
            u64::MAX
        } else {
            0
        };
        root.replay(
            Version::new(2).unwrap(),
            &AccountEvent::Deposited(MoneyDeposited { amount: deposited }),
        )
        .unwrap();
        let replayed = catch_unwind(AssertUnwindSafe(|| {
            root.replay(Version::new(3).unwrap(), &invalid)
        }));
        assert!(replayed.is_ok(), "historical arithmetic must not panic");
        replayed.unwrap().unwrap();
        let expected = if deposited == u64::MAX {
            AccountError::BalanceOverflow {
                balance: u64::MAX,
                amount: 1,
            }
        } else {
            AccountError::InsufficientFunds {
                balance: 0,
                amount: u64::MAX,
            }
        };
        assert_eq!(root.state().unwrap().balance(), Err(expected.clone()));
        assert!(
            matches!(root.handle(Deposit { amount: 0 }), Err(DecisionError::Domain(error)) if error == expected)
        );
        assert!(
            matches!(root.handle(Withdraw { amount: 0 }), Err(DecisionError::Domain(error)) if error == expected)
        );
        assert!(
            matches!(root.handle(OpenAccount { owner: "repair".to_owned() }), Err(DecisionError::Domain(error)) if error == expected)
        );
        root.replay(
            Version::new(4).unwrap(),
            &AccountEvent::Opened(AccountOpened {
                owner: "repair".to_owned(),
            }),
        )
        .unwrap();
        assert_eq!(root.state().unwrap().balance(), Err(expected));
        assert_eq!(root.state().unwrap().owner, "owner");
        assert_eq!(root.version(), Version::new(4));
    }
}

#[tokio::test]
async fn rejected_deposit_preserves_rows_state_and_global_counter() {
    let directory = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(directory.path())
        .open()
        .unwrap()
        .into_store();
    let repo = store.repository::<BankAccount>().json().build();
    let id = AccountId("maximum".to_owned());
    let key = StreamKey::from_slice(id.as_ref());
    let mut root = BankAccount::new(id.clone());
    let _ = repo
        .execute(
            &mut root,
            OpenAccount {
                owner: "owner".to_owned(),
            },
        )
        .await
        .unwrap();
    let _ = repo
        .execute(&mut root, Deposit { amount: u64::MAX })
        .await
        .unwrap();
    let before: Vec<_> = store
        .read_stream(&key, Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| (row.version(), row.payload().to_vec()))
        .try_collect()
        .await
        .unwrap();
    assert_eq!(before.len(), 2);
    let failure = repo
        .execute(&mut root, Deposit { amount: 1 })
        .await
        .unwrap_err();
    assert!(!failure.is_conflict());
    assert!(matches!(
        failure,
        ExecuteError::Decide(AccountError::BalanceOverflow {
            balance: u64::MAX,
            amount: 1
        })
    ));
    assert_eq!(root.version(), Version::new(2));
    assert_eq!(root.state().unwrap().balance(), Ok(u64::MAX));
    let after: Vec<_> = store
        .read_stream(&key, Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| (row.version(), row.payload().to_vec()))
        .try_collect()
        .await
        .unwrap();
    assert_eq!(after, before);
    let reloaded = repo.load(id).await.unwrap();
    assert_eq!(reloaded.state().unwrap(), root.state().unwrap());
    let next = repo
        .execute(&mut root, Withdraw { amount: 1 })
        .await
        .unwrap();
    assert!(
        matches!(next, Execution::Executed { position, .. } if position == GlobalSeq::new(3).unwrap())
    );
    assert_eq!(root.state().unwrap().balance(), Ok(u64::MAX - 1));
    let shutdown = store.raw().clone();
    drop(repo);
    drop(store);
    shutdown.close().await.unwrap();
}

#[tokio::test]
async fn invalid_persisted_arithmetic_reopens_as_a_typed_unusable_balance() {
    let directory = tempfile::tempdir().unwrap();
    let store = FjallStore::builder(directory.path())
        .open()
        .unwrap()
        .into_store();
    let repo = store.repository::<BankAccount>().json().build();
    let id = AccountId("invalid-history".to_owned());
    let key = StreamKey::from_slice(id.as_ref());
    let mut root = BankAccount::new(id.clone());
    let _ = repo
        .execute(
            &mut root,
            OpenAccount {
                owner: "owner".to_owned(),
            },
        )
        .await
        .unwrap();
    let _ = repo
        .execute(&mut root, Deposit { amount: u64::MAX })
        .await
        .unwrap();
    let invalid = AccountEvent::Deposited(MoneyDeposited { amount: 1 });
    let payload = serde_json::to_vec(&invalid).unwrap();
    let row = pending_envelope(Version::new(3).unwrap())
        .event_type(invalid.name())
        .payload(payload.clone())
        .build()
        .unwrap();
    store
        .append(&key, Version::new(2), PendingBatch::new(&[row]).unwrap())
        .await
        .unwrap();
    let first_shutdown = store.raw().clone();
    drop(repo);
    drop(store);
    first_shutdown.close().await.unwrap();
    let reopened = FjallStore::builder(directory.path())
        .open()
        .unwrap()
        .into_store();
    let loaded_repo = reopened.repository::<BankAccount>().json().build();
    let mut loaded = loaded_repo.load(id).await.unwrap();
    let expected = AccountError::BalanceOverflow {
        balance: u64::MAX,
        amount: 1,
    };
    assert_eq!(invalid.balance_after(u64::MAX), Err(expected.clone()));
    assert_eq!(loaded.state().unwrap().balance(), Err(expected.clone()));
    assert_eq!(loaded.version(), Version::new(3));
    let rejected = loaded_repo
        .execute(&mut loaded, Deposit { amount: 0 })
        .await
        .unwrap_err();
    assert!(matches!(rejected, ExecuteError::Decide(error) if error == expected));
    let rows: Vec<_> = reopened
        .read_stream(&key, Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|record| (record.version(), record.payload().to_vec()))
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.last().unwrap(), &(Version::new(3).unwrap(), payload));
    let final_shutdown = reopened.raw().clone();
    drop(loaded_repo);
    drop(reopened);
    final_shutdown.close().await.unwrap();
}

#[test]
fn deposits_at_the_balance_boundary_and_zero_are_exact() {
    for (balance, amount, expected) in [
        (u64::MAX - 1, 1, u64::MAX),
        (u64::MAX, 0, u64::MAX),
        (0, 0, 0),
        (0, u64::MAX, u64::MAX),
    ] {
        let state = AccountState {
            balance: Ok(balance),
            is_open: true,
            owner: "owner".to_owned(),
        };
        let events = <BankAccount as Handle<Deposit>>::handle(&state, Deposit { amount })
            .unwrap()
            .unwrap();
        assert_eq!(events.len(), 1);
        let folded = state.apply(events.first());
        assert_eq!(folded.balance(), Ok(expected));
        assert_eq!(folded.owner, "owner");
        assert!(folded.is_open);
    }
    let closed = AccountState::initial();
    assert_eq!(
        <BankAccount as Handle<Deposit>>::handle(&closed, Deposit { amount: 0 }).unwrap_err(),
        AccountError::Closed
    );
}
