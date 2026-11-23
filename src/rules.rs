use crate::policy::{Matcher, Policy};
use crate::{Boundary, ChangedLine, Evidence, Finding, FunctionAnalysis, Report};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("built-in rule regex")
}

pub(crate) fn is_test(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.split('.').next().unwrap_or(name);
    lower.split('/').any(|part| {
        [
            "test",
            "tests",
            "__tests__",
            "fixtures",
            "testdata",
            "__fixtures__",
        ]
        .contains(&part)
    }) || name.starts_with("test_")
        || lower.ends_with("_test.go")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || stem.ends_with("Test")
        || stem.ends_with("Tests")
        || stem.ends_with("CT")
}

fn comment(text: &str) -> bool {
    ["//", "#", "--", "/*", "*"]
        .iter()
        .any(|s| text.trim().starts_with(s))
}

fn config_key(line: &ChangedLine, key_value: &Regex) -> Option<String> {
    let captures = key_value.captures(&line.text)?;
    let key = captures[1].to_ascii_lowercase();
    if key != "enabled" && key != "mode" && key != "dry-run" && key != "dry_run" {
        return Some(key);
    }
    // Recover nested YAML namespace using indentation, limited to this diff hunk.
    let indent = line.text.len() - line.text.trim_start().len();
    let mut parents: Vec<(usize, String)> = Vec::new();
    for text in line.context.lines() {
        if text == line.text {
            break;
        }
        let depth = text.len() - text.trim_start().len();
        if let Some(c) = key_value.captures(text) {
            while parents.last().is_some_and(|(i, _)| *i >= depth) {
                parents.pop();
            }
            if text.trim_end().ends_with(':') {
                parents.push((depth, c[1].to_ascii_lowercase()));
            }
        }
    }
    let mut path: Vec<_> = parents
        .into_iter()
        .filter(|(depth, _)| *depth < indent)
        .map(|(_, name)| name)
        .collect();
    path.push(key);
    Some(path.join("."))
}

/// Default policy excludes test/fixture files. All contextual findings remain review-only.
pub fn analyze(lines: &[ChangedLine]) -> Report {
    analyze_with_options(lines, false)
}

pub fn analyze_with_options(lines: &[ChangedLine], include_tests: bool) -> Report {
    analyze_with_policy(lines, include_tests, &Policy::default())
}

