//! serde types mirroring cargo-mutants `outcomes.json` / `mutants.json` and this
//! repo's `mutants-baseline.json`. Only the fields the gate needs are modelled;
//! unknown fields are ignored (no `deny_unknown_fields`), so a cargo-mutants
//! minor bump that adds fields does not break the parse.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The top of `mutants.out/outcomes.json`.
#[derive(Debug, Deserialize)]
pub(crate) struct Report {
    pub(crate) outcomes: Vec<Outcome>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Outcome {
    pub(crate) summary: Summary,
    pub(crate) scenario: Scenario,
}

/// Externally-tagged: `"CaughtMutant"` etc. deserialize as unit variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) enum Summary {
    Success,
    CaughtMutant,
    MissedMutant,
    Unviable,
    Timeout,
    /// cargo-mutants emits this when a test process fails for a non-mutation
    /// reason — most often the **unmutated baseline** suite failing to pass (a
    /// broken or wrongly-filtered test run), in which case NO mutants are tested.
    /// Modelled so the parse never crashes; a baseline `Failure` is caught as a
    /// hard error before the empty result can masquerade as a clean baseline.
    Failure,
}

/// Either the bare string `"Baseline"` or `{ "Mutant": { .. } }`.
#[derive(Debug, Deserialize)]
pub(crate) enum Scenario {
    Baseline,
    Mutant(MutantInfo),
}

#[derive(Debug, Deserialize)]
pub(crate) struct MutantInfo {
    pub(crate) file: String,
    /// `null` for a mutation that is not inside a function — e.g. an associated
    /// `const` like `Capacity::MAX` (cargo-mutants reports `"function": null`).
    pub(crate) function: Option<FunctionInfo>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FunctionInfo {
    pub(crate) function_name: String,
}

/// One element of the top-level `mutants.json` array (candidate list).
#[derive(Debug, Deserialize)]
pub(crate) struct Candidate {
    pub(crate) file: String,
    /// `null` for a non-function mutation (see [`MutantInfo::function`]).
    pub(crate) function: Option<FunctionInfo>,
}

/// `mutants-baseline.json`: the committed ratchet.
#[derive(Debug, Deserialize)]
pub(crate) struct Baseline {
    /// `"file::function_name"` -> minimum viable mutant count (>= 1).
    pub(crate) floors: BTreeMap<String, usize>,
    /// `"file::function_name"` documented as structurally 0-viable.
    pub(crate) known_zero_viable: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_mutant_outcome() {
        let json = r#"{
            "summary": "CaughtMutant",
            "scenario": { "Mutant": {
                "file": "crates/mnesis/src/aggregate.rs",
                "function": { "function_name": "handle_message" }
            } }
        }"#;
        let outcome: Outcome = serde_json::from_str(json).unwrap();
        assert_eq!(outcome.summary, Summary::CaughtMutant);
        let Scenario::Mutant(m) = outcome.scenario else {
            unreachable!("expected a Mutant scenario");
        };
        assert_eq!(m.file, "crates/mnesis/src/aggregate.rs");
        assert_eq!(m.function.unwrap().function_name, "handle_message");
    }

    #[test]
    fn parses_a_null_function_mutant() {
        // A mutation of an associated const (not inside a fn) has function: null.
        let json = r#"{
            "summary": "CaughtMutant",
            "scenario": { "Mutant": {
                "file": "crates/mnesis/src/version.rs",
                "function": null
            } }
        }"#;
        let outcome: Outcome = serde_json::from_str(json).unwrap();
        let Scenario::Mutant(m) = outcome.scenario else {
            unreachable!("expected a Mutant scenario");
        };
        assert_eq!(m.file, "crates/mnesis/src/version.rs");
        assert!(m.function.is_none());
    }

    #[test]
    fn parses_the_baseline_scenario_string() {
        let json = r#"{ "summary": "Success", "scenario": "Baseline" }"#;
        let outcome: Outcome = serde_json::from_str(json).unwrap();
        assert_eq!(outcome.summary, Summary::Success);
        assert!(matches!(outcome.scenario, Scenario::Baseline));
    }

    #[test]
    fn parses_a_candidate() {
        let json = r#"[{
            "file": "bombay-core/src/mailbox.rs",
            "function": { "function_name": "recv" }
        }]"#;
        let candidates: Vec<Candidate> = serde_json::from_str(json).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].function.as_ref().unwrap().function_name,
            "recv"
        );
    }
}
