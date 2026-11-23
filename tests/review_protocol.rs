use rollback_risk_lint::review_protocol::{packet, verdict};
use serde_json::json;
#[test]
fn no_findings_is_not_automatic_permission_to_merge() {
    let p=packet("base","head",json!({}),json!({"findings":[],"skipped_files":[],"function_analysis":{"warnings":[]}}));
    assert_eq!(verdict(&p,&json!({}))["verdict"],"INCOMPLETE");
    let review=json!({"packet":p,"resolutions":[{"id":"coverage","status":"verified","evidence":["Inspected complete changed-file set and downstream write paths"]}]});
    assert_eq!(verdict(&p,&review)["verdict"],"ALLOW");
    let stale=packet("base","different-head",json!({}),json!({"findings":[]}));
    assert_eq!(verdict(&stale,&review)["verdict"],"INCOMPLETE");
}
#[test]
fn unresolved_risks_block_and_missing_evidence_never_allows() {
    let p=packet("base","head",json!({}),json!({"findings":[{"risk":"write"}],"skipped_files":["schema.json"],"function_analysis":{"warnings":["parser gap"]}}));
    let mut r=json!({"packet":p,"resolutions":[{"id":"finding:0","status":"blocked","evidence":["Repair path missing"]}]});
    assert_eq!(verdict(&p,&r)["verdict"],"BLOCK");
    r["resolutions"][0]["status"]=json!("verified");
    assert_eq!(verdict(&p,&r)["verdict"],"INCOMPLETE");
}