pub fn analyze_with_policy(lines: &[ChangedLine], include_tests: bool, policy: &Policy) -> Report {
    let matcher = Matcher::new(policy);
    let destructive = regex(
        r"(?i)\b(drop\s+(table|database|schema|column)\b|truncate\s+(table\s+)?[\w\x22`]|alter\s+table\b.*\bdrop\b)",
    );
    let mutation = regex(r"(?i)\b(update\s+[\w\x22`]+\s+set\b|delete\s+from\b)");
    let idempotency = regex(r"(?i)\b(idempotenc\w*|dedup\w*|idempotent\w*)\b");
    let calls = regex(
        r"(?P<receiver>(?:[A-Za-z_$][\w$]*\s*\.\s*)*[A-Za-z_$][\w$]*)\s*\.\s*(?P<method>[A-Za-z_$][\w$]*)\s*\(",
    );
    let write_config = regex(r"(?i)(write[_.-]?mode|dry[_.-]?run)$");
    let write_component = regex(r"(?i)(writer|repository|persistence|storage)");
    let replacement = regex(r"(?i)\.(removeIf|removeAll|retainAll|retain|filter|filterNot)\s*\(");
    let payload = regex(
        r#"(?:\.set[A-Z]\w*\s*\(|\.(?:put|set)\s*\(\s*"[A-Za-z_]\w*"\s*,|\b(?:record|row|entity|event|value|payload|sample|extra)\s*(?:\[\s*["'][\w-]+["']\s*\]|\.\w+)\s*=[^=])"#,
    );
    let guard = regex(r"\b(if|else|return|switch|case|when)\b");
    let key_value = regex(r#"^\s*["']?([A-Za-z_][\w.-]*)["']?\s*[:=]"#);
    let effect_config = regex(r"(?i)(publish|producer|outbox|cdc|change|event|queue|stream|topic)");
    let config_setting =
        regex(r"(?i)(enabled|destination|topic|binding|queue|write[_.-]?mode|dry[_.-]?run)$");
    let mut findings: Vec<Finding> = Vec::new();
    let mut groups = BTreeMap::new();
    let mut boundaries = BTreeMap::new();
    let mut skipped = BTreeSet::new();
    let old_text: BTreeSet<_> = lines
        .iter()
        .filter(|l| l.side == "old")
        .map(|l| (&l.path, l.hunk, l.text.trim()))
        .collect();
    let new_text: BTreeSet<_> = lines
        .iter()
        .filter(|l| l.side == "new")
        .map(|l| (&l.path, l.hunk, l.text.trim()))
        .collect();
    for line in lines {
        let ext = line
            .path
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let config = ["yaml", "yml", "properties", "toml", "json", "env"].contains(&ext.as_str())
            || line
                .path
                .rsplit('/')
                .next()
                .is_some_and(|n| n.starts_with(".env."));
        if (!include_tests && is_test(&line.path))
            || (!config
                && ![
                    "sql", "py", "go", "java", "kt", "kts", "rs", "js", "jsx", "ts", "tsx", "rb",
                    "php", "cs", "scala", "sh", "proto",
                ]
                .contains(&ext.as_str()))
        {
            skipped.insert(line.path.clone());
            continue;
        }
        let text = line.text.trim();
        let identity = (&line.path, line.hunk, text);
        if comment(text) || (old_text.contains(&identity) && new_text.contains(&identity)) {
            continue;
        }
        let mut emit = |id: &'static str,
                        severity: &'static str,
                        group: String,
                        risk: &'static str,
                        recovery: &'static str| {
            let key = (line.path.clone(), id, group);
            let evidence = Evidence {
                line: line.line,
                side: line.side,
                text: line.text.clone(),
            };
            if let Some(&index) = groups.get(&key) {
                let finding: &mut Finding = &mut findings[index];
                if finding.side == "old" && line.side == "new" {
                    finding.line = line.line;
                    finding.side = line.side;
                    finding.evidence = line.text.clone();
                }
                finding.related_evidence.push(evidence);
                index
            } else {
                let index = findings.len();
                groups.insert(key, index);
                findings.push(Finding {
                    rule_id: id,
                    severity,
                    path: line.path.clone(),
                    line: line.line,
                    side: line.side,
                    evidence: line.text.clone(),
                    risk,
                    recovery_evidence: recovery,
                    related_evidence: vec![evidence],
                    boundary: None,
                    function_name: None,
                });
                index
            }
        };
        let hunk = line.hunk.to_string();
        if config {
            if let Some(key) = config_key(line, &key_value)
                && config_setting.is_match(&key)
                && (effect_config.is_match(&key) || write_config.is_match(&key))
            {
                emit(
                    "RRL009",
                    "review",
                    format!("{hunk}:{key}"),
                    "Changed publication/routing or write-mode configuration may enable durable effects or leave queued work after code rollback.",
                    "Verify the owning feature gate, effective deployment configuration, destination, consumer idempotency and config/in-flight-work rollback.",
                );
            }
            continue;
        }
        if line.side == "new" && destructive.is_match(text) {
            emit(
                "RRL001",
                "high",
                hunk.clone(),
                "Destructive SQL may remove persisted data that a code rollback cannot restore.",
                "Show a tested restore, retained original data, or an expand/contract rollout plan.",
            );
        }
        if line.side == "new" && mutation.is_match(text) {
            emit(
                "RRL002",
                "review",
                hunk.clone(),
                "SQL mutation may rewrite or remove existing data, including during a migration.",
                "Verify affected rows, predicates, original-value retention and a tested repair procedure.",
            );
        }
        if idempotency.is_match(text) {
            emit(
                "RRL003",
                "review",
                hunk.clone(),
                "Changing deduplication or idempotency may repeat effects across retries or versions.",
                "Show retry/replay and mixed-version tests plus a way to identify and repair duplicates.",
            );
        }
        for call in calls.captures_iter(text) {
            let receiver: String = call["receiver"]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let method = &call["method"];
            let Some(effect) = matcher.classify_in(&line.path, &receiver, method, &line.context)
            else {
                continue;
            };
            let index = emit(
                effect.kind.rule(),
                "review",
                format!("{hunk}:{receiver}:{method}"),
                "A changed candidate effect call may leave persistent or external effects after code rollback; inspect the declared or inferred boundary.",
                "Confirm the receiver's actual effect, write identity/scope and values, retry behavior, compensation and repair procedure.",
            );
            let boundary = Boundary {
                source: effect.source,
                kind: effect.kind.as_str(),
                name: effect.name,
                receiver: receiver.clone(),
                method: method.into(),
                line: line.line,
                side: line.side,
                text: line.text.clone(),
            };
            if line.side == "new" || !boundaries.contains_key(&index) {
                boundaries.insert(index, boundary);
            }
        }
        // These bounded context heuristics intentionally do not claim interprocedural data flow.
        let writes = write_component.is_match(&line.path);
        if writes && replacement.is_match(text) && !text.contains(".entrySet()") {
            emit(
                "RRL007",
                "review",
                "file".into(),
                "Changed record selection in a write-oriented component may alter which historical records are replaced, even with an unchanged sink.",
                "Trace this predicate to the actual write; verify interval/identity boundaries, replay, retained originals and a repair/recalculation plan.",
            );
        }
        if writes
            && (payload.is_match(text) || (guard.is_match(text) && payload.is_match(&line.context)))
        {
            emit(
                "RRL008",
                "review",
                "file".into(),
                "Changed payload values or eligibility in a write-oriented component may persist incorrect data that code rollback does not repair.",
                "Trace changed values/guards to the write, check old/new readers and authoritative/shadow behavior, and provide replay or repair evidence.",
            );
        }
        if ext == "proto" && line.side == "old" && text.contains('=') {
            emit(
                "RRL006",
                "review",
                line.line.to_string(),
                "A removed or changed protobuf field/enum declaration may break stored or queued messages.",
                "Verify old/new readers and writers, field-number reservation and queued data compatibility.",
            );
        }
    }
    for (index, boundary) in boundaries {
        findings[index].boundary = Some(boundary);
    }
    Report {
        policy_source: "default",
        function_analysis: FunctionAnalysis { mode: "patch_only", warnings: vec!["Function-level analysis requires base/head revisions; only supplied hunks were scanned.".into()], ..Default::default() },
        schema_version: 1,
        findings,
        skipped_files: skipped.into_iter().collect(),
        limitation: "Review candidates, not proof of rollback safety. Repository policy is declarative, not runtime/type verification. Revision-mode Java/Python/Go function comparison associates changes with candidate effects; it is not interprocedural data-flow analysis. Other languages and patch-only input receive hunk checks. Unknown boundaries, upstream helpers, syntax gaps and most schema formats can be missed.",
    }
}
