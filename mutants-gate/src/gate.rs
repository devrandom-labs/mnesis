//! Validate report integrity before evaluating mutation coverage floors.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::model::{Baseline, Candidate, Genre, MutantInfo, Report, Scenario, Summary};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaselinePolicy {
    Required,
    Skipped,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum RunError {
    #[error("candidate list is empty")]
    Empty,
    #[error("invalid mutation identity: {0:?}")]
    InvalidIdentity(Box<MutantInfo>),
    #[error("duplicate candidate: {0:?}")]
    DuplicateCandidate(Box<MutantInfo>),
    #[error("unknown candidate outcome: {0:?}")]
    UnknownOutcome(Box<MutantInfo>),
    #[error("duplicate candidate outcome: {0:?}")]
    DuplicateOutcome(Box<MutantInfo>),
    #[error("missing candidate outcome: {0:?}")]
    MissingOutcome(Box<MutantInfo>),
    #[error("invalid mutant summary: {0:?}")]
    InvalidSummary(Summary),
    #[error("unmutated baseline did not pass: {0:?}")]
    BaselineFailed(Summary),
    #[error("expected one baseline under Required, zero under Skipped; got {count} ({policy:?})")]
    BaselineCount {
        count: usize,
        policy: BaselinePolicy,
    },
    #[error("cannot seed a baseline from a surviving or timed-out mutant")]
    UncleanSeed,
}

#[derive(Debug)]
pub(crate) struct ValidatedRun {
    tallies: BTreeMap<String, Vec<Summary>>,
}

fn key(info: &MutantInfo) -> String {
    info.function.as_ref().map_or_else(
        || format!("{}::<module>", info.file),
        |function| format!("{}::{}", info.file, function.function_name),
    )
}

fn valid_identity(info: &MutantInfo) -> bool {
    !info.package.is_empty()
        && !info.file.is_empty()
        && info.span.start <= info.span.end
        && (info.genre != Genre::FnValue || info.function.is_some())
        && info.function.as_ref().is_none_or(|function| {
            !function.function_name.is_empty()
                && function.span.start <= info.span.start
                && info.span.end <= function.span.end
        })
}

fn baseline_check(report: &Report, policy: BaselinePolicy) -> Result<(), RunError> {
    let baselines: Vec<_> = report
        .outcomes
        .iter()
        .filter(|outcome| matches!(outcome.scenario, Scenario::Baseline))
        .collect();
    let expected = usize::from(policy == BaselinePolicy::Required);
    if baselines.len() != expected {
        return Err(RunError::BaselineCount {
            count: baselines.len(),
            policy,
        });
    }
    for baseline in baselines {
        if baseline.summary != Summary::Success {
            return Err(RunError::BaselineFailed(baseline.summary));
        }
    }
    Ok(())
}

