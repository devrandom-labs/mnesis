//! CI tool: turns a cargo-mutants run into a pass/fail verdict that cannot be
//! vacuously green. See `mutants-gate/README.md` and `.github/workflows/mutants.yml`.
//!
//! Usage:
//!   mutants-gate check <mutants.out-dir> <baseline.json>
//!   mutants-gate emit-baseline <mutants.out-dir>

#![allow(
    clippy::redundant_pub_crate,
    reason = "multi-module binary crate: pub(crate) documents the crate-internal API surface across sibling modules, the intent-revealing choice over bare pub"
)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "mutants-gate is a CLI tool: stdout carries the ratio report, stderr carries failure reasons"
)]

mod gate;
mod model;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gate::BaselinePolicy;
use model::{Baseline, Candidate, Report};

#[derive(Debug, thiserror::Error)]
enum GateError {
    #[error(
        "usage: mutants-gate <check <out-dir> <baseline.json> | emit-baseline <out-dir>> [--skip-baseline]"
    )]
    Usage,
    #[error("reading {}: {source}", .path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("parsing {}: {source}", .path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, GateError> {
    let raw = std::fs::read_to_string(path).map_err(|source| GateError::Io {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_str(&raw).map_err(|source| GateError::Parse {
        path: path.to_owned(),
        source,
    })
}

fn run() -> Result<bool, GateError> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
    let policy = if args.last().is_some_and(|arg| arg == "--skip-baseline") {
        args.pop();
        BaselinePolicy::Skipped
    } else {
        BaselinePolicy::Required
    };
    let (mode, directory_arg, baseline_path) = match args.as_slice() {
        [mode, directory] if mode == "emit-baseline" => (mode, directory, None),
        [mode, directory, baseline] if mode == "check" => (mode, directory, Some(baseline)),
        _ => return Err(GateError::Usage),
    };
    let directory = Path::new(directory_arg);
    let report: Report = read_json(&directory.join("outcomes.json"))?;
    let candidates: Vec<Candidate> = read_json(&directory.join("mutants.json"))?;
    let run = match gate::validate(&report, &candidates, policy) {
        Ok(run) => run,
        Err(error) => {
            eprintln!("mutants-gate FAIL: {error}");
            return Ok(false);
        }
    };
    match mode.as_str() {
        "emit-baseline" => {
            let text = match gate::emit_baseline(&run) {
                Ok(text) => text,
                Err(error) => {
                    eprintln!("mutants-gate FAIL: {error}");
                    return Ok(false);
                }
            };
            println!("{text}");
            Ok(true)
        }
        "check" => {
            let baseline: Baseline = read_json(Path::new(baseline_path.ok_or(GateError::Usage)?))?;
            let failures = gate::evaluate(&run, &baseline);
            print!("{}", gate::render_report(&run));
            for failure in &failures {
                eprintln!("mutants-gate FAIL: {failure}");
            }
            Ok(failures.is_empty())
        }
        _ => Err(GateError::Usage),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("mutants-gate: {err}");
            ExitCode::from(2)
        }
    }
}
