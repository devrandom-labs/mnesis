//! Generated command histories use the same independent model as stateful fuzzing.
#![allow(clippy::unwrap_used, reason = "property failures retain diagnostics")]
#[path = "support/history.rs"]
mod history;
#[path = "support/races.rs"]
mod races;

use std::num::NonZeroU64;

use history::{faults, shutdown};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases:256, failure_persistence:Some(Box::new(proptest::test_runner::FileFailurePersistence::WithSource("regressions"))), ..ProptestConfig::default() })]
    #[test]
    fn generated_histories_preserve_identity_and_reconstruction(bytes in prop::collection::vec(any::<u8>(), 0..384)) {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(history::run(&bytes));
    }

    #[test]
    fn generated_races_preserve_one_original_acceptance(
        kind in prop_oneof![Just(races::Kind::Duplicate), Just(races::Kind::Conflict), Just(races::Kind::Mismatch)],
        winner in prop_oneof![Just(races::Winner::First), Just(races::Winner::Second)],
        id in prop_oneof![Just(NonZeroU64::MIN), Just(NonZeroU64::new(2).unwrap()), Just(NonZeroU64::new(u64::MAX - 1).unwrap()), Just(NonZeroU64::MAX), (1..=u64::MAX).prop_map(|value| NonZeroU64::new(value).unwrap())],
        amount in prop_oneof![Just(0), Just(1), Just(u8::MAX - 1), Just(u8::MAX), any::<u8>()],
        prefix in prop::option::of(any::<u8>())
    ) {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
            .block_on(races::run(kind, winner, id, amount, prefix));
    }
}
