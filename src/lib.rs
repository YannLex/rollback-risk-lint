use regex::Regex;
use serde::Serialize;
use std::sync::Arc;

mod function_review;
pub mod policy;
mod rules;
mod syntax;
pub use function_review::{FileVersions, review_functions, supported_path};
pub use rules::{analyze, analyze_with_options, analyze_with_policy};

#[derive(Debug)]
pub struct ChangedLine {
    pub path: String,
    pub line: usize,
    pub side: &'static str,
    pub text: String,
    pub hunk: usize,
    pub context: Arc<str>,
}

/// Parse a Git-style unified diff. Reject truncated/unsupported input rather than pass it.
pub fn parse_diff(input: &str) -> Result<Vec<ChangedLine>, String> {
    let header = Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap();
    let mut result = Vec::new();
    let (mut old_path, mut new_path) = (None, None);
    let (mut old_line, mut new_line) = (0, 0);
    let (mut old_left, mut new_left) = (0, 0);
    let mut saw_file = false;
    let mut contexts = vec![(String::new(), String::new())];
    let mut hunk = 0;
    for raw in input.lines() {
        if raw == "\\ No newline at end of file" {
            continue;
        }
        // Hunk contents take precedence: source code can start with +++ or ---.
        if old_left > 0 || new_left > 0 {
            let (old, new, side) = match raw.as_bytes().first() {
                Some(b' ') => (1, 1, None),
                Some(b'-') => (1, 0, Some("old")),
                Some(b'+') => (0, 1, Some("new")),
                _ => return Err("Malformed or truncated diff hunk".into()),
            };
            if old > old_left || new > new_left {
                return Err("Diff hunk line counts do not match".into());
            }
            if old > 0 {
                contexts[hunk].0.push_str(&raw[1..]);
                contexts[hunk].0.push('\n');
            }
            if new > 0 {
                contexts[hunk].1.push_str(&raw[1..]);
                contexts[hunk].1.push('\n');
            }
            if let Some(side) = side {
                let path = if side == "new" { &new_path } else { &old_path };
                result.push(ChangedLine {
                    path: path.clone().ok_or("Missing file header")?,
                    line: if side == "new" { new_line } else { old_line },
                    side,
                    text: raw[1..].to_owned(),
                    hunk,
                    context: Arc::from(""),
                });
            }
            old_left -= old;
            new_left -= new;
            old_line = old_line
                .checked_add(old)
                .ok_or("Hunk line number overflow")?;
            new_line = new_line
                .checked_add(new)
                .ok_or("Hunk line number overflow")?;
            continue;
        }
        if raw.starts_with("diff --git ") {
            saw_file = true;
            old_path = None;
            new_path = None;
        } else if let Some(path) = raw.strip_prefix("--- ") {
            old_path = Some(decode_path(path, "a/")?);
        } else if let Some(path) = raw.strip_prefix("+++ ") {
            new_path = Some(decode_path(path, "b/")?);
        } else if let Some(c) = header.captures(raw) {
            hunk = contexts.len();
            let heading = format!("{raw}\n");
            contexts.push((heading.clone(), heading));
            if old_path.is_none() || new_path.is_none() {
                return Err("Missing file headers before hunk".into());
            }
            let number = |i: usize, default: usize| -> Result<usize, String> {
                c.get(i).map_or(Ok(default), |v| {
                    v.as_str().parse().map_err(|_| "Invalid hunk number".into())
                })
            };
            old_line = number(1, 0)?;
            new_line = number(3, 0)?;
            old_left = number(2, 1)?;
            new_left = number(4, 1)?;
        } else if raw.starts_with("Binary files ")
            || raw == "GIT binary patch"
            || raw.starts_with("diff --cc ")
            || raw.starts_with("diff --combined ")
            || raw.contains("160000") && (raw.starts_with("index ") || raw.contains(" mode "))
        {
            return Err(
                "Binary, submodule and combined diffs are unsupported; review separately".into(),
            );
        } else if raw.is_empty()
            || raw.starts_with("index ")
            || raw.starts_with("new file mode ")
            || raw.starts_with("deleted file mode ")
            || raw.starts_with("old mode ")
            || raw.starts_with("new mode ")
            || raw.starts_with("similarity index ")
            || raw.starts_with("dissimilarity index ")
            || raw.starts_with("rename from ")
            || raw.starts_with("rename to ")
            || raw.starts_with("copy from ")
            || raw.starts_with("copy to ")
        {
            continue;
        } else {
            return Err(format!("Unrecognized diff input: {raw}"));
        }
    }
    if old_left != 0 || new_left != 0 {
        return Err("Truncated diff hunk".into());
    }
    if !saw_file && !input.trim().is_empty() {
        return Err("Expected Git-style unified diff (git diff)".into());
    }
    let contexts: Vec<(Arc<str>, Arc<str>)> = contexts
        .into_iter()
        .map(|(old, new)| (Arc::from(old), Arc::from(new)))
        .collect();
    for line in &mut result {
        let (old, new) = &contexts[line.hunk];
        line.context = Arc::clone(if line.side == "old" { old } else { new });
    }
    Ok(result)
}