pub(crate) fn validate(
    report: &Report,
    candidates: &[Candidate],
    policy: BaselinePolicy,
) -> Result<ValidatedRun, RunError> {
    baseline_check(report, policy)?;
    if candidates.is_empty() {
        return Err(RunError::Empty);
    }
    let mut pending = BTreeSet::new();
    for candidate in candidates {
        if !valid_identity(candidate) {
            return Err(RunError::InvalidIdentity(Box::new(candidate.clone())));
        }
        if !pending.insert(candidate) {
            return Err(RunError::DuplicateCandidate(Box::new(candidate.clone())));
        }
    }
    let expected = pending.clone();
    let mut tallies: BTreeMap<String, Vec<Summary>> = BTreeMap::new();
    for outcome in &report.outcomes {
        let Scenario::Mutant(info) = &outcome.scenario else {
            continue;
        };
        if !expected.contains(info) {
            return Err(RunError::UnknownOutcome(Box::new(info.clone())));
        }
        if !pending.remove(info) {
            return Err(RunError::DuplicateOutcome(Box::new(info.clone())));
        }
        match outcome.summary {
            Summary::Success | Summary::Failure => {
                return Err(RunError::InvalidSummary(outcome.summary));
            }
            Summary::CaughtMutant
            | Summary::MissedMutant
            | Summary::Unviable
            | Summary::Timeout => {}
        }
        tallies.entry(key(info)).or_default().push(outcome.summary);
    }
    if let Some(missing) = pending.into_iter().next() {
        return Err(RunError::MissingOutcome(Box::new(missing.clone())));
    }
    Ok(ValidatedRun { tallies })
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Failure {
    #[error("surviving mutant in {key}")]
    Survivor { key: String },
    #[error("timed-out mutant in {key}")]
    Timeout { key: String },
    #[error("{key}: viable count {viable} below floor {floor}")]
    Collapse {
        key: String,
        floor: usize,
        viable: usize,
    },
    #[error("unaccounted function {key} ({viable} viable)")]
    Unaccounted { key: String, viable: usize },
    #[error("stale floor for {key} ({floor})")]
    StaleFloor { key: String, floor: usize },
    #[error("invalid zero floor for {key}")]
    InvalidFloor { key: String },
    #[error("duplicate known-zero entry or overlapping floor for {key}")]
    InvalidKnownZero { key: String },
}

fn viable(summaries: &[Summary]) -> usize {
    summaries
        .iter()
        .filter(|summary| {
            matches!(
                summary,
                Summary::CaughtMutant | Summary::MissedMutant | Summary::Timeout
            )
        })
        .count()
}

pub(crate) fn evaluate(run: &ValidatedRun, baseline: &Baseline) -> Vec<Failure> {
    let mut failures = Vec::new();
    let mut zero = BTreeSet::new();
    for name in &baseline.known_zero_viable {
        if !zero.insert(name) || baseline.floors.contains_key(name) {
            failures.push(Failure::InvalidKnownZero { key: name.clone() });
        }
    }
    for (name, &floor) in &baseline.floors {
        if floor == 0 {
            failures.push(Failure::InvalidFloor { key: name.clone() });
        }
        if !run.tallies.contains_key(name) {
            failures.push(Failure::StaleFloor {
                key: name.clone(),
                floor,
            });
        }
    }
    for (name, summaries) in &run.tallies {
        if summaries.contains(&Summary::MissedMutant) {
            failures.push(Failure::Survivor { key: name.clone() });
        }
        if summaries.contains(&Summary::Timeout) {
            failures.push(Failure::Timeout { key: name.clone() });
        }
        let count = viable(summaries);
        match baseline.floors.get(name) {
            Some(&floor) if count < floor => failures.push(Failure::Collapse {
                key: name.clone(),
                floor,
                viable: count,
            }),
            None if !zero.contains(name) => failures.push(Failure::Unaccounted {
                key: name.clone(),
                viable: count,
            }),
            Some(_) | None => {}
        }
    }
    failures
}

pub(crate) fn render_report(run: &ValidatedRun) -> String {
    let mut text = String::new();
    let total = run.tallies.values().flatten().count();
    let viable_count = run
        .tallies
        .values()
        .flatten()
        .filter(|summary| {
            matches!(
                summary,
                Summary::CaughtMutant | Summary::MissedMutant | Summary::Timeout
            )
        })
        .count();
    let _ = writeln!(
        text,
        "mutation coverage: {viable_count} viable / {total} total"
    );
    let _ = writeln!(
        text,
        "{:<60} {:>6} {:>6} {:>8}",
        "file::function", "viable", "total", "unviable"
    );
    for (name, summaries) in &run.tallies {
        let _ = writeln!(
            text,
            "{:<60} {:>6} {:>6} {:>8}",
            name,
            viable(summaries),
            summaries.len(),
            summaries
                .iter()
                .filter(|summary| **summary == Summary::Unviable)
                .count()
        );
    }
    text
}

pub(crate) fn emit_baseline(run: &ValidatedRun) -> Result<String, EmitError> {
    if run.tallies.values().any(|summaries| {
        summaries
            .iter()
            .any(|summary| matches!(summary, Summary::MissedMutant | Summary::Timeout))
    }) {
        return Err(RunError::UncleanSeed.into());
    }
    let floors: BTreeMap<_, _> = run
        .tallies
        .iter()
        .filter_map(|(name, summaries)| {
            let count = viable(summaries);
            (count != 0).then_some((name, count))
        })
        .collect();
    let known_zero: Vec<_> = run
        .tallies
        .iter()
        .filter_map(|(name, summaries)| (viable(summaries) == 0).then_some(name))
        .collect();
    serde_json::to_string_pretty(
        &serde_json::json!({"floors": floors, "known_zero_viable": known_zero}),
    )
    .map_err(EmitError::Serialize)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum EmitError {
    #[error(transparent)]
    Run(#[from] RunError),
    #[error("serializing baseline: {0}")]
    Serialize(#[source] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use super::*;
    use crate::model::Outcome;

    fn candidates() -> Vec<Candidate> {
        serde_json::from_str(include_str!(
            "../tests/audit_cases/real-candidates-27.1.0.json"
        ))
        .unwrap()
    }

    fn report(candidates: &[Candidate], summary: Summary) -> Report {
        let mut outcomes = vec![Outcome {
            summary: Summary::Success,
            scenario: Scenario::Baseline,
        }];
        outcomes.extend(candidates.iter().cloned().map(|info| Outcome {
            summary,
            scenario: Scenario::Mutant(info),
        }));
        Report { outcomes }
    }

    fn baseline(run: &ValidatedRun) -> Baseline {
        serde_json::from_str(&emit_baseline(run).unwrap()).unwrap()
    }

    #[test]
    fn incomplete_or_invalid_identity_is_rejected() {
        let value: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/audit_cases/real-candidates-27.1.0.json"
        ))
        .unwrap();
        for field in [
            "package",
            "file",
            "function",
            "span",
            "replacement",
            "genre",
        ] {
            let mut missing = value[0].clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<Candidate>(missing).is_err(),
                "{field}"
            );
        }
        let mut zero_position = value[0].clone();
        zero_position["span"]["start"]["line"] = serde_json::json!(0);
        assert!(serde_json::from_value::<Candidate>(zero_position).is_err());
        let mut unknown_genre = value[0].clone();
        unknown_genre["genre"] = serde_json::json!("UnknownGenre");
        assert!(serde_json::from_value::<Candidate>(unknown_genre).is_err());
        let mut candidates = candidates();
        candidates[0].span.end = candidates[0].span.start;
        candidates[0].span.start.column = NonZeroU64::new(u64::MAX).unwrap();
        let report = report(&candidates, Summary::CaughtMutant);
        assert_eq!(
            validate(&report, &candidates, BaselinePolicy::Required).unwrap_err(),
            RunError::InvalidIdentity(Box::new(candidates[0].clone()))
        );
        let mut no_function: Candidate = serde_json::from_value(value[0].clone()).unwrap();
        no_function.function = None;
        let invalid = vec![no_function.clone()];
        let invalid_report = self::report(&invalid, Summary::CaughtMutant);
        assert_eq!(
            validate(&invalid_report, &invalid, BaselinePolicy::Required).unwrap_err(),
            RunError::InvalidIdentity(Box::new(no_function))
        );
    }

    #[test]
    fn real_exporter_identities_round_trip_and_enforce_floors() {
        let candidates = candidates();
        let report = report(&candidates, Summary::CaughtMutant);
        let run = validate(&report, &candidates, BaselinePolicy::Required).unwrap();
        let base = baseline(&run);
        assert_eq!(evaluate(&run, &base), vec![]);
        assert_eq!(base.floors.values().sum::<usize>(), candidates.len());
        assert_eq!(base.known_zero_viable, Vec::<String>::new());
    }

    #[test]
    fn every_summary_is_checked_for_each_scenario() {
        let candidates = candidates();
        for summary in [
            Summary::Success,
            Summary::CaughtMutant,
            Summary::MissedMutant,
            Summary::Unviable,
            Summary::Timeout,
            Summary::Failure,
        ] {
            let mut mutant_report = report(&candidates, summary);
            let outcome = validate(&mutant_report, &candidates, BaselinePolicy::Required);
            match summary {
                Summary::Success | Summary::Failure => {
                    assert_eq!(outcome.unwrap_err(), RunError::InvalidSummary(summary));
                }
                Summary::CaughtMutant
                | Summary::MissedMutant
                | Summary::Unviable
                | Summary::Timeout => assert!(outcome.is_ok()),
            }
            mutant_report.outcomes[0].summary = summary;
            if summary != Summary::Success {
                assert_eq!(
                    validate(&mutant_report, &candidates, BaselinePolicy::Required).unwrap_err(),
                    RunError::BaselineFailed(summary)
                );
            }
        }
    }

    #[test]
    fn baseline_skipping_must_be_explicit_and_consistent() {
        let candidates = candidates();
        let mut report = report(&candidates, Summary::CaughtMutant);
        assert_eq!(
            validate(&report, &candidates, BaselinePolicy::Skipped).unwrap_err(),
            RunError::BaselineCount {
                count: 1,
                policy: BaselinePolicy::Skipped
            }
        );
        report.outcomes.remove(0);
        assert_eq!(
            validate(&report, &candidates, BaselinePolicy::Required).unwrap_err(),
            RunError::BaselineCount {
                count: 0,
                policy: BaselinePolicy::Required
            }
        );
        assert!(validate(&report, &candidates, BaselinePolicy::Skipped).is_ok());
        report.outcomes.extend([
            Outcome {
                summary: Summary::Success,
                scenario: Scenario::Baseline,
            },
            Outcome {
                summary: Summary::Success,
                scenario: Scenario::Baseline,
            },
        ]);
        assert_eq!(
            validate(&report, &candidates, BaselinePolicy::Required).unwrap_err(),
            RunError::BaselineCount {
                count: 2,
                policy: BaselinePolicy::Required
            }
        );
    }

    #[test]
    fn duplicate_missing_and_empty_reports_fail() {
        let candidates = candidates();
        let mut report = report(&candidates, Summary::CaughtMutant);
        report.outcomes.pop();
        assert_eq!(
            validate(&report, &candidates, BaselinePolicy::Required).unwrap_err(),
            RunError::MissingOutcome(Box::new(candidates.last().unwrap().clone()))
        );
        let mut duplicate_report = self::report(&candidates, Summary::CaughtMutant);
        duplicate_report.outcomes.push(Outcome {
            summary: Summary::CaughtMutant,
            scenario: Scenario::Mutant(candidates[0].clone()),
        });
        assert_eq!(
            validate(&duplicate_report, &candidates, BaselinePolicy::Required).unwrap_err(),
            RunError::DuplicateOutcome(Box::new(candidates[0].clone()))
        );
        let duplicate = vec![candidates[0].clone(), candidates[0].clone()];
        assert_eq!(
            validate(&duplicate_report, &duplicate, BaselinePolicy::Required).unwrap_err(),
            RunError::DuplicateCandidate(Box::new(candidates[0].clone()))
        );
        assert_eq!(
            validate(&duplicate_report, &[], BaselinePolicy::Required).unwrap_err(),
            RunError::Empty
        );
    }

    #[test]
    fn identity_changes_cannot_substitute_another_candidate() {
        let candidates = candidates();
        let original = &candidates[0];
        let mut substitutions = vec![original.clone(); 7];
        substitutions[0].package = "wrong-package".to_owned();
        substitutions[1].file = "wrong.rs".to_owned();
        substitutions[2].replacement = "wrong replacement".to_owned();
        substitutions[3].genre = Genre::StructField;
        substitutions[4].span.end = substitutions[4].span.start;
        substitutions[5].function.as_mut().unwrap().function_name = "wrong-function".to_owned();
        substitutions[6].function.as_mut().unwrap().return_type = "-> Wrong".to_owned();
        for substitute in substitutions {
            assert_ne!(&substitute, original);
            let mut report = report(&candidates, Summary::CaughtMutant);
            report.outcomes[1].scenario = Scenario::Mutant(substitute.clone());
            assert_eq!(
                validate(&report, &candidates, BaselinePolicy::Required).unwrap_err(),
                RunError::UnknownOutcome(Box::new(substitute))
            );
        }
    }

    #[test]
    fn unclean_runs_cannot_seed_and_known_zero_does_not_excuse_survivors() {
        let candidates = candidates();
        for summary in [Summary::MissedMutant, Summary::Timeout] {
            let report = report(&candidates, summary);
            let run = validate(&report, &candidates, BaselinePolicy::Required).unwrap();
            assert!(matches!(
                emit_baseline(&run),
                Err(EmitError::Run(RunError::UncleanSeed))
            ));
            let base = Baseline {
                floors: BTreeMap::new(),
                known_zero_viable: run.tallies.keys().cloned().collect(),
            };
            let failures = evaluate(&run, &base);
            let expected: Vec<_> = run
                .tallies
                .keys()
                .map(|name| match summary {
                    Summary::MissedMutant => Failure::Survivor { key: name.clone() },
                    _ => Failure::Timeout { key: name.clone() },
                })
                .collect();
            assert_eq!(failures, expected);
        }
    }

    #[test]
    fn module_mutations_and_unviable_results_remain_accounted() {
        let mut candidates = candidates();
        candidates[0].function = None;
        candidates[0].genre = Genre::BinaryOperator;
        let report = report(&candidates, Summary::Unviable);
        let run = validate(&report, &candidates, BaselinePolicy::Required).unwrap();
        let base = baseline(&run);
        assert_eq!(base.floors, BTreeMap::new());
        assert_eq!(
            base.known_zero_viable,
            run.tallies.keys().cloned().collect::<Vec<_>>()
        );
        assert_eq!(evaluate(&run, &base), vec![]);
    }

    #[test]
    fn ratchet_rejects_stale_invalid_collapsed_and_unaccounted_entries() {
        let candidates = candidates();
        let report = report(&candidates, Summary::CaughtMutant);
        let run = validate(&report, &candidates, BaselinePolicy::Required).unwrap();
        let mut base = baseline(&run);
        let name = base.floors.keys().next().unwrap().clone();
        let count = base.floors[&name];
        base.floors
            .insert(name.clone(), count.checked_add(1).unwrap());
        assert_eq!(
            evaluate(&run, &base),
            vec![Failure::Collapse {
                key: name.clone(),
                floor: count + 1,
                viable: count
            }]
        );
        base.floors.insert(name.clone(), 0);
        assert_eq!(
            evaluate(&run, &base),
            vec![Failure::InvalidFloor { key: name.clone() }]
        );
        base.floors.remove(&name);
        assert_eq!(
            evaluate(&run, &base),
            vec![Failure::Unaccounted {
                key: name.clone(),
                viable: count
            }]
        );
        base.floors.insert(name.clone(), count);
        base.floors.insert("gone.rs::f".to_owned(), 2);
        assert_eq!(
            evaluate(&run, &base),
            vec![Failure::StaleFloor {
                key: "gone.rs::f".to_owned(),
                floor: 2
            }]
        );
        base.floors.remove("gone.rs::f");
        base.known_zero_viable.push(name.clone());
        assert_eq!(
            evaluate(&run, &base),
            vec![Failure::InvalidKnownZero { key: name }]
        );
    }
}
