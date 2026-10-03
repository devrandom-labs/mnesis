use crate::version::Version;
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum KernelError {
    /// An application panic consumed the aggregate state. Reload before use.
    #[error("aggregate state is unavailable after an application panic; reload committed history")]
    PoisonedAggregate,

    #[error("Version mismatch: expected {expected}, got {actual}")]
    VersionMismatch { expected: Version, actual: Version },

    #[error("Rehydration limit exceeded: max {max} events")]
    RehydrationLimitExceeded { max: usize },

    #[error("Version sequence exhausted: cannot exceed u64::MAX events")]
    VersionOverflow,
}

/// Failure to decide a command or react to an event. Kernel integrity failures
/// and application rejections remain distinct and retain their typed sources.
#[derive(Debug, Error)]
pub enum DecisionError<E> {
    #[error("aggregate integrity error: {0}")]
    Kernel(#[from] KernelError),
    #[error("application rejected input: {0}")]
    Domain(#[source] E),
}
