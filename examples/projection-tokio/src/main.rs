// examples/projection-tokio — a consumer-owned projection loop over
// mnesis-store primitives (Projector, PersistTrigger, Subscription,
// SnapshotStore). mnesis ships no event-loop runner; this demonstrates
// one concrete loop under tokio. See src/lib.rs for the loop itself.
//
// Run with: cargo run -p mnesis-example-projection-tokio

#![allow(
    clippy::print_stdout,
    reason = "example code prints to demonstrate output"
)]

fn main() {
    println!(
        "Projection is a consumer-owned loop over mnesis-store primitives. \
         See `cargo test -p mnesis-example-projection-tokio` and src/lib.rs."
    );
}
