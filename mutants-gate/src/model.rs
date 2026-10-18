//! Identity fields from cargo-mutants 27.1.0; display names and diffs are derived.
use std::collections::BTreeMap;
use std::num::NonZeroU64;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct Report {
    pub(crate) outcomes: Vec<Outcome>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Outcome {
    pub(crate) summary: Summary,
    pub(crate) scenario: Scenario,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) enum Summary {
    Success,
    CaughtMutant,
    MissedMutant,
    Unviable,
    Timeout,
    Failure,
}

#[derive(Debug, Deserialize)]
pub(crate) enum Scenario {
    Baseline,
    Mutant(MutantInfo),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub(crate) struct MutantInfo {
    pub(crate) package: String,
    pub(crate) file: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub(crate) function: Option<FunctionInfo>,
    pub(crate) span: Span,
    pub(crate) replacement: String,
    pub(crate) genre: Genre,
}

pub(crate) type Candidate = MutantInfo;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub(crate) struct FunctionInfo {
    pub(crate) function_name: String,
    pub(crate) return_type: String,
    pub(crate) span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub(crate) struct Span {
    pub(crate) start: Position,
    pub(crate) end: Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub(crate) struct Position {
    pub(crate) line: NonZeroU64,
    pub(crate) column: NonZeroU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub(crate) enum Genre {
    FnValue,
    BinaryOperator,
    UnaryOperator,
    MatchArm,
    MatchArmGuard,
    StructField,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Baseline {
    pub(crate) floors: BTreeMap<String, usize>,
    pub(crate) known_zero_viable: Vec<String>,
}
