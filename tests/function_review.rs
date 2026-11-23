use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Repo {
    path: PathBuf,
    base: String,
}
impl Repo {
    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    fn new(path: &str, source: &str, policy: Option<Value>) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rrl-functions-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let mut repo = Self {
            path: dir,
            base: String::new(),
        };
        repo.git(&["init", "-b", "main"]);
        repo.git(&["config", "user.name", "Test"]);
        repo.git(&["config", "user.email", "test@example.invalid"]);
        fs::write(repo.path.join(path), source).unwrap();
        if let Some(policy) = policy {
            repo.policy(policy);
        }
        repo.commit();
        repo.base = repo.git(&["rev-parse", "HEAD"]);
        repo
    }
    fn policy(&self, policy: Value) {
        fs::write(
            self.path.join(".rollback-risk-lint.json"),
            policy.to_string(),
        )
        .unwrap();
    }
    fn commit(&self) {
        self.git(&["add", "."]);
        self.git(&["commit", "--allow-empty", "-m", "fixture"]);
    }
    fn run(&self, flags: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_rollback-risk-lint"))
            .args([
                "--repo",
                self.path.to_str().unwrap(),
                "--base",
                &self.base,
                "--head",
                "HEAD",
                "--format",
                "json",
                "--fail-on",
                "never",
            ])
            .args(flags)
            .output()
            .unwrap()
    }
    fn report(&self) -> Value {
        let p = self.run(&[]);
        assert!(p.status.success(), "{}", String::from_utf8_lossy(&p.stderr));
        serde_json::from_slice(&p.stdout).unwrap()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}
fn policy(method: &str) -> Value {
    json!({"version":1,"effects":[{"name":"order-store","receiver":"gateway","method":method,"kind":"persistence"}]})
}
fn function_findings(report: &Value) -> Vec<&Value> {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule_id"] == "RRL010")
        .collect()
}

#[test]
fn unchanged_configured_sink_is_visible_beyond_patch_context_in_three_languages() {
    for (path, old, method) in [
        (
            "Worker.java",
            "class Worker { void process(Order order) {\norder.amount = 1;\nlog.info(\"a\");\nlog.info(\"b\");\nlog.info(\"c\");\nlog.info(\"d\");\nlog.info(\"e\");\ngateway.replace(order);\n} }\n",
            "replace",
        ),
        (
            "worker.py",
            "def process(order):\n    order['amount'] = 1\n    pass\n    pass\n    pass\n    pass\n    pass\n    gateway.replace(order)\n",
            "replace",
        ),
        (
            "worker.go",
            "package worker\nfunc process(order *Order) {\norder.Amount = 1\n_ = order\n_ = order\n_ = order\n_ = order\n_ = order\ngateway.Replace(order)\n}\n",
            "Replace",
        ),
    ] {
        let repo = Repo::new(path, old, Some(policy(method)));
        fs::write(repo.path.join(path), old.replace("= 1", "= 2")).unwrap();
        repo.commit();
        let report = repo.report();
        let findings = function_findings(&report);
        assert_eq!(findings.len(), 1, "{path}: {report}");
        assert_eq!(findings[0]["boundary"]["source"], "configured");
        assert!(
            findings[0]["function_name"]
                .as_str()
                .unwrap()
                .contains("process")
        );
        assert!(
            findings[0]["boundary"]["line"].as_u64().unwrap()
                > findings[0]["line"].as_u64().unwrap() + 3
        );
        assert_eq!(report["policy_source"], "base");
        assert!(report["function_analysis"]["functions"].as_u64().unwrap() > 0);
    }
}

