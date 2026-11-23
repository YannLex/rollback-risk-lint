use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_REPO: AtomicU64 = AtomicU64::new(0);
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
            "rrl-functions-{}-{}-{}",
            NEXT_REPO.fetch_add(1, Ordering::Relaxed),
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

#[test]
fn latest_target_branch_policy_is_used_after_branches_diverge() {
    let old = "def process(order):\n    order.amount = 1\n    gateway.replace(order)\n";
    let mut repo = Repo::new("worker.py", old, None);
    repo.git(&["checkout", "-b", "feature"]);
    fs::write(repo.path.join("worker.py"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    repo.git(&["checkout", "main"]);
    repo.policy(policy("replace"));
    repo.commit();
    repo.base = repo.git(&["rev-parse", "HEAD"]);
    repo.git(&["checkout", "feature"]);
    assert_eq!(function_findings(&repo.report()).len(), 1);
}

#[test]
fn empty_or_misspelled_base_policy_is_not_silently_ignored() {
    for text in ["", "{\"version\":1,\"efffects\":[]}"] {
        let mut repo = Repo::new("worker.py", "x = 1\n", None);
        fs::write(repo.path.join(".rollback-risk-lint.json"), text).unwrap();
        repo.commit();
        repo.base = repo.git(&["rev-parse", "HEAD"]);
        fs::write(repo.path.join("worker.py"), "x = 2\n").unwrap();
        repo.commit();
        assert_eq!(repo.run(&[]).status.code(), Some(2));
    }
}

#[test]
fn a_known_local_map_is_not_a_function_effect() {
    let old = "class Worker { void process(Order order) {\nMap<String, String> client = new HashMap<>();\norder.amount = 1;\nclient.put(key, value);\n} }\n";
    let repo = Repo::new("Worker.java", old, None);
    fs::write(repo.path.join("Worker.java"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    assert!(function_findings(&repo.report()).is_empty());
}

#[test]
fn string_whitespace_is_data_while_docstrings_are_ignored() {
    let old = "def process(order):\n    payload = ' '\n    repository.save(payload)\n";
    let repo = Repo::new("worker.py", old, None);
    fs::write(repo.path.join("worker.py"), old.replace("' '", "''")).unwrap();
    repo.commit();
    assert_eq!(function_findings(&repo.report()).len(), 1);
    let old = "def process(order):\n    'documentation'\n    repository.save(order)\n";
    let repo = Repo::new("worker.py", old, None);
    fs::write(
        repo.path.join("worker.py"),
        old.replace("documentation", "updated documentation"),
    )
    .unwrap();
    repo.commit();
    assert!(function_findings(&repo.report()).is_empty());
}

#[test]
fn declared_free_functions_and_multiline_calls_are_supported() {
    let old = "def process(order):\n    order.amount = 1\n    persist(\n        order\n    )\n";
    let repo = Repo::new(
        "worker.py",
        old,
        Some(
            json!({"version":1,"effects":[{"name":"free-write","receiver":"","method":"persist","kind":"persistence"}]}),
        ),
    );
    fs::write(repo.path.join("worker.py"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    assert_eq!(function_findings(&repo.report()).len(), 1);
}

#[test]
fn overloads_with_distinct_parameter_types_are_compared() {
    let old = "class Worker {\nvoid process(Order order) { order.amount = 1; repository.save(order); }\nvoid process(Other order) { repository.save(order); }\n}\n";
    let repo = Repo::new("Worker.java", old, None);
    fs::write(repo.path.join("Worker.java"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    let report = repo.report();
    assert_eq!(function_findings(&report).len(), 1);
    assert!(
        report["function_analysis"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| !v.as_str().unwrap().contains("ambiguous"))
    );
}

#[test]
fn wildcard_rules_are_path_scoped_and_effects_override_pure_rules() {
    let old = "def process(order):\n    order.amount = 1\n    custom.commit(order)\n";
    let repo = Repo::new(
        "worker.py",
        old,
        Some(
            json!({"version":1,"effects":[{"name":"write","receiver":"*","method":"commit","kind":"message","path_suffix":"worker.py"}],"pure":[{"receiver":"custom","method":"commit"}]}),
        ),
    );
    fs::write(repo.path.join("worker.py"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    let report = repo.report();
    assert_eq!(function_findings(&report).len(), 1);
    assert_eq!(function_findings(&report)[0]["boundary"]["kind"], "message");
}

#[test]
fn deleted_declared_free_call_keeps_old_side_evidence() {
    let old = "def process(order):\n    persist(order)\n";
    let repo = Repo::new(
        "worker.py",
        old,
        Some(
            json!({"version":1,"effects":[{"name":"free-write","receiver":"","method":"persist","kind":"persistence"}]}),
        ),
    );
    fs::write(
        repo.path.join("worker.py"),
        "def process(order):\n    return order\n",
    )
    .unwrap();
    repo.commit();
    let report = repo.report();
    assert!(
        report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["side"] == "old" && f["boundary"]["source"] == "configured")
    );
}

#[test]
fn configured_call_names_in_changed_strings_are_not_declared_effects() {
    let old = "def process(order):\n    text = 'gateway.replace(order)'\n    return text\n";
    let repo = Repo::new("worker.py", old, Some(policy("replace")));
    fs::write(
        repo.path.join("worker.py"),
        old.replace("replace(order)", "replace(order.copy())"),
    )
    .unwrap();
    repo.commit();
    assert!(repo.report()["findings"].as_array().unwrap().is_empty());
}

#[test]
fn syntactically_duplicate_declarations_are_not_paired_arbitrarily() {
    let old = "class Worker {\nvoid process(Order first) { first.amount = 1; repository.save(first); }\nvoid process(Order second) { repository.save(second); }\n}\n";
    let repo = Repo::new("Worker.java", old, None);
    fs::write(repo.path.join("Worker.java"), old.replace("= 1", "= 2")).unwrap();
    repo.commit();
    let report = repo.report();
    assert!(function_findings(&report).is_empty());
    assert!(
        !report["function_analysis"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn effect_calls_nested_in_logging_or_interpolation_are_not_hidden() {
    for wrapper in [
        "logger.info('written', gateway.replace(order))",
        "f'{gateway.replace(order)}'",
    ] {
        let old = format!("def process(order):\n    order.amount = 1\n    {wrapper}\n");
        let repo = Repo::new("worker.py", &old, Some(policy("replace")));
        fs::write(repo.path.join("worker.py"), old.replace("= 1", "= 2")).unwrap();
        repo.commit();
        assert_eq!(function_findings(&repo.report()).len(), 1, "{wrapper}");
    }
}

#[test]
fn agent_review_cli_recomputes_packet_and_rejects_changed_heads() {
    let repo = Repo::new("worker.py", "x = 1\n", None);
    fs::write(repo.path.join("worker.py"), "x = 2\n").unwrap();
    repo.commit();
    let output = repo.run(&["--agent-review"]);
    assert!(output.status.success());
    let packet: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(packet["head_sha"], repo.git(&["rev-parse", "HEAD"]));
    let receipt = repo.path.join("receipt.json");
    fs::write(&receipt,json!({"packet":packet,"resolutions":[{"id":"coverage","status":"verified","evidence":["Reviewed all changes and downstream effects"]}]}).to_string()).unwrap();
    let flags = ["--agent-review", "--review", receipt.to_str().unwrap()];
    let allowed = repo.run(&flags);
    assert_eq!(allowed.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<Value>(&allowed.stdout).unwrap()["verdict"],
        "ALLOW"
    );
    fs::write(repo.path.join("worker.py"), "x = 3\n").unwrap();
    repo.git(&["commit", "-am", "new head"]);
    let stale = repo.run(&flags);
    assert_eq!(stale.status.code(), Some(1));
    assert_eq!(
        serde_json::from_slice::<Value>(&stale.stdout).unwrap()["verdict"],
        "INCOMPLETE"
    );
}
