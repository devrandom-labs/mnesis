//! Signed, content-addressed, hash-chained events on mnesis (#185).
//!
//! This example proves a **signed, content-addressed, hash-chained** aggregate
//! fits mnesis's existing traits with **zero kernel/store changes**. The pattern
//! is KERI-*shaped* but is not KERI: no KEL, SAID, key rotation, witnesses, or
//! CESR. It is the cheap validation of the claim "mnesis needs no kernel changes
//! for signed content-addressed events", and a template for Task/Inventory-style
//! domains.
//!
//! # What it exercises
//!
//! - **Signed events** — every event carries an ed25519 signature over a
//!   deterministic blake3 preimage.
//! - **Content-addressed id** — [`RegisterId`](domain::RegisterId) `=
//!   blake3(owner_pubkey)`, a 32-byte [`mnesis::Id`] via the blanket impl.
//! - **Hash chain** — each `Set` event carries the prior event's digest, so the
//!   stream is tamper-evident.
//! - **Crypto in `Handle`** — [`Handle`](mnesis::Handle) verifies the signer is
//!   the owner (state-dependent) before deciding an event.
//! - **Pure fold in `apply`** — [`AggregateState::apply`](mnesis::AggregateState)
//!   folds an already-accepted event with no verification.
//! - **`Projector`** — [`RegisterProjector`](projection::RegisterProjector)
//!   re-verifies signatures and chain links on the untrusted read side, folding
//!   external signed events into a [`RegisterView`](projection::RegisterView) and
//!   rejecting forgeries with `Err`.
//!
//! ## mnesis surfaces used
//!
//! `#[mnesis::aggregate]` / [`Handle`](mnesis::Handle) / [`events!`](mnesis::events),
//! [`AggregateState`](mnesis::AggregateState) / [`AggregateRoot`](mnesis::AggregateRoot),
//! [`DomainEvent`](mnesis::DomainEvent), [`mnesis::Id`],
//! `Store::repository::<A>().json().build()` →
//! [`EventStore`](mnesis_store::EventStore) facade,
//! [`CommandRepository::execute`](mnesis_store::CommandRepository) returning
//! [`Execution`](mnesis_store::Execution) with
//! [`ExecuteError::is_conflict`](mnesis_store::ExecuteError::is_conflict),
//! [`Repository::load`](mnesis_store::Repository), [`Projector`](mnesis_store::Projector),
//! `RawEventStore` via [`FjallStore`](mnesis_fjall::FjallStore), and
//! [`AggregateFixture`](mnesis::testing::AggregateFixture) in the unit tests.
//!
//! # Strain point (filed follow-up)
//!
//! The typed [`EventStore`](mnesis_store::EventStore) facade's `save` /
//! `save_with` build the pending envelope with no `.metadata()`, so the
//! high-level repository path always persists `metadata = None`. A signed
//! consumer that wants signatures in **envelope metadata** (KERI's actual
//! placement) must drop to the raw `RawEventStore::append` seam. This example
//! deliberately embeds the signature **inside the event payload** so it stays on
//! the blessed typed path — see `README.md` and issue #344.

// Example crate: the read model / demo narrate their own error and panic
// conditions, so the doc lints add noise here (production crates keep them on).
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "example: error/panic conditions are obvious from the narrative"
)]

pub mod domain;
pub mod projection;
