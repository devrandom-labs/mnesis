use std::time::Duration;

use mnesis_command::{Shutdown, Worker};

pub async fn finish<C, I>(task: Worker<C, I>) -> Shutdown<C, I> {
    let report = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("host did not stop within its test budget");
    assert!(
        report.cause().is_none(),
        "host task failed: {:?}",
        report.cause()
    );
    report
}