#[test]
fn unchanged_heuristic_sink_is_labeled_as_a_guess() {
    let old = "def process(order):\n    order.amount = 1\n    pass\n    pass\n    pass\n    pass\n    repository.save(order)\n";
    let repo = Repo::new("worker.py", old, None);
    fs::write(repo.path.join("worker.py"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    let report = repo.report();
    let findings = function_findings(&report);
    assert_eq!(findings.len(), 1, "{report}");
    assert_eq!(findings[0]["boundary"]["source"], "heuristic");
}

#[test]
fn a_pure_declaration_suppresses_direct_and_function_sink_guesses() {
    let old = "def process(order):\n    order.amount = 1\n    cache.save(order)\n";
    let repo = Repo::new(
        "worker.py",
        old,
        Some(json!({"version":1,"pure":[{"receiver":"cache","method":"save"}]})),
    );
    fs::write(
        repo.path.join("worker.py"),
        old.replace("= 1", "= 2")
            .replace("cache.save(order)", "cache.save(order.copy())"),
    )
    .unwrap();
    repo.commit();
    assert!(repo.report()["findings"].as_array().unwrap().is_empty());
}

#[test]
fn a_configured_custom_call_is_detected_when_the_call_itself_changes() {
    let old = "def process(order):\n    gateway.replace(order)\n";
    let repo = Repo::new("worker.py", old, Some(policy("replace")));
    fs::write(
        repo.path.join("worker.py"),
        old.replace("replace(order)", "replace(order.copy())"),
    )
    .unwrap();
    repo.commit();
    let report = repo.report();
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["boundary"]["source"] == "configured"),
        "{report}"
    );
}

#[test]
fn unrelated_function_changes_do_not_borrow_a_sibling_functions_sink() {
    let old =
        "def normalize(order):\n    return 1\n\ndef process(order):\n    repository.save(order)\n";
    let repo = Repo::new("worker.py", old, None);
    fs::write(
        repo.path.join("worker.py"),
        old.replace("return 1", "return 2"),
    )
    .unwrap();
    repo.commit();
    assert!(function_findings(&repo.report()).is_empty());
}

#[test]
fn comments_and_logging_only_changes_do_not_create_function_risks() {
    let old = "def process(order):\n    # comment\n    logger.info('before')\n    repository.save(order)\n";
    let repo = Repo::new("worker.py", old, None);
    fs::write(
        repo.path.join("worker.py"),
        old.replace("# comment", "# changed comment")
            .replace("'before'", "'after'"),
    )
    .unwrap();
    repo.commit();
    assert!(function_findings(&repo.report()).is_empty());
}

#[test]
fn call_names_inside_strings_are_not_function_effects() {
    let old = "def process(order):\n    order.amount = 1\n    text = 'repository.save(order)'\n    return text\n";
    let repo = Repo::new("worker.py", old, None);
    fs::write(repo.path.join("worker.py"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    assert!(function_findings(&repo.report()).is_empty());
}

#[test]
fn a_pull_request_cannot_disable_its_own_base_policy() {
    let old = "def process(order):\n    order.amount = 1\n    gateway.replace(order)\n";
    let repo = Repo::new("worker.py", old, Some(policy("replace")));
    repo.policy(json!({"version":1,"pure":[{"receiver":"gateway","method":"replace"}]}));
    fs::write(repo.path.join("worker.py"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    assert_eq!(function_findings(&repo.report()).len(), 1);
}

#[test]
fn invalid_policy_is_an_error_even_in_report_only_mode() {
    let repo = Repo::new("worker.py", "x = 1\n", Some(json!({"version":99})));
    fs::write(repo.path.join("worker.py"), "x = 2\n").unwrap();
    repo.commit();
    let output = repo.run(&[]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn explicit_policy_can_override_base_policy() {
    let old = "def process(order):\n    order.amount = 1\n    gateway.replace(order)\n";
    let repo = Repo::new("worker.py", old, None);
    let file = repo.path.join("trusted.json");
    fs::write(&file, policy("replace").to_string()).unwrap();
    fs::write(repo.path.join("worker.py"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    let output = repo.run(&["--config", file.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["policy_source"], "explicit");
    assert_eq!(function_findings(&report).len(), 1);
}

#[test]
fn syntax_errors_are_reported_as_function_analysis_gaps() {
    let old = "def process(order):\n    order.amount = 1\n    repository.save(order)\n";
    let repo = Repo::new("worker.py", old, None);
    fs::write(
        repo.path.join("worker.py"),
        "def process(:\n    order.amount = 2\n",
    )
    .unwrap();
    repo.commit();
    let report = repo.report();
    assert!(
        !report["function_analysis"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
