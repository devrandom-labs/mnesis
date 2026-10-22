//! Processor acceptance on real disk, with observed adapter and worker closure.
#![allow(
    clippy::unwrap_used,
    reason = "native assertions retain exact outcomes"
)]

use std::num::{NonZeroU64, NonZeroUsize};
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_command::{
    Acceptance, AcceptanceCodec, Cause, CodecError, CommandProcessor, Error, Limits, Receipt,
    Request, Shutdown,
};
use mnesis_fjall::{Durability, FjallError, FjallStore};
use mnesis_store::{
    Decode, PendingBatch, RawEventStore, Store, StoreError, StreamKey, pending_envelope,
};

#[path = "support/counter.rs"]
mod counter;
#[path = "support/finish.rs"]
#[allow(
    clippy::expect_used,
    reason = "shared shutdown assertion retains its deadline diagnostic"
)]
mod shutdown;
use counter::{Counter, Increment, Operation, Rejection, Sum};
use shutdown::finish;

type Factory = CommandProcessor<Counter, Operation>;

struct Saved {
    receipts: [Receipt; 2],
    report: Shutdown<Operation>,
    payloads: Vec<Vec<u8>>,
}

const fn limits() -> Limits {
    Limits {
        queue: NonZeroUsize::MIN,
        operation: Duration::from_secs(3),
    }
}

const fn originals() -> [Request<Operation>; 2] {
    [
        Request {
            id: NonZeroU64::MIN,
            expected: None,
            command: Operation::Add(7),
        },
        Request {
            id: NonZeroU64::new(2).unwrap(),
            expected: Some(Version::INITIAL),
            command: Operation::Ignore,
        },
    ]
}

async fn payloads(store: &Store<FjallStore>) -> Vec<Vec<u8>> {
    store
        .raw()
        .read_stream(&StreamKey::from_slice(b"native"), Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| row.payload().to_vec())
        .try_collect()
        .await
        .unwrap()
}

async fn persist(store: &Store<FjallStore>) -> Saved {
    let [first, noop] = originals();
    let (host, worker) = Factory::start(store, "native".into(), limits()).unwrap();
    let accepted = host.submit(first.clone()).await.unwrap();
    assert_eq!(host.submit(first.clone()).await.unwrap(), accepted);
    let nothing = host.submit(noop).await.unwrap();
    assert_eq!((accepted.version, accepted.events), (Version::INITIAL, 1));
    assert_eq!(
        (Some(nothing.version), nothing.events),
        (Version::INITIAL.next(), 0)
    );
    let changed = Request {
        command: Operation::Ignore,
        ..first.clone()
    };
    assert!(matches!(host.submit(changed).await, Err(Error::Mismatch)));
    let stale = Request {
        id: NonZeroU64::MAX,
        ..first
    };
    assert!(matches!(host.submit(stale).await, Err(Error::Conflict)));
    let rejected = Request {
        id: NonZeroU64::MAX,
        expected: Some(nothing.version),
        command: Operation::Reject,
    };
    assert!(
        matches!(host.submit(rejected).await, Err(Error::Failed(cause))
        if matches!(*cause, Cause::Rejected(Rejection)))
    );
    assert_eq!(host.read().await.unwrap(), (Some(nothing.version), Sum(7)));
    let rows = store
        .raw()
        .read_stream(&StreamKey::from_slice(b"native"), Version::INITIAL)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for (row, original) in rows.iter().zip(originals()) {
        assert_eq!(row.event_type(), "controller.accepted.v2");
        assert_eq!(row.schema_version(), 1);
        let record: Acceptance<Operation, Increment> = AcceptanceCodec.decode(row).unwrap();
        assert_eq!(record.request(), &original);
        assert_eq!(record.receipt().version, row.version());
    }
    let decoded: Acceptance<Operation, Increment> = AcceptanceCodec.decode(&rows[0]).unwrap();
    assert_eq!(decoded.facts(), &[Increment(7)]);
    let empty: Acceptance<Operation, Increment> = AcceptanceCodec.decode(&rows[1]).unwrap();
    assert_eq!(empty.facts(), &[]);
    host.stop();
    let report = finish(worker).await;
    assert_eq!(report.uncertain(), None);
    Saved {
        receipts: [accepted, nothing],
        report,
        payloads: payloads(store).await,
    }
}

fn corrupt<T>(outcome: Result<T, Error<Rejection, NonZeroU64, FjallError>>) {
    assert!(matches!(outcome, Err(Error::Failed(cause))
        if matches!(*cause, Cause::Store(StoreError::Decode(CodecError::Json(ref source)))
            if source.is_syntax())));
}

async fn resumed(store: &Store<FjallStore>, saved: Saved) {
    let (host, worker) = Factory::resume(store, "native".into(), limits(), &saved.report).unwrap();
    assert_eq!(
        host.read().await.unwrap(),
        (Some(saved.receipts[1].version), Sum(7))
    );
    for (original, receipt) in originals().into_iter().zip(saved.receipts) {
        assert_eq!(
            host.retry(original.id).await.unwrap(),
            Some(receipt.clone())
        );
        assert_eq!(host.submit(original).await.unwrap(), receipt);
    }
    assert_eq!(payloads(store).await, saved.payloads);
    let broken = pending_envelope(Version::new(3).unwrap())
        .event_type("controller.accepted.v2")
        .payload(b"!".as_slice())
        .build()
        .unwrap();
    store
        .raw()
        .append(
            &StreamKey::from_slice(b"native"),
            Version::new(2),
            PendingBatch::of(&broken),
        )
        .await
        .unwrap();
    corrupt(host.read().await);
    corrupt(host.retry(NonZeroU64::MIN).await);
    let after = payloads(store).await;
    assert_eq!(after.len(), 3);
    assert_eq!(&after[..2], saved.payloads.as_slice());
    assert_eq!(after[2], b"!");
    host.stop();
    assert_eq!(finish(worker).await.uncertain(), None);
}

#[tokio::test]
async fn acceptances_survive_fjall_close_reopen_and_corrupt_tail_is_never_cached_away() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("db");
    let backing = FjallStore::builder(&path).open().unwrap();
    assert_eq!(backing.durability(), Durability::SyncAll);
    let store = Store::new(backing.clone());
    let saved = persist(&store).await;
    drop(store);
    backing.close().await.unwrap();
    let reopened = FjallStore::builder(&path).open().unwrap();
    let persistent = Store::new(reopened.clone());
    resumed(&persistent, saved).await;
    drop(persistent);
    reopened.close().await.unwrap();
}
