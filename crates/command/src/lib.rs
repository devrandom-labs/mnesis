//! Reusable durable command handling over Mnesis and Tokio.
//!
//! One atomic stream record retains a request, its original receipt and all
//! decided domain facts, including accepted no-ops. Application code owns
//! authorization and effects; the supplied aggregate owns domain meaning.
//! Replay only folds facts. It never invokes handlers or external effects.

mod engine;
mod host;
mod model;
mod trace;

pub use host::{CommandProcessor, Limits, Shutdown, Worker};
pub use model::{Acceptance, AcceptanceCodec, Cause, CodecError, Error, Receipt, Request};
