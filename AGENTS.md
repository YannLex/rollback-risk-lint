# Working on rollback-risk-lint

Keep rules independent of the target application's build system and runtime.
Call findings review signals unless the evidence supports a stronger claim.
No findings must never be presented as proof of rollback safety.

Add regression tests for new rules and diff parsing changes. Preserve exit codes:
0 completed below threshold, 1 findings at threshold, 2 input/tool failure.
Preserve old-side locations for removals and merge-base semantics for revision input.
Never execute the target repository's diff drivers, textconv or application code.

Before committing: cargo fmt --check, cargo clippy --locked --all-targets -- -D warnings,
and cargo test --locked. Do not introduce secrets or copy source from other projects.

Use the GitHub account identity for commits in this repository: YannLex
<3049310+YannLex@users.noreply.github.com>. Do not inherit another repository’s identity.

Repository-specific evaluation reports, MR identifiers, source patches and raw
findings must remain local under ignored `docs/evaluations/` or outside the repo.
Do not add them to tracked docs, release notes, packages or CI logs. Examples and
regression fixtures must be synthetic and contain no internal project identifiers.
