//! A20 in todo.md: incomplete or invalid mutation reports must fail closed.
#![allow(clippy::expect_used, reason = "audit regression process assertions")]
use std::path::Path;
use std::process::Command;

fn assert_rejected(case: &str, reason: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/audit_cases")
        .join(case);
    let output = Command::new(env!("CARGO_BIN_EXE_mutants-gate"))
        .arg("check")
        .arg(&dir)
        .arg(dir.join("baseline.json"))
        .output()
        .expect("run mutation gate");
    assert!(
        !output.status.success(),
        "gate accepted invalid report: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(reason), "unexpected rejection: {stderr}");
}

#[test]
fn mutant_failure_is_rejected() {
    assert_rejected("failure", "invalid mutant summary: Failure");
}

#[test]
fn duplicate_outcomes_cannot_replace_missing_candidates() {
    assert_rejected("duplicate", "duplicate candidate outcome");
}

#[test]
fn cli_checks_and_seeding_require_complete_clean_runs() {
    let candidates: serde_json::Value =
        serde_json::from_str(include_str!("audit_cases/real-candidates-27.1.0.json"))
            .expect("candidate fixture");
    let rows = candidates.as_array().expect("candidate array");
    for summary in [
        "CaughtMutant",
        "Unviable",
        "MissedMutant",
        "Timeout",
        "Success",
        "Failure",
    ] {
        let dir = tempfile::tempdir().expect("temp directory");
        let mut outcomes = vec![serde_json::json!({"summary":"Success","scenario":"Baseline"})];
        outcomes.extend(
            rows.iter()
                .map(|row| serde_json::json!({"summary":summary,"scenario":{"Mutant":row}})),
        );
        std::fs::write(dir.path().join("mutants.json"), candidates.to_string())
            .expect("write candidates");
        std::fs::write(
            dir.path().join("outcomes.json"),
            serde_json::json!({"outcomes":outcomes}).to_string(),
        )
        .expect("write outcomes");
        let seed = Command::new(env!("CARGO_BIN_EXE_mutants-gate"))
            .arg("emit-baseline")
            .arg(dir.path())
            .output()
            .expect("seed command");
        let clean = matches!(summary, "CaughtMutant" | "Unviable");
        assert_eq!(
            seed.status.code(),
            Some(i32::from(!clean)),
            "{summary}: {}",
            String::from_utf8_lossy(&seed.stderr)
        );
        if clean {
            let baseline: serde_json::Value =
                serde_json::from_slice(&seed.stdout).expect("valid baseline JSON");
            std::fs::write(dir.path().join("baseline.json"), baseline.to_string())
                .expect("write baseline");
            let check = Command::new(env!("CARGO_BIN_EXE_mutants-gate"))
                .arg("check")
                .arg(dir.path())
                .arg(dir.path().join("baseline.json"))
                .output()
                .expect("check command");
            assert_eq!(
                check.status.code(),
                Some(0),
                "{summary}: {}",
                String::from_utf8_lossy(&check.stderr)
            );
        } else {
            assert_eq!(seed.stdout, Vec::<u8>::new());
        }
        outcomes.remove(0);
        std::fs::write(
            dir.path().join("outcomes.json"),
            serde_json::json!({"outcomes":outcomes}).to_string(),
        )
        .expect("write skipped report");
        let required = Command::new(env!("CARGO_BIN_EXE_mutants-gate"))
            .arg("emit-baseline")
            .arg(dir.path())
            .output()
            .expect("required baseline command");
        assert_eq!(required.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&required.stderr).contains("expected one baseline"));
        let skipped = Command::new(env!("CARGO_BIN_EXE_mutants-gate"))
            .arg("emit-baseline")
            .arg(dir.path())
            .arg("--skip-baseline")
            .output()
            .expect("skipped baseline command");
        assert_eq!(skipped.status.code(), Some(i32::from(!clean)));
    }
}
