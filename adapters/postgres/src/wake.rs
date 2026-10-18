//! Notifications accelerate scans; an independent deadline guarantees rescans.

use std::future::Future;
use std::time::Duration;

use mnesis_store::wake::{WakeRegistration, WakeSource};
use mnesis_wake::{NotifyError, WakeReg};

use crate::store::PostgresStore;

const RESCAN_INTERVAL: Duration = Duration::from_secs(1);

/// A routed wake registration with an independent rescan deadline.
pub struct Registration {
    inner: WakeReg,
}

impl WakeRegistration for Registration {
    fn arm(&self) -> impl Future<Output = ()> + Send + 'static {
        // Capture the generation before the confirming scan, not at first poll.
        let wake = self.inner.arm();
        async move {
            tokio::select! {
                () = wake => {},
                () = tokio::time::sleep(RESCAN_INTERVAL) => {},
            }
        }
    }
}

impl WakeSource for PostgresStore {
    type Registration = Registration;
    type Error = NotifyError;

    fn register(&self, stream: Option<&[u8]>) -> Result<Self::Registration, Self::Error> {
        self.wake_registry()
            .register(stream)
            .map(|inner| Registration { inner })
    }

    fn wake(&self, stream: &[u8]) {
        self.wake_registry().wake(stream);
    }
}
