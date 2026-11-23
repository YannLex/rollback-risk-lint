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
gh release download v0.4.0 --repo YannLex/rollback-risk-lint --pattern '*aarch64-apple-darwin*'
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

## Coding review agent protocol

Use revision mode to generate a review packet for a trusted coding review agent:

```sh
rollback-risk-lint --repo /path/to/project --base BASE_SHA --head HEAD_SHA \
  --agent-review > /private/review-packet.json
# The agent investigates every item and writes /private/review-receipt.json.
rollback-risk-lint --repo /path/to/project --base BASE_SHA --head HEAD_SHA \
  --agent-review --review /private/review-receipt.json
```

See [the review protocol](docs/agent-review.md) for receipt format and merge workflow.
The validator recomputes the packet and returns `ALLOW`, `BLOCK` or `INCOMPLETE`.
Every packet requires a complete-diff coverage assessment, even with no findings.
Evidence is an agent attestation: this checks completeness and freshness, not
whether the evidence is true. `ALLOW` completes rollback-risk review; repository
authorization, other reviews, required CI and branch protection still apply.

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
| RRL010 | review | Changed Java/Python/Go function with an unchanged candidate effect call | Trace changed inputs/control/write sets to the reported boundary |

## Repository effect policy

Create `.rollback-risk-lint.json` in the repository being reviewed. Start from
[the synthetic example](.rollback-risk-lint.example.json):

```json
{
  "version": 1,
  "effects": [
    {"name": "replace-orders", "receiver": "orderStore", "method": "replace", "kind": "persistence"},
    {"name": "publish-events", "receiver": "eventBus", "method": "publish", "kind": "message"}
  ],
  "pure": [{"receiver": "cache", "method": "save"}]
}
```

Kinds: `persistence`, `message`, `http`, `payment`. Each effect requires a unique,
nonempty name; methods must be nonempty. Unknown JSON fields and policy versions
are errors. Policy matching is case-sensitive and literal: `receiver: "orderStore"`
also matches `this.orderStore` or `self.orderStore`. An empty receiver selects
unqualified function calls. `receiver: "*"` deliberately matches any receiver;
use optional `path_suffix` to constrain such rules to a file or directory suffix.
The first matching effect wins. Declared effects take precedence over `pure`
entries; pure entries suppress only sink guesses, not code from the surrounding
function comparison. These declarations are repository knowledge, not verified
types or proof of an external effect.

Revision mode loads policy from the **resolved `--base` commit**, while the code
diff uses merge-base(BASE, HEAD)..HEAD. Thus a PR cannot disable its own checks by
editing policy, and policy newly added to the target branch applies even when
the feature branch diverged earlier. Use `--config /trusted/policy.json` for an
explicit override. Patch-only mode uses that explicit file or built-in defaults;
it does not search the working directory for policy.

## Function-level review and limits

With `--base`/`--head`, Java, Python and Go files are read directly from Git objects
and parsed with Tree-sitter. Nothing from the target application is executed.
Changed functions are compared against their previous version. If a semantic
change shares a function with an unchanged candidate effect call, RRL010 reports
both the changed lines and the actual call location. This can reveal changed
payloads, eligibility or write-set construction outside the small patch hunk.

Java methods are paired by enclosing names and syntactic parameter types; Python
and Go use enclosing names (plus Go receiver types). Duplicate identities and
syntax errors are reported in `function_analysis.warnings`. This is syntactic
comparison, not compiler type resolution, a control-flow proof or interprocedural
data-flow analysis. An unrelated semantic edit in the same function can still
produce a candidate. Preparation in a separate function, deferred commit stages,
changed signatures, aliases, reflection, unknown boundaries and function/file
renames can still escape detection. Ordinary comments, formatting, docstrings and
standalone logging with no nested calls are ignored; nested calls remain visible.

Patch-only reports explicitly use `function_analysis.mode: "patch_only"` and warn
that full-function review was unavailable. Other source languages continue to use
hunk rules: SQL, Kotlin, Rust, JavaScript/TypeScript, Ruby, PHP, C#, Scala, shell
and protobuf. Configuration rules cover YAML, properties, TOML, JSON and env files.
These hunk rules are lexical and can miss multiline or indirect behavior.

Receiver-name heuristics and known local declarations reduce ambiguous `update`,
`put`, `remove` and `capture` alerts. They are not type resolution. Tests/fixtures
are excluded by default (test directories, Go `_test.go`, Python `test_*`, JS/TS
`.test`/`.spec`, Java `*Test`/`*Tests`/`*CT`); opt in with `--include-tests`.
Excluded files appear in `skipped_files`. Old/new sink lines are grouped; contextual
replacement/payload signals are grouped by file. Mode-only and rename-only diffs
have no rule coverage. Binary, submodule, combined, non-UTF-8 and malformed input
fail with exit 2. Empty Git-style unified diffs are valid; mail wrappers are not.

No findings does not establish rollback safety. The tool cannot verify whether
a configuration was deployed or whether a proposed repair actually works.

## Output and CI policy

Text is intended for reviewers. `--format json` returns a versioned object with
`schema_version`, `findings`, `skipped_files`, and `limitation`. Each finding carries
its rule ID, severity, path, line, diff side, literal evidence, risk and requested
recovery evidence. `related_evidence` retains grouped old/new locations and text.
`boundary` distinguishes `configured` from `heuristic` calls and includes its kind,
policy name, receiver, method and location. `function_name` identifies function
candidates. `policy_source` records `base`, `explicit` or `default`.
`function_analysis` reports its mode, successfully parsed files, paired functions
and coverage warnings. None of these counts measure detection accuracy.
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
Java/Python/Go function parsing is implemented. Compiler type resolution,
interprocedural data flow and model review are not implemented.
