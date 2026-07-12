#![cfg_attr(not(feature = "std"), no_std)]

// The production kernel is pure `core` — no allocator required on-device.
// `alloc` is pulled in only for the test fixture and unit tests.
#[cfg(any(test, feature = "testing"))]
extern crate alloc;

mod aggregate;
mod error;
mod error_id;
mod event;
mod events;
mod id;
mod message;
mod saga;
mod version;

#[cfg(feature = "testing")]
pub mod testing;

pub mod closing_the_books;

pub use aggregate::{
    Aggregate, AggregateRoot, AggregateState, DEFAULT_MAX_REHYDRATION_EVENTS, EventOf, Handle,
};
pub use error::KernelError;
pub use error_id::{DEFAULT_ERROR_ID_CAP, ErrorId};
pub use event::DomainEvent;
pub use events::{Events, EventsIntoIter};
pub use id::Id;
pub use message::Message;
pub use saga::{React, Saga};
pub use version::{Version, VersionedEvent};

#[cfg(feature = "derive")]
pub use mnesis_macros::DomainEvent;
#[cfg(feature = "derive")]
pub use mnesis_macros::aggregate;
