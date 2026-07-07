//! `no_std` compile smoke-test for the nexus derive macros (#304).
//!
//! #279 (PR #303) made the `nexus` kernel `no_std` and added flake gates that
//! build the kernel **bare** (`--no-default-features`). Those gates prove the
//! *core* compiles `no_std`, but they never compile the **output** of
//! `#[nexus::aggregate]` / `#[derive(DomainEvent)]` for a `no_std` target — the
//! macro source was only grepped for `std::` paths, which is weak.
//!
//! This crate closes that gap: it defines a real aggregate using BOTH macros
//! plus a [`Handle`] impl, entirely in `core` (no allocator — `Events<E, 0>` is
//! the single-event `ArrayVec` path). The two flake gates build it for
//! `thumbv7em-none-eabihf` and `wasm32-unknown-unknown`; if a macro ever emits a
//! `std::` path, the generated code fails to compile for `thumbv7em` and the
//! gate goes red. Logic correctness is already covered on the host by the
//! `nexus-cross-crate-test` nextest suite — this crate is a *compile* probe.
#![no_std]

// The whole probe lives behind `derive`: with the feature off this is an empty
// `#![no_std]` lib (trivially portable); with it on, the macro output is what
// gets compiled for the target. `pub` so the probe's items are reachable API —
// otherwise every generated/hand-written item reads as dead code (a hard error
// under the flake's `clippy --deny warnings`).
#[cfg(feature = "derive")]
pub mod smoke;
