use rollback_risk_lint::{analyze, parse_diff};

fn patch(path: &str, old: &[&str], new: &[&str], context: &[&str]) -> String {
    let mut body = String::new();
    for line in old {
        body.push_str(&format!("-{line}\n"));
    }
    for line in new {
        body.push_str(&format!("+{line}\n"));
    }
    for line in context {
        body.push_str(&format!(" {line}\n"));
    }
    format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -10,{} +10,{} @@\n{body}",
        old.len() + context.len(),
        new.len() + context.len()
    )
}

fn ids(patch: &str) -> Vec<&'static str> {
    analyze(&parse_diff(patch).unwrap())
        .findings
        .iter()
        .map(|f| f.rule_id)
        .collect()
}

#[test]
fn test_and_fixture_sources_are_excluded_by_default() {
    for path in [
        "src/test/java/OrderTest.java",
        "tests/test_orders.py",
        "orders_test.go",
        "src/order.test.ts",
        "fixtures/orders.sql",
        "src/OrderCT.java",
        "testdata/migration.sql",
    ] {
        let report = analyze(
            &parse_diff(&patch(
                path,
                &[],
                &["db.save(order); DROP TABLE users;"],
                &[],
            ))
            .unwrap(),
        );
        assert!(report.findings.is_empty(), "{path}");
        assert_eq!(report.skipped_files, [path]);
    }
}

#[test]
fn ambiguous_local_operations_do_not_become_effects() {
    for code in [
        "crc.update(content);",
        "MDC.put(key, value);",
        "MDC.remove(key);",
        "rows.put(key, value);",
        "token.put(bytes);",
        "captor.capture();",
        "diagnostics.capture(change);",
        "context.getBaggage().remove(key);",
    ] {
        assert!(
            ids(&patch("processor.java", &[], &[code], &[])).is_empty(),
            "{code}"
        );
    }
}

#[test]
fn actual_persistence_http_payment_and_message_calls_remain_visible() {
    for (path, code, id) in [
        ("order.py", "session.save(order)", "RRL004"),
        ("order.go", "repository.Update(order)", "RRL004"),
        ("Order.java", "dao.saveAllToTable(orders);", "RRL004"),
        ("order.rs", "client.put(url);", "RRL005"),
        ("order.ts", "payments.capture(payment);", "RRL005"),
        ("worker.go", "producer.Send(message)", "RRL005"),
    ] {
        assert!(ids(&patch(path, &[], &[code], &[])).contains(&id), "{code}");
    }
}

#[test]
fn pairs_of_old_and_new_sink_lines_are_one_finding() {
    let p = patch(
        "order.py",
        &["repository.save(old_order)"],
        &["repository.save(new_order)"],
        &[],
    );
    let report = analyze(&parse_diff(&p).unwrap());
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].side, "new");
}

#[test]
fn storage_update_is_not_suppressed_just_because_local_updates_are() {
    assert!(ids(&patch("worker.py", &[], &["db.update(record)"], &[])).contains(&"RRL004"));
}

#[test]
fn replacement_predicate_changes_in_write_components_are_reviewed() {
    for path in [
        "src/OrderWriter.java",
        "order_writer.py",
        "storage/order_writer.go",
    ] {
        let p = patch(
            path,
            &["records.removeIf(row -> row.end >= start);"],
            &["records.removeIf(row -> row.start >= start);"],
            &["return records;"],
        );
        assert!(ids(&p).contains(&"RRL007"), "{path}");
    }
}

#[test]
fn filter_changes_outside_write_components_are_not_replacement_alerts() {
    let p = patch(
        "ReportView.java",
        &["records.removeIf(row -> row.end >= start);"],
        &["records.removeIf(row -> row.start >= start);"],
        &[],
    );
    assert!(!ids(&p).contains(&"RRL007"));
}

#[test]
fn changed_values_in_write_components_are_reviewed_without_a_changed_sink() {
    for (path, old, new) in [
        (
            "OrderWriter.java",
            "value.setStatus(oldStatus);",
            "value.setStatus(newStatus);",
        ),
        (
            "order_writer.py",
            "record[\"status\"] = old_status",
            "record[\"status\"] = new_status",
        ),
        (
            "order_writer.go",
            "record.Status = oldStatus",
            "record.Status = newStatus",
        ),
    ] {
        assert!(
            ids(&patch(path, &[old], &[new], &["return record;"])).contains(&"RRL008"),
            "{path}"
        );
    }
}

