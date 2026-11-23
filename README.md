# rollback-risk-lint

A Rust CLI for reviewing changes whose effects may outlive a service rollback.
It reads Git diffs rather than depending on a Java, Python or Go build system.

Rolling back code does not undo incorrect writes, restore deleted data, cancel
external actions, or guarantee that an old version can read newly written data.
This tool surfaces evidence that deserves a recovery or compatibility review.
It does **not** certify rollback safety.

## Download a binary

Download a platform archive and its `.sha256` from the repository's
[Releases](https://github.com/YannLex/rollback-risk-lint/releases). Builds cover
Linux x86-64 (GNU, glibc 2.35+), macOS Apple Silicon, and Windows x86-64.
GitHub login is required while the repository is private. Release assets persist;
per-commit [Actions artifacts](https://github.com/YannLex/rollback-risk-lint/actions)
are retained for 30 days.

```sh
# Requires authenticated gh. Choose your platform's asset pattern.
gh release download v0.2.0 --repo YannLex/rollback-risk-lint --pattern '*aarch64-apple-darwin*'
shasum -a 256 -c rollback-risk-lint-aarch64-apple-darwin.tar.gz.sha256
tar -xzf rollback-risk-lint-aarch64-apple-darwin.tar.gz
./rollback-risk-lint-aarch64-apple-darwin/rollback-risk-lint --help
```

Linux users can use `sha256sum -c`; Windows users can compare `Get-FileHash`
with the checksum file and extract the zip. On macOS, local security settings
may require approval before running a downloaded, unsigned binary.

## Install and run

Requires Rust with edition 2024 support and Git for revision comparisons.

```sh
cargo install --path . --locked

# PR/MR scope: merge-base(BASE, HEAD)..HEAD. Fetch both refs and enough history first.
rollback-risk-lint --repo /path/to/project --base origin/main --head HEAD

# Git-style unified patches from either GitHub or GitLab; no platform token needed.
git diff --no-ext-diff --no-textconv origin/main...HEAD > change.patch
rollback-risk-lint --diff change.patch --format json

# stdin; use pipefail so an upstream Git error is not mistaken for an empty diff.
set -o pipefail
git diff --no-ext-diff --no-textconv origin/main...HEAD |
  rollback-risk-lint --diff - --fail-on review

# Included demonstration (exits 1 by design).
rollback-risk-lint --diff examples/risky.patch
```

Revision mode scans **committed** changes; it does not include uncommitted edits.
It resolves refs to commit IDs, disables external diff/textconv execution, and
requires a common ancestor. Missing refs/history produce an error, not a pass.
There is no direct PR/MR URL fetcher in this version: CI supplies refs or a patch.

## Rules

| ID | Severity | Signal | Review evidence |
| --- | --- | --- | --- |
| RRL001 | high | Added DROP TABLE/DATABASE/SCHEMA/COLUMN, TRUNCATE, ALTER TABLE … DROP | Restore rehearsal, retained originals, expand/contract rollout |
| RRL002 | review | Added SQL UPDATE … SET / DELETE FROM | Target rows, predicates, original values, repair procedure |
| RRL003 | review | Added/removed idempotency or deduplication identifiers | Retry/replay and mixed-version behavior, duplicate repair |
| RRL004 | review | Likely persistence calls, using receiver hints | Write identity, scope, values and repair procedure |
| RRL005 | review | Likely outbound/payment calls, using receiver hints | Destination, compensation, retries and queued/in-flight work |
| RRL006 | review | Removed/changed protobuf assignments | Reader/writer compatibility, field numbers, queued data |
| RRL007 | review | Record selection/replacement changes in write components | Trace to write, boundary/replay tests, retained originals, repair |
| RRL008 | review | Payload values or eligibility changes in write components | Old/new readers, write modes, replay/repair |
| RRL009 | review | Publication/routing/write-mode configuration changes | Effective config, owning gate, consumer idempotency, in-flight work |

These are **heuristic review signals**, not whole-program or type-resolved analysis.
The extension filter covers SQL, Python, Go, Java, Kotlin, Rust, JavaScript,
TypeScript, Ruby, PHP, C#, Scala, shell and protobuf. Configuration rules also
inspect YAML, properties, TOML, JSON and environment files.

Persistence/outbound calls use receiver-name tokens to distinguish likely effect
boundaries from ambiguous names such as `update`, `put`, `remove` or `capture`.
Known local declarations (maps, JSON nodes, byte buffers, CRC accumulators) in the
same hunk suppress sink-call alerts. This is bounded declaration recognition,
not actual type resolution: differently named real sinks can still be missed.

Replacement and payload rules inspect write-oriented component paths containing
`writer`, `repository`, `persistence` or `storage`. They flag changed selection,
setters, record-field assignments, or eligibility guards near payload assignments,
even when no sink line changes. They ask the reviewer to trace the actual write;
they do not prove that the local data reaches durable storage. Changes in upstream
orchestration, helpers outside these components, and unknown component names can
still escape detection. Configuration matching recognizes flat effect-related
keys and nested YAML namespaces visible within the hunk; it is not a complete
YAML/JSON/TOML interpreter or effective-deployment evaluator.

Tests and fixtures are **excluded by default**, including conventional test
directories, Go `_test.go`, Python `test_*`, JS/TS `.test`/`.spec`, and Java
`*Test`, `*Tests`, `*CT`. Use `--include-tests` to inspect them. Both excluded tests
and unsupported extensions appear in `skipped_files`. Formatting-only replacements
are ignored. Old/new evidence for the same sink in a hunk is grouped into one
finding; contextual replacement/payload signals are grouped by file.

Multiline statements, indirect calls, dynamic SQL, ORM-specific migrations,
storage units/semantics, most schema formats, and side effects with unknown
names can escape detection. Simple full-line comments are ignored; this is not
a comment/string parser. Mode-only changes and pure renames have no rule coverage.
Binary, submodule, combined, non-UTF-8 or malformed diffs fail with exit 2 and need
separate review. Empty diffs are valid. Input must be Git-style unified diff,
without mail-format wrappers. No findings does not establish rollback safety.

## Output and CI policy

Text is intended for reviewers. `--format json` returns a versioned object with
`schema_version`, `findings`, `skipped_files`, and `limitation`. Each finding carries
its rule ID, severity, path, line, diff side, literal evidence, risk and requested
recovery evidence. `related_evidence` retains grouped old/new locations and text.
The primary location prefers a new-side line when one exists. Removed lines use the **old** path/line; additions use the new.
Evidence may contain proprietary data: apply your repository's log access policy.

| Exit | Meaning |
| --- | --- |
| 0 | Completed without findings at the selected failure threshold |
| 1 | Findings reached the selected threshold |
| 2 | Invalid arguments, input, Git failure or unsupported patch |

Default `--fail-on high` gates destructive SQL. `--fail-on review` gates all signals;
`--fail-on never` reports findings without failing, but input errors still fail.
No inline suppression or automatically accepted recovery claim is provided.
A reviewer must evaluate recovery evidence; this tool cannot validate a plan by
checking that a paragraph exists.

For GitHub Actions, fetch full history (`actions/checkout` with `fetch-depth: 0`)
and run an installed, reviewed build of the linter:

```yaml
- name: Review rollback risk
  env:
    BASE_SHA: ${{ github.event.pull_request.base.sha }}
    HEAD_SHA: ${{ github.event.pull_request.head.sha }}
  run: rollback-risk-lint --base "$BASE_SHA" --head "$HEAD_SHA" --format json
```

Use this step on `pull_request` events. Make sure both commit objects are present.
Do not execute source code from the target PR just to analyze it. Provision the
linter from a trusted revision in your CI image or preceding installation step;
GitHub repository access is needed while this project is private.

For a GitLab merge request job with the linter already installed:

```yaml
rollback-risk:
  variables:
    GIT_DEPTH: "0"
  rules:
    - if: '$CI_PIPELINE_SOURCE == "merge_request_event"'
  script:
    - git fetch origin "$CI_MERGE_REQUEST_TARGET_BRANCH_NAME"
    - rollback-risk-lint --base "origin/$CI_MERGE_REQUEST_TARGET_BRANCH_NAME" --head "$CI_COMMIT_SHA" --format json
```

The repository's own CI checks formatting, Clippy and tests; the integration
snippets above show how to adopt the CLI in a separate application repository.

## Development

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Tests exercise cross-language signals, old/new line locations, UTF-8 and spaced
paths, malformed/unsupported input, exit thresholds, JSON output, safe revision
resolution, and merge-base behavior on a real diverged Git repository.

## Evaluation privacy

Keep repository-specific evaluations in `docs/evaluations/` (ignored by Git).
Do not commit internal project identifiers, source patches, MR links, or raw findings.
Release packages include only the binary, this README and the license. CI uploads
only packaged binaries and checksums, never local evaluation results.

## Next steps

The next layer should use language/schema adapters to extract changed writes,
contracts, destinations and idempotency behavior, followed by semantic review.
Keep observations separate from inferred risks. An optional model-based reviewer
should cite code and identify missing context rather than assert reversibility.
Neither AST analysis nor model review is implemented in this version.
