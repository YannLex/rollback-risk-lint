use rollback_risk_lint::review_protocol::{packet, verdict};
use serde_json::json;
#[test]
fn no_findings_is_not_automatic_permission_to_merge() {
    let p = packet(
        "base",
        "head",
        json!({}),
        json!({"findings":[],"skipped_files":[],"function_analysis":{"warnings":[]}}),
    );
    assert_eq!(verdict(&p, &json!({}))["verdict"], "INCOMPLETE");
    let review = json!({"packet":p,"resolutions":[{"id":"coverage","status":"verified","evidence":["Inspected complete changed-file set and downstream write paths"]}]});
    assert_eq!(verdict(&p, &review)["verdict"], "ALLOW");
    let stale = packet("base", "different-head", json!({}), json!({"findings":[]}));
    assert_eq!(verdict(&stale, &review)["verdict"], "INCOMPLETE");
}
#[test]
fn unresolved_risks_block_and_missing_evidence_never_allows() {
    let p = packet(
        "base",
        "head",
        json!({}),
        json!({"findings":[{"risk":"write"}],"skipped_files":["schema.json"],"function_analysis":{"warnings":["parser gap"]}}),
    );
    let mut r = json!({"packet":p,"resolutions":[{"id":"finding:0","status":"blocked","evidence":["Repair path missing"]}]});
    assert_eq!(verdict(&p, &r)["verdict"], "BLOCK");
    r["resolutions"][0]["status"] = json!("verified");
    assert_eq!(verdict(&p, &r)["verdict"], "INCOMPLETE");
}

#[test]
fn malformed_duplicate_unknown_and_blank_resolutions_cannot_allow() {
    let p = packet("base", "head", json!({}), json!({"findings":[]}));
    for resolutions in [
        json!([{"id":"coverage","status":"verified","evidence":[" "]}]),
        json!([{"id":"coverage","status":"verified","evidence":["checked"]},{"id":"coverage","status":"verified","evidence":["checked"]}]),
        json!([{"id":"coverage","status":"verified","evidence":["checked"]},{"id":"unknown","status":"verified","evidence":["checked"]}]),
        json!([{"status":"verified","evidence":["checked"]}]),
    ] {
        assert_eq!(
            verdict(&p, &json!({"packet":p,"resolutions":resolutions}))["verdict"],
            "INCOMPLETE"
        );
    }
    let changed_policy = packet("base", "head", json!({"version":2}), json!({"findings":[]}));
    assert_eq!(
        verdict(&changed_policy, &json!({"packet":p,"resolutions":[]}))["verdict"],
        "INCOMPLETE"
    );
}