fn decode_path(raw: &str, prefix: &str) -> Result<String, String> {
    let raw = raw.split('\t').next().unwrap_or(raw);
    let decoded = if raw.starts_with('"') {
        if !raw.ends_with('"') || raw.len() < 2 {
            return Err("Unterminated quoted path".into());
        }
        let mut bytes = raw.as_bytes()[1..raw.len() - 1].iter().copied();
        let mut out = Vec::new();
        while let Some(b) = bytes.next() {
            if b != b'\\' {
                out.push(b);
                continue;
            }
            let escaped = bytes.next().ok_or("Invalid path escape")?;
            out.push(match escaped {
                b'n' => b'\n',
                b't' => b'\t',
                b'r' => b'\r',
                b'a' => 7,
                b'b' => 8,
                b'v' => 11,
                b'f' => 12,
                b'\\' => b'\\',
                b'"' => b'"',
                b'0'..=b'3' => {
                    let a = bytes.next().ok_or("Invalid octal path escape")?;
                    let b = bytes.next().ok_or("Invalid octal path escape")?;
                    if !(b'0'..=b'7').contains(&a) || !(b'0'..=b'7').contains(&b) {
                        return Err("Invalid octal path escape".into());
                    }
                    (escaped - b'0') * 64 + (a - b'0') * 8 + (b - b'0')
                }
                _ => return Err("Unsupported path escape".into()),
            });
        }
        String::from_utf8(out).map_err(|_| "Non-UTF-8 paths are unsupported")?
    } else {
        raw.to_owned()
    };
    Ok(decoded.strip_prefix(prefix).unwrap_or(&decoded).to_owned())
}

#[derive(Debug, Serialize)]
pub struct Finding {
    pub rule_id: &'static str,
    pub severity: &'static str,
    pub path: String,
    pub line: usize,
    pub side: &'static str,
    pub evidence: String,
    pub risk: &'static str,
    pub recovery_evidence: &'static str,
    pub related_evidence: Vec<Evidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boundary: Option<Boundary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Evidence {
    pub line: usize,
    pub side: &'static str,
    pub text: String,
}

#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub findings: Vec<Finding>,
    pub skipped_files: Vec<String>,
    pub limitation: &'static str,
    pub policy_source: &'static str,
    pub function_analysis: FunctionAnalysis,
}

#[derive(Debug, Serialize)]
pub struct Boundary {
    pub source: &'static str,
    pub kind: &'static str,
    pub name: Option<String>,
    pub receiver: String,
    pub method: String,
    pub line: usize,
    pub side: &'static str,
    pub text: String,
}

#[derive(Debug, Default, Serialize)]
pub struct FunctionAnalysis {
    pub mode: &'static str,
    pub files: usize,
    pub functions: usize,
    pub warnings: Vec<String>,
}
