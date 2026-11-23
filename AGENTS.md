# Contributing to rollback-risk-lint

This guide applies to developers and coding agents working on the linter.
The tool helps review agents investigate effects that can survive a code rollback.
Read [README.md](README.md) for usage and development commands, and
[the agent review protocol](docs/agent-review.md) before changing review decisions.

## Design constraints

- Keep analysis independent of the target application's build system and runtime.
  Never execute its application code, Git diff drivers or textconv filters to scan it.
- Treat heuristic findings as review signals. Neither an empty scan nor a completed
  review proves rollback safety. Keep observations, inferred risks and evidence
  limitations distinct in output and documentation.
- Keep scanning and receipt validation separate from merge execution. `ALLOW`
  completes this review stage; it does not grant repository merge authorization
  or replace required checks, other reviews or branch protection.

## Compatibility and review focus

- Preserve merge-base diff semantics, trusted base-revision policy loading and
  old-side locations for removed code. A target change must not disable its own
  checks by editing its policy or supplying a review receipt.
- Preserve documented exit codes and JSON contracts. Review protocol changes must
  retain freshness checks and require complete-diff coverage even with no findings.
- Evidence in a receipt is a trusted review agent's attestation, not verified proof.
  Do not automatically resolve findings with canned evidence or accept PR-authored
  receipts as authoritative.

## Validation

Add regression cases for changed detection rules, diff parsing and review decisions.
Use synthetic fixtures that exercise both useful findings and false positives.
Before committing, run `cargo fmt --check`,
`cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`.
For CLI or packaging changes, also run the native smoke test described by CI.

## Evaluation privacy

Keep repository-specific evaluations, source patches, PR/MR identifiers and raw
findings under ignored `docs/evaluations/` or outside this repository. Do not copy
other projects' source, secrets or internal identifiers into tracked fixtures,
documentation, release notes, packages or CI logs. Shared examples must be synthetic;
release packages contain only the executable, generic README and license.
