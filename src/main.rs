use clap::{Parser, ValueEnum};
use rollback_risk_lint::{
    FileVersions, analyze_with_policy, parse_diff, policy::Policy, review_functions, supported_path,
};
use std::collections::BTreeSet;
use std::{
    fs,
    io::{self, Read},
    path::PathBuf,
    process::{Command, ExitCode},
};

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}
#[derive(Clone, Copy, ValueEnum)]
enum FailOn {
    High,
    Review,
    Never,
}

#[derive(Parser)]
#[command(
    version,
    about = "Find changes whose effects may outlive a service rollback"
)]
struct Args {
    /// Git-style unified patch file, or - for stdin
    #[arg(long, conflicts_with_all = ["base", "head", "repo"])]
    diff: Option<PathBuf>,
    /// Base revision. Scans merge-base(BASE, HEAD)..HEAD, like a PR/MR
    #[arg(long, requires = "head")]
    base: Option<String>,
    /// Head revision (requires --base)
    #[arg(long, requires = "base")]
    head: Option<String>,
    /// Git checkout used for revision comparisons
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
    /// Trusted JSON policy file; otherwise revision mode uses the base-revision policy
    #[arg(long)]
    config: Option<PathBuf>,
    /// Include test and fixture files (excluded by default)
    #[arg(long)]
    include_tests: bool,
    /// Exit 1 for high findings (default), any review signal, or never
    #[arg(long, value_enum, default_value = "high")]
    fail_on: FailOn,
}

fn git(args: &Args, command: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(&args.repo)
        .args(command)
        .output()
        .map_err(|e| format!("Cannot run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| "Non-UTF-8 diff/revision output is unsupported".into())
}

fn blob(args: &Args, revision: &str, path: &str) -> Result<String, String> {
    let exists = git(args, &["ls-tree", "--name-only", revision, "--", path])?;
    if exists.is_empty() {
        return Ok(String::new());
    }
    git(args, &["show", &format!("{revision}:{path}")])
}

fn run(args: Args) -> Result<bool, String> {
    let mut revisions = None;
    let patch = if let Some(path) = &args.diff {
        if path.as_os_str() == "-" {
            let mut s = String::new();
            io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| e.to_string())?;
            s
        } else {
            fs::read_to_string(path).map_err(|e| e.to_string())?
        }
    } else if let (Some(base), Some(head)) = (&args.base, &args.head) {
        // Resolve refs before diffing: no shell interpolation or option/revision-range injection.
        let resolve = |reference: &str| {
            git(
                &args,
                &[
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                    &format!("{reference}^{{commit}}"),
                ],
            )
        };
        let base = resolve(base)?;
        let head = resolve(head)?;
        let ancestor = git(&args, &["merge-base", base.trim(), head.trim()])?;
        revisions = Some((
            ancestor.trim().to_owned(),
            head.trim().to_owned(),
            base.trim().to_owned(),
        ));
        git(
            &args,
            &[
                "-c",
                "core.quotePath=true",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--unified=3",
                ancestor.trim(),
                head.trim(),
                "--",
            ],
        )?
    } else {
        return Err("Supply --diff FILE (or -), or both --base REF --head REF".into());
    };
    let (policy, policy_source) = if let Some(path) = &args.config {
        (
            Policy::parse(
                &fs::read_to_string(path).map_err(|e| format!("Cannot read policy: {e}"))?,
            )?,
            "explicit",
        )
    } else if let Some((_, _, base)) = &revisions {
        let exists = git(
            &args,
            &[
                "ls-tree",
                "--name-only",
                base,
                "--",
                ".rollback-risk-lint.json",
            ],
        )?;
        let text = blob(&args, base, ".rollback-risk-lint.json")?;
        if exists.is_empty() {
            (Policy::default(), "default")
        } else {
            (Policy::parse(&text)?, "base")
        }
    } else {
        (Policy::default(), "default")
    };
    let lines = parse_diff(&patch)?;
    let mut report = analyze_with_policy(&lines, args.include_tests, &policy);
    report.policy_source = policy_source;
    if let Some((base, head, _)) = &revisions {
        let paths: BTreeSet<_> = lines
            .iter()
            .map(|l| &l.path)
            .filter(|p| supported_path(p, args.include_tests))
            .collect();
        let mut files = Vec::new();
        for path in paths {
            files.push(FileVersions {
                path: path.clone(),
                before: blob(&args, base, path)?,
                after: blob(&args, head, path)?,
            });
        }
        review_functions(&mut report, &lines, &files, &policy);
    }
    match args.format {
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        ),
        Format::Text => {
            for f in &report.findings {
                println!(
                    "{} {} {}:{} ({})\n  {}\n  Evidence: {}\n  Needed: {}",
                    f.severity,
                    f.rule_id,
                    f.path,
                    f.line,
                    f.side,
                    f.risk,
                    f.evidence.trim(),
                    f.recovery_evidence
                );
            }
            for f in &report.findings {
                if let Some(boundary) = &f.boundary {
                    println!(
                        "  Boundary: {} {} {}:{} ({}) {}.{}",
                        boundary.source,
                        boundary.kind,
                        f.path,
                        boundary.line,
                        boundary.side,
                        boundary.receiver,
                        boundary.method
                    );
                }
                if let Some(name) = &f.function_name {
                    println!("  Function: {name}");
                }
            }
            for warning in &report.function_analysis.warnings {
                eprintln!("Analysis gap: {warning}");
            }
            println!(
                "{} finding(s); {} file(s) skipped.\n{}",
                report.findings.len(),
                report.skipped_files.len(),
                report.limitation
            );
            for path in &report.skipped_files {
                println!("Skipped: {path}");
            }
        }
    }
    Ok(report.findings.iter().any(|f| match args.fail_on {
        FailOn::High => f.severity == "high",
        FailOn::Review => true,
        FailOn::Never => false,
    }))
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(false) => ExitCode::SUCCESS,
        Ok(true) => ExitCode::from(1),
        Err(e) => {
            eprintln!("rollback-risk-lint: {e}");
            ExitCode::from(2)
        }
    }
}
