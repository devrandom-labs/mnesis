//! Port of axum's `examples/todos` onto mnesis + mnesis-fjall (#326).
//!
//! (Narrative docs land in a later task; this is the compile skeleton.)

// Example code relaxes strict lints locally (production crates do NOT) —
// same posture as `examples/fjall-end-to-end`. `unwrap_used` is allowed
// because upstream handler/main code kept verbatim uses `unwrap()` and the
// diff against the upstream baseline is the point (see PROVENANCE.md).
#![allow(
    clippy::unwrap_used,
    reason = "upstream axum example code is kept verbatim; the port's diff is the deliverable"
)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "example: error/panic conditions are obvious from the narrative"
)]
#![allow(
    clippy::expect_used,
    reason = "example: expect documents an assumption at startup/teardown"
)]

pub mod domain;
pub mod http;
pub mod index;
