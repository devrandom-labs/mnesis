//! Generated command histories use the same independent model as stateful fuzzing.
#![allow(clippy::unwrap_used, reason = "property failures retain diagnostics")]
#[path = "support/history.rs"]
mod history;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases:256, failure_persistence:Some(Box::new(proptest::test_runner::FileFailurePersistence::WithSource("regressions"))), ..ProptestConfig::default() })]
    #[test]
    fn generated_histories_preserve_identity_and_reconstruction(bytes in prop::collection::vec(any::<u8>(), 0..384)) {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(history::run(&bytes));
    }
}