#[test]
fn changed_guard_near_unchanged_payload_assignment_is_reviewed() {
    let p = patch(
        "OrderWriter.java",
        &["if (eligible && hasToken) {"],
        &["if (eligible) {"],
        &["    extra.put(\"needsNotification\", true);", "}"],
    );
    assert!(ids(&p).contains(&"RRL008"));
}

#[test]
fn unrelated_import_or_logging_changes_in_writer_do_not_trigger_payload_rule() {
    for code in [
        "import java.time.Clock;",
        "log.info(\"completed\");",
        "cache.put(key, value);",
    ] {
        assert!(!ids(&patch("OrderWriter.java", &[], &[code], &[])).contains(&"RRL008"));
    }
}

#[test]
fn production_configuration_enabling_publication_is_visible() {
    for (path, old, new) in [
        (
            "helm/prod.yaml",
            "  EVENT_PUBLISHER_ENABLED: \"false\"",
            "  EVENT_PUBLISHER_ENABLED: \"true\"",
        ),
        (
            "app.properties",
            "outbox.enabled=false",
            "outbox.enabled=true",
        ),
        (
            "app.toml",
            "EVENT_STREAM_ENABLED = false",
            "EVENT_STREAM_ENABLED = true",
        ),
        (
            ".env",
            "CHANGE_FEED_ENABLED=false",
            "CHANGE_FEED_ENABLED=true",
        ),
        (
            "app.json",
            "\"publisher_enabled\": false,",
            "\"publisher_enabled\": true,",
        ),
    ] {
        assert!(
            ids(&patch(path, &[old], &[new], &[])).contains(&"RRL009"),
            "{path}"
        );
    }
}

#[test]
fn nested_yaml_configuration_uses_parent_context() {
    let p = "diff --git a/app.yml b/app.yml\n--- a/app.yml\n+++ b/app.yml\n@@ -1,2 +1,2 @@\n publisher:\n-  enabled: false\n+  enabled: true\n";
    assert!(ids(p).contains(&"RRL009"));
}

#[test]
fn write_modes_and_destination_changes_are_reviewed() {
    for (old, new) in [
        ("WRITE_MODE: shadow", "WRITE_MODE: authoritative"),
        ("OUTPUT_TOPIC: previous", "OUTPUT_TOPIC: next"),
        ("DRY_RUN: true", "DRY_RUN: false"),
    ] {
        assert!(ids(&patch("prod.yml", &[old], &[new], &[])).contains(&"RRL009"));
    }
}

#[test]
fn cache_logging_and_unrelated_config_changes_are_not_publication_alerts() {
    for (old, new) in [
        ("CACHE_ENABLED: false", "CACHE_ENABLED: true"),
        ("LOGGING_ENABLED: false", "LOGGING_ENABLED: true"),
        ("replicas: 2", "replicas: 3"),
    ] {
        assert!(ids(&patch("prod.yml", &[old], &[new], &[])).is_empty());
    }
}

#[test]
fn receiver_tokens_and_known_local_types_do_not_become_external_effects() {
    for code in ["requested.put(key, value);", "forest.put(key, value);"] {
        assert!(ids(&patch("processor.java", &[], &[code], &[])).is_empty());
    }
    let p = patch(
        "processor.java",
        &[],
        &["publisher.put(key, value);"],
        &["Map<String, String> publisher = new HashMap<>();"],
    );
    assert!(ids(&p).is_empty());
}

#[test]
fn camel_case_and_acronym_receiver_tokens_keep_real_sinks() {
    for code in [
        "streamBridge.send(binding, message);",
        "watchScoreWriter.publish(record);",
        "httpClient.put(url);",
        "HTTPClient.post(url);",
    ] {
        assert!(
            ids(&patch("processor.java", &[], &[code], &[])).contains(&"RRL005"),
            "{code}"
        );
    }
}

#[test]
fn changed_receiver_type_uses_the_correct_diff_side() {
    let p = patch(
        "worker.java",
        &[
            "Map<String, String> client = new HashMap<>();",
            "client.put(key, value);",
        ],
        &[
            "HttpClient client = buildClient();",
            "client.put(url, payload);",
        ],
        &[],
    );
    assert!(ids(&p).contains(&"RRL005"));
}
