//! Hard process loss before native append and after its durable commit.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "native crash assertions"
)]

#[path = "support/counter.rs"]
mod counter;
mod faults;
#[path = "support/finish.rs"]
mod shutdown;

use std::io::Write;
use std::num::{NonZeroU64, NonZeroUsize};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures::TryStreamExt;
use mnesis::Version;
use mnesis_command::{Acceptance, AcceptanceCodec, CommandProcessor, Limits, Receipt, Request};
use mnesis_fjall::{Durability, FjallStore};
use mnesis_store::{Decode, RawEventStore, Store, StreamKey};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;
use tower::{Service, ServiceExt};

use counter::{Counter, Increment, Operation, Sum};
use faults::{Fault, FaultStore, Gate};

const PATH: &str = "MNESIS_COMMAND_CRASH_PATH";
const PHASE: &str = "MNESIS_COMMAND_CRASH_PHASE";
const MARKER: &str = "MNESIS_COMMAND_NATIVE_BOUNDARY";
const TEST: &str = "exact_original_recovers_after_killed_process_on_both_sides_of_commit";
type Factory = CommandProcessor<Counter, Operation>;

#[derive(Clone, Copy)]
enum Boundary {
    Before,
    Committed,
}
impl Boundary {
    const fn name(self) -> &'static str {
        match self {
            Self::Before => "before",
            Self::Committed => "committed",
        }
    }
    const fn rows(self) -> usize {
        match self {
            Self::Before => 0,
            Self::Committed => 1,
        }
    }
}
const fn original() -> Request<Operation> {
    Request {
        id: NonZeroU64::MIN,
        expected: None,
        command: Operation::Add(7),
    }
}
const fn receipt() -> Receipt {
    Receipt {
        id: NonZeroU64::MIN,
        version: Version::INITIAL,
        events: 1,
    }
}
const fn limits() -> Limits {
    Limits {
        queue: NonZeroUsize::MIN,
        operation: Duration::from_secs(60),
    }
}
async fn records(raw: &FjallStore) -> Vec<Acceptance<Operation, Increment>> {
    raw.read_stream(&StreamKey::from_slice(b"crash"), Version::INITIAL)
        .await
        .unwrap()
        .map_ok(|row| AcceptanceCodec.decode(&row).unwrap())
        .try_collect()
        .await
        .unwrap()
}
fn exact(record: &Acceptance<Operation, Increment>) {
    assert_eq!(record.request(), &original());
    assert_eq!(record.receipt(), &receipt());
    assert_eq!(record.facts(), &[Increment(7)]);
}

async fn child(path: &Path, boundary: Boundary) {
    let raw = FjallStore::builder(path.join("db")).open().unwrap();
    assert_eq!(raw.durability(), Durability::SyncAll);
    let fault = FaultStore::new(raw);
    let gate = Gate::new();
    match boundary {
        Boundary::Before => *fault.append_gate.lock() = Some(Arc::clone(&gate)),
        Boundary::Committed => fault.arm(Fault::HoldCommitted(Arc::clone(&gate))),
    }
    let store = Store::new(fault);
    let (mut host, _worker) = Factory::start(&store, "crash".into(), limits()).unwrap();
    let _reply = host.ready().await.unwrap().call(original());
    gate.entered().await;
    let observed = records(&store.raw().inner).await;
    assert_eq!(observed.len(), boundary.rows());
    for record in &observed {
        exact(record);
    }
    {
        let mut output = std::io::stdout().lock();
        writeln!(output, "\n{MARKER}").unwrap();
        output.flush().unwrap();
    }
    std::future::pending::<()>().await;
}

async fn killed(path: &Path, boundary: Boundary) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
        .env(PATH, path)
        .env(PHASE, boundary.name())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut last = None;
    let reached = timeout(Duration::from_secs(30), async {
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("child exited before native boundary");
            if line == MARKER {
                break;
            }
            last = Some(line);
        }
    })
    .await;
    assert!(
        reached.is_ok(),
        "native boundary absent; last line: {last:?}"
    );
    timeout(Duration::from_secs(3), child.kill())
        .await
        .unwrap()
        .unwrap();
    let status = timeout(Duration::from_secs(3), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.signal(), Some(9));
}

async fn recover(path: &Path, boundary: Boundary) {
    let raw = FjallStore::builder(path.join("db")).open().unwrap();
    let observed = records(&raw).await;
    assert_eq!(observed.len(), boundary.rows());
    for record in &observed {
        exact(record);
    }
    let store = Store::new(raw.clone());
    let (host, worker) = Factory::start(&store, "crash".into(), limits()).unwrap();
    match boundary {
        Boundary::Before => {
            assert_eq!(host.retry(original().id).await.unwrap(), None);
            assert_eq!(host.read().await.unwrap(), (None, Sum(0)));
        }
        Boundary::Committed => {
            assert_eq!(host.retry(original().id).await.unwrap(), Some(receipt()));
            assert_eq!(host.read().await.unwrap(), (Some(Version::INITIAL), Sum(7)));
        }
    }
    assert_eq!(host.submit(original()).await.unwrap(), receipt());
    assert_eq!(host.retry(original().id).await.unwrap(), Some(receipt()));
    assert_eq!(host.read().await.unwrap(), (Some(Version::INITIAL), Sum(7)));
    host.stop();
    assert_eq!(shutdown::finish(worker).await.uncertain(), None);
    let final_records = records(&raw).await;
    assert_eq!(final_records.len(), 1);
    exact(&final_records[0]);
    drop(store);
    raw.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exact_original_recovers_after_killed_process_on_both_sides_of_commit() {
    if let Some(path) = std::env::var_os(PATH) {
        let phase = std::env::var(PHASE).unwrap();
        let boundary = [Boundary::Before, Boundary::Committed]
            .into_iter()
            .find(|boundary| boundary.name() == phase)
            .expect("invalid native child phase");
        child(Path::new(&path), boundary).await;
        return;
    }
    for boundary in [Boundary::Before, Boundary::Committed] {
        let directory = tempfile::tempdir().unwrap();
        killed(directory.path(), boundary).await;
        recover(directory.path(), boundary).await;
    }
}
