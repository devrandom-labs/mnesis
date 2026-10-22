#![allow(
    clippy::unwrap_used,
    reason = "CLI failures retain exact process output"
)]

use std::num::NonZeroU64;
use std::time::Duration;

use mnesis::Version;
use mnesis_command::Receipt;
use tokio::process::Command;

#[tokio::test]
async fn executable_runs_the_documented_flow_and_finishes() {
    let output = tokio::time::timeout(
        Duration::from_secs(3),
        Command::new(env!("CARGO_BIN_EXE_mnesis-example-command"))
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stderr, Vec::<u8>::new());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let first = Receipt {
        id: NonZeroU64::MIN,
        version: Version::INITIAL,
        events: 1,
    };
    let noop = Receipt {
        id: NonZeroU64::MAX,
        version: Version::INITIAL.next().unwrap(),
        events: 0,
    };
    let expected = format!(
        "first={first:?} duplicate={first:?} noop={noop:?} retry={:?} state=7→7",
        Some(&first)
    );
    assert_eq!(
        stdout
            .lines()
            .filter(|line| line.starts_with("first="))
            .collect::<Vec<_>>(),
        [expected]
    );
    #[cfg(feature = "telemetry")]
    assert_eq!(
        stdout
            .lines()
            .filter_map(|line| line.strip_prefix("\tName         : "))
            .filter(|name| *name == "application")
            .collect::<Vec<_>>(),
        ["application"]
    );
}
