use core::num::NonZeroU64;
use mnesis_store::checkpoint::CheckpointHydrated;

/// Extract all three persisted facts; tests must retain revision assertions.
///
/// # Panics
/// Panics when the checkpoint is absent or stale.
#[allow(clippy::panic, reason = "a missing checkpoint fails the test")]
pub fn found<S, P>(hydrated: CheckpointHydrated<S, P>) -> (NonZeroU64, P, S) {
    let CheckpointHydrated::Found {
        revision,
        position,
        state,
    } = hydrated
    else {
        panic!("checkpoint must exist at the requested schema");
    };
    (revision, position, state)
}
