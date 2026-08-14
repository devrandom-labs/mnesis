//! Downstream proof that the closed CQRS error protocols require total maps.

use mnesis_store::{ExecuteError, SagaError};

const fn classify_execute(error: &ExecuteError<&'static str, &'static str>) -> u8 {
    match error {
        ExecuteError::Decide(_) => 1,
        ExecuteError::Store(_) => 2,
    }
}

const fn classify_saga(error: &SagaError<&'static str, &'static str>) -> u8 {
    match error {
        SagaError::React(_) => 1,
        SagaError::Store(_) => 2,
        SagaError::VersionOverflow => 3,
    }
}

#[test]
fn execute_error_exposes_every_closed_failure_domain() {
    assert_eq!(classify_execute(&ExecuteError::Decide("domain")), 1);
    assert_eq!(classify_execute(&ExecuteError::Store("store")), 2);
}

#[test]
fn saga_error_exposes_every_closed_failure_domain() {
    assert_eq!(classify_saga(&SagaError::React("domain")), 1);
    assert_eq!(classify_saga(&SagaError::Store("store")), 2);
    assert_eq!(classify_saga(&SagaError::VersionOverflow), 3);
}
