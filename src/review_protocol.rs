use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The full canonical packet is echoed, avoiding weak or truncated identifiers.
pub fn packet(base: &str, head: &str, policy: Value, scan: Value) -> Value {
    let mut items = vec![
        json!({"id":"coverage","task":"Review the complete diff and downstream effects beyond scanner coverage; record checks and limits."}),
    ];
    for (field, prefix) in [("findings", "finding"), ("skipped_files", "skip")] {
        if let Some(values) = scan[field].as_array() {
            for (i, value) in values.iter().enumerate() {
                items.push(json!({"id":format!("{prefix}:{i}"),"observation":value}));
            }
        }
    }
    if let Some(warnings) = scan["function_analysis"]["warnings"].as_array() {
        for (i, value) in warnings.iter().enumerate() {
            items.push(json!({"id":format!("gap:{i}"),"observation":value}));
        }
    }
    json!({"protocol_version":1,"base_sha":base,"head_sha":head,"policy":policy,"scan":scan,"items":items})
}

/// Validates agent attestations, not their truth or the repository's merge policy.
pub fn verdict(packet: &Value, review: &Value) -> Value {
    let result = |status, reasons: Vec<String>| json!({"verdict":status,"head_sha":packet["head_sha"],"reasons":reasons,"limitation":"Evidence is an agent attestation, not verified proof. ALLOW covers this review only; repository checks and exact-head merge authorization are still required."});
    if review.get("packet") != Some(packet) {
        return result("INCOMPLETE",vec!["Review packet is missing or stale; regenerate and review the current head/policy/scan.".into()]);
    }
    let mut resolutions = BTreeMap::new();
    if let Some(rows) = review["resolutions"].as_array() {
        for row in rows {
            let Some(id) = row["id"].as_str() else {
                return result("INCOMPLETE", vec!["Resolution lacks an id".into()]);
            };
            if resolutions.insert(id, row).is_some() {
                return result("INCOMPLETE", vec![format!("Duplicate resolution: {id}")]);
            }
        }
    }
    let mut pending = Vec::new();
    let mut blocked = Vec::new();
    let Some(items) = packet["items"].as_array() else {
        return result("INCOMPLETE", vec!["Invalid packet".into()]);
    };
    for row in items {
        let id = row["id"].as_str().unwrap_or("");
        let Some(resolution) = resolutions.remove(id) else {
            pending.push(format!("Missing assessment: {id}"));
            continue;
        };
        let evidence = resolution["evidence"].as_array().is_some_and(|e| {
            !e.is_empty()
                && e.iter()
                    .all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
        });
        match resolution["status"].as_str() {
            Some("blocked") => blocked.push(format!("Unresolved risk: {id}")),
            Some("verified" | "false_positive" | "mitigated") if evidence => {}
            _ => pending.push(format!("Evidence or valid disposition missing: {id}")),
        }
    }
    for id in resolutions.keys() {
        pending.push(format!("Unknown assessment: {id}"));
    }
    if !blocked.is_empty() {
        result("BLOCK", blocked)
    } else if !pending.is_empty() {
        result("INCOMPLETE", pending)
    } else {
        result("ALLOW", Vec::new())
    }
}
