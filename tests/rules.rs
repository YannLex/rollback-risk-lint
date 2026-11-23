use rollback_risk_lint::{analyze, parse_diff};

fn added(path: &str, code: &str) -> String {
    let count = code.lines().count();
    format!(
        "diff --git a/{path} b/{path}\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{count} @@\n{}",
        code.lines().map(|s| format!("+{s}\n")).collect::<String>()
    )
}

#[test]
fn destructive_sql_is_high_with_recovery_evidence() {
    for sql in [
        "DROP TABLE users;",
        "ALTER TABLE users DROP COLUMN email;",
        "TRUNCATE users;",
    ] {
        let report = analyze(&parse_diff(&added("migrations/002.sql", sql)).unwrap());
        let f = &report.findings[0];
        assert_eq!(
            (f.rule_id, f.severity, f.line, f.side),
            ("RRL001", "high", 1, "new")
        );
        assert!(f.recovery_evidence.contains("restore"));
    }
}

#[test]
fn sql_inside_application_code_is_detected() {
    let report =
        analyze(&parse_diff(&added("cleanup.py", "db.execute('DROP TABLE users')")).unwrap());
    assert!(report.findings.iter().any(|f| f.rule_id == "RRL001"));
}

#[test]
fn cross_language_persistence_and_outbound_signals() {
    for (path, code, id) in [
        ("worker.py", "session.save(order)", "RRL004"),
        ("worker.go", "producer.Send(message)", "RRL005"),
        ("Worker.java", "repository.saveAll(orders);", "RRL004"),
        ("worker.rs", "client.post(url);", "RRL005"),
        ("worker.ts", "bus.publish(event);", "RRL005"),
    ] {
        let report = analyze(&parse_diff(&added(path, code)).unwrap());
        assert!(report.findings.iter().any(|f| f.rule_id == id), "{path}");
    }
}

#[test]
fn removed_idempotency_and_proto_fields_are_reviewed_on_old_side() {
    let patch = "diff --git a/event.proto b/event.proto\n--- a/event.proto\n+++ b/event.proto\n@@ -7,2 +7,0 @@\n-string idempotency_key = 1;\n-int64 amount = 2;\n";
    let report = analyze(&parse_diff(patch).unwrap());
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|f| f.rule_id == "RRL006")
            .count(),
        2
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.rule_id == "RRL003" && f.side == "old" && f.line == 7)
    );
}

#[test]
fn context_and_removed_sql_do_not_trigger_destructive_rule() {
    let patch = "diff --git a/a.sql b/a.sql\n--- a/a.sql\n+++ b/a.sql\n@@ -10,2 +10,2 @@\n DROP TABLE historical;\n-DROP TABLE users;\n+SELECT 1;\n";
    assert!(analyze(&parse_diff(patch).unwrap()).findings.is_empty());
}

#[test]
fn comments_and_docs_do_not_trigger_rules() {
    let mut patch = added("demo.py", "# db.delete(users)\nprint('ok')");
    patch.push_str(&added("README.md", "DROP TABLE users;"));
    let report = analyze(&parse_diff(&patch).unwrap());
    assert!(report.findings.is_empty());
    assert_eq!(report.skipped_files, ["README.md"]);
}

#[test]
fn sql_mutations_request_review_even_with_where_clause() {
    for code in [
        "UPDATE users SET credit = 0 WHERE id = 1;",
        "DELETE FROM users WHERE active = false;",
    ] {
        let report = analyze(&parse_diff(&added("migration.sql", code)).unwrap());
        assert_eq!(report.findings[0].rule_id, "RRL002");
        assert_eq!(report.findings[0].severity, "review");
    }
}

#[test]
fn line_numbers_follow_multiple_hunks_and_source_header_like_text() {
    let patch = "diff --git a/a.py b/a.py\n--- a/a.py\n+++ b/a.py\n@@ -1 +1,2 @@\n x\n+++value\n@@ -20 +21 @@\n-old\n+db.save(order)\n\\ No newline at end of file\n";
    let lines = parse_diff(patch).unwrap();
    assert_eq!(lines[0].text, "++value");
    assert_eq!(lines[2].line, 21);
    assert_eq!(analyze(&lines).findings[0].line, 21);
}

#[test]
fn quoted_utf8_and_space_paths() {
    let patch = "diff --git a/x b/x\n--- /dev/null\n+++ \"b/\\344\\270\\255 name.py\"\n@@ -0,0 +1 @@\n+db.save(x)\n";
    assert_eq!(parse_diff(patch).unwrap()[0].path, "中 name.py");
    assert_eq!(
        parse_diff(&added("space name.py", "db.save(x)")).unwrap()[0].path,
        "space name.py"
    );
}

#[test]
fn truncated_malformed_and_unsupported_patches_fail() {
    for patch in [
        "not a diff",
        "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1,2 @@\n-a\n+b\n",
        "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -0,0 +1 @@\n context\n",
        "diff --git a/x b/x\nBinary files a/x and b/x differ\n",
        "diff --git a/x b/x\nGIT binary patch\n",
        "diff --cc x\n",
        "diff --git a/x b/x\nindex abc..def 160000\n",
    ] {
        assert!(parse_diff(patch).is_err(), "{patch}");
    }
}

#[test]
fn empty_and_rename_only_diffs_are_valid() {
    assert!(parse_diff("").unwrap().is_empty());
    assert!(
        parse_diff(
            "diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n"
        )
        .unwrap()
        .is_empty()
    );
}
