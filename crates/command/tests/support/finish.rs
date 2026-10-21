use std::time::Duration;

use mnesis_command::Shutdown;
use tokio::task::JoinHandle;

pub async fn finish<C, I>(task: JoinHandle<Shutdown<C, I>>) -> Shutdown<C, I> {
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .expect("host did not stop within its test budget")
        .expect("host task failed")
}
