use crate::{
    Boundary, ChangedLine, Evidence, Finding, FunctionAnalysis, Report,
    policy::{Matcher, Policy},
    syntax,
};
use std::collections::{BTreeMap, BTreeSet};

pub struct FileVersions {
    pub path: String,
    pub before: String,
    pub after: String,
}

/// Compare syntactic functions. This reports contextual association, not proven data flow.
pub fn review_functions(
    report: &mut Report,
    lines: &[ChangedLine],
    files: &[FileVersions],
    policy: &Policy,
) {
    report.function_analysis = FunctionAnalysis {
        mode: "revision",
        ..Default::default()
    };
    let matcher = Matcher::new(policy);
    for file in files {
        let (before, after) = match (
            syntax::functions(&file.path, &file.before),
            syntax::functions(&file.path, &file.after),
        ) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                report.function_analysis.warnings.push(format!(
                    "{}: {}",
                    file.path,
                    a.err().or(b.err()).unwrap()
                ));
                continue;
            }
        };
        report.function_analysis.files += 1;
        // Replace configured lexical candidates inside parsed functions with real call nodes.
        report.findings.retain(|finding| {
            if finding.path != file.path
                || !finding
                    .boundary
                    .as_ref()
                    .is_some_and(|b| b.source == "configured")
            {
                return true;
            }
            let functions = if finding.side == "old" {
                &before
            } else {
                &after
            };
            !functions
                .iter()
                .any(|f| finding.line >= f.start && finding.line <= f.end)
        });
        direct_changes(report, lines, file, &before, &after, &matcher);

        let mut old = BTreeMap::new();
        let mut duplicates = BTreeSet::new();
        for f in &before {
            if old.insert(&f.name, f).is_some() {
                duplicates.insert(&f.name);
            }
        }
        let mut seen = BTreeSet::new();
        for f in &after {
            if !seen.insert(&f.name) {
                duplicates.insert(&f.name);
            }
        }
        for f in &after {
            if duplicates.contains(&f.name) {
                let message = format!(
                    "{}: ambiguous function {}; overloaded functions were not paired",
                    file.path, f.name
                );
                if !report.function_analysis.warnings.contains(&message) {
                    report.function_analysis.warnings.push(message);
                }
                continue;
            }
            let Some(previous) = old.get(&f.name) else {
                continue;
            };
            report.function_analysis.functions += 1;
            if previous.tokens == f.tokens {
                continue;
            }
            let changed: Vec<_> = lines
                .iter()
                .filter(|l| {
                    l.path == file.path
                        && ((l.side == "new" && l.line >= f.start && l.line <= f.end)
                            || (l.side == "old"
                                && l.line >= previous.start
                                && l.line <= previous.end))
                })
                .collect();
            let Some(line) = changed
                .iter()
                .find(|l| l.side == "new")
                .or_else(|| changed.first())
            else {
                continue;
            };
            let mut reported = BTreeSet::new();
            for call in &f.calls {
                let Some(effect) =
                    matcher.classify_in(&file.path, &call.receiver, &call.method, &f.text)
                else {
                    continue;
                };
                if !previous.calls.iter().any(|old| {
                    old.receiver == call.receiver
                        && old.method == call.method
                        && old.tokens == call.tokens
                }) {
                    continue;
                }
                if !reported.insert((&call.receiver, &call.method)) {
                    continue;
                }
                report.findings.push(Finding{rule_id:"RRL010",severity:"review",path:file.path.clone(),line:line.line,side:line.side,evidence:line.text.clone(),risk:"A changed function contains an unchanged candidate effect call. Input, selection or control changes may alter persistent/external effects; this association does not prove data flow.",recovery_evidence:"Trace changed identities, conditions, collections and payloads to this boundary; check old/new behavior, replay and repair or compensation.",related_evidence:changed.iter().map(|l|Evidence{line:l.line,side:l.side,text:l.text.clone()}).collect(),function_name:Some(f.name.clone()),boundary:Some(Boundary{source:effect.source,kind:effect.kind.as_str(),name:effect.name,receiver:call.receiver.clone(),method:call.method.clone(),line:call.line,side:"new",text:call.text.clone()})});
            }
        }
    }
}

fn direct_changes(
    report: &mut Report,
    lines: &[ChangedLine],
    file: &FileVersions,
    before: &[syntax::Function],
    after: &[syntax::Function],
    matcher: &Matcher<'_>,
) {
    for (functions, other, side) in [(before, after, "old"), (after, before, "new")] {
        for f in functions {
            for call in &f.calls {
                let Some(effect) =
                    matcher.classify_in(&file.path, &call.receiver, &call.method, &f.text)
                else {
                    continue;
                };
                if effect.source != "configured" {
                    continue;
                }
                if other.iter().filter(|o| o.name == f.name).any(|o| {
                    o.calls.iter().any(|c| {
                        c.receiver == call.receiver
                            && c.method == call.method
                            && c.tokens == call.tokens
                    })
                }) {
                    continue;
                }
                let changed: Vec<_> = lines
                    .iter()
                    .filter(|l| {
                        l.path == file.path
                            && l.side == side
                            && l.line >= call.line
                            && l.line <= call.end
                    })
                    .collect();
                if changed.is_empty() {
                    continue;
                }
                let line = changed[0];
                let evidence = changed.iter().map(|l| Evidence {
                    line: l.line,
                    side: l.side,
                    text: l.text.clone(),
                });
                let boundary = Boundary {
                    source: effect.source,
                    kind: effect.kind.as_str(),
                    name: effect.name,
                    receiver: call.receiver.clone(),
                    method: call.method.clone(),
                    line: call.line,
                    side,
                    text: call.text.clone(),
                };
                if let Some(existing) = report.findings.iter_mut().find(|finding| {
                    finding.path == file.path
                        && finding.rule_id == effect.kind.rule()
                        && finding.function_name.as_ref() == Some(&f.name)
                        && finding.boundary.as_ref().is_some_and(|b| {
                            b.source == "configured"
                                && b.receiver == call.receiver
                                && b.method == call.method
                        })
                }) {
                    existing.related_evidence.extend(evidence);
                    if side == "new" {
                        existing.line = line.line;
                        existing.side = side;
                        existing.evidence = line.text.clone();
                        existing.boundary = Some(boundary);
                    }
                } else {
                    report.findings.push(Finding{rule_id:effect.kind.rule(),severity:"review",path:file.path.clone(),line:line.line,side,evidence:line.text.clone(),risk:"A repository-declared effect call changed; verify the durable or external impact.",recovery_evidence:"Check write scope/values, compatibility, retries and repair/compensation.",related_evidence:evidence.collect(),function_name:Some(f.name.clone()),boundary:Some(boundary)});
                }
            }
        }
    }
}

pub fn supported_path(path: &str, include_tests: bool) -> bool {
    syntax::supported(path) && (include_tests || !crate::rules::is_test(path))
}
