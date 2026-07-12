//! Adapter-pluggable wake mechanism for live subscriptions.
//!
//! The generic subscription loop parks on a [`WakeRegistration`] until new
//! events may exist. In-process adapters use `mnesis_wake::StreamNotifiers`;
//! distributed adapters (e.g. postgres) implement these traits over
//! `LISTEN`/`NOTIFY`.
//!
//! Only ever used as a generic bound (never `dyn`), so `arm` is an RPITIT
//! future — no associated `Wait` type, no boxing.

use core::future::Future;

/// A live wake source. `register` returns a handle that keeps wake-routing
/// alive for a target (a stream, or `$all` when `stream` is `None`).
pub trait WakeSource: Send + Sync + 'static {
    /// Handle keeping wake-routing alive; dropped when the subscription ends.
    type Registration: WakeRegistration;
    /// Failure to register (e.g. a subscriber-count overflow).
    type Error: core::error::Error + Send + Sync + 'static;

    /// Register interest in `stream` (`None` = `$all`).
    ///
    /// # Errors
    /// Adapter-specific registration failure.
    fn register(&self, stream: Option<&[u8]>) -> Result<Self::Registration, Self::Error>;

    /// Signal that new events for `stream` (and therefore `$all`) are durably
    /// committed. MUST be called by the adapter *after* commit.
    fn wake(&self, stream: &[u8]);
}

/// An arm-able registration. `arm` returns an owned, lost-wakeup-safe future.
pub trait WakeRegistration: Send + 'static {
    /// Arm a wait. CONTRACT: the returned future captures a "seen version" at
    /// the moment `arm` is called; awaiting it resolves once a wake is
    /// delivered *after* that point — a wake between `arm` and the await is
    /// NOT lost. Spurious wakes permitted. The future is `'static` (carries no
    /// borrow of `self`).
    fn arm(&self) -> impl Future<Output = ()> + Send + 'static;
}
