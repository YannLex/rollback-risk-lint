use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

fn cli(patch: &str, flags: &[&str]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rollback-risk-lint"))
        .args(["--diff", "-"])
        .args(flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

const HIGH: &str =
    "diff --git a/a.sql b/a.sql\n--- /dev/null\n+++ b/a.sql\n@@ -0,0 +1 @@\n+DROP TABLE users;\n";
const REVIEW: &str =
    "diff --git a/a.py b/a.py\n--- /dev/null\n+++ b/a.py\n@@ -0,0 +1 @@\n+db.save(order)\n";

#[test]
fn exit_codes_and_json_contract() {
    let output = cli(HIGH, &["--format", "json"]);
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["findings"][0]["rule_id"], "RRL001");
    assert_eq!(cli(REVIEW, &[]).status.code(), Some(0));
    assert_eq!(cli(REVIEW, &["--fail-on", "review"]).status.code(), Some(1));
    assert_eq!(cli(HIGH, &["--fail-on", "never"]).status.code(), Some(0));
    assert_eq!(
        cli("bad patch", &["--fail-on", "never"]).status.code(),
        Some(2)
    );
    assert_eq!(cli("", &[]).status.code(), Some(0));
}

#[test]
fn conflicting_inputs_are_rejected() {
    assert_eq!(
        cli("", &["--base", "main", "--head", "HEAD"]).status.code(),
        Some(2)
    );
}

#[test]
fn revision_mode_scans_merge_base_and_handles_spaces() {
    let path = std::env::temp_dir().join(format!(
        "rollback-risk-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "user.name", "Test"]);
    fs::write(path.join("base.sql"), "SELECT 1;\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "base"]);
    git(&["checkout", "-b", "feature"]);
    fs::write(path.join("space name.sql"), "DROP TABLE users;\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "feature"]);
    let head = git(&["rev-parse", "HEAD"]);
    git(&["checkout", "main"]);
    fs::write(path.join("base.sql"), "DROP TABLE unrelated;\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "diverged base"]);
    let out = Command::new(env!("CARGO_BIN_EXE_rollback-risk-lint"))
        .args([
            "--repo",
            path.to_str().unwrap(),
            "--base",
            "main",
            "--head",
            &head,
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["findings"].as_array().unwrap().len(), 1);
    assert_eq!(report["findings"][0]["path"], "space name.sql");
    let bad = Command::new(env!("CARGO_BIN_EXE_rollback-risk-lint"))
        .args([
            "--repo",
            path.to_str().unwrap(),
            "--base=--output=oops",
            "--head",
            &head,
        ])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(!path.join("oops").exists());
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn include_tests_is_an_explicit_opt_in() {
    let patch = HIGH.replace("a.sql", "tests/fixture.sql");
    assert_eq!(cli(&patch, &[]).status.code(), Some(0));
    assert_eq!(cli(&patch, &["--include-tests"]).status.code(), Some(1));
    let changed = "diff --git a/order.py b/order.py\n--- a/order.py\n+++ b/order.py\n@@ -1 +1 @@\n-repository.save(old)\n+repository.save(new)\n";
    let output = cli(changed, &["--format", "json", "--fail-on", "review"]);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["findings"].as_array().unwrap().len(), 1);
    assert_eq!(
        report["findings"][0]["related_evidence"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn patch_file_input_and_missing_file_return_expected_exit_codes() {
    let path = std::env::temp_dir().join(format!(
        "rollback-risk-input-{}-{}.patch",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, HIGH).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_rollback-risk-lint"))
            .args(["--diff", path.to_str().unwrap()])
            .output()
            .unwrap()
    };
    assert_eq!(run().status.code(), Some(1));
    fs::remove_file(&path).unwrap();
    assert_eq!(run().status.code(), Some(2));
    assert_eq!(
        Command::new(env!("CARGO_BIN_EXE_rollback-risk-lint"))
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}
