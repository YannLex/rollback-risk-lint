# rollback-risk-lint

A review tool for developers and coding agents deciding whether a PR/MR can be
merged when its effects may survive a service rollback.

A coding agent changes the code; a trusted review agent uses this CLI to collect
rollback-risk signals, investigates the actual effects, and records its evidence.
The CLI validates that review against the current inputs. An authorized merge
agent can then use the result as one gate in an automatic merge workflow.

Rolling back a service does not restore overwritten data, undo external actions,
remove queued messages or make new data readable by an older version. Those are
the risks this review targets. The scanner is heuristic; the receipt validator
checks completeness and freshness, not evidence truth or rollback safety.

## The review-to-merge workflow

| Stage | Responsible component | Output |
| --- | --- | --- |
| Identify the change | Orchestrator | Repository, PR/MR, immutable base/head commits, merge policy |
| Collect signals | `rollback-risk-lint --agent-review` | Packet with findings, skips, analysis gaps and coverage task |
| Investigate | Trusted review agent | Evidence and a disposition for every packet item |
| Validate | `rollback-risk-lint --agent-review --review …` | `ALLOW`, `BLOCK` or `INCOMPLETE` |
| Merge when authorized | Merge agent / platform | Protected exact-head merge or queue entry |

The CLI does not contain a model reviewer, fetch PR metadata, authenticate a
reviewer or perform merges. Your agent harness supplies those capabilities.
`ALLOW` completes the rollback-risk review stage; repository authorization,
functional/security review, required CI and branch protection still apply.

## Install a trusted binary

Download a pinned version and its checksum from
[Releases](https://github.com/YannLex/rollback-risk-lint/releases).
Available platforms are Linux x86-64 (GNU, glibc 2.35+), macOS Apple Silicon and
Windows x86-64. Private repository downloads require GitHub authentication.
Git is required for revision analysis; Rust is only needed to build from source.

```sh
# macOS Apple Silicon example; choose the archive for your platform.
gh release download v0.4.0 --repo YannLex/rollback-risk-lint \
  --pattern '*aarch64-apple-darwin*'
shasum -a 256 -c rollback-risk-lint-aarch64-apple-darwin.tar.gz.sha256
tar -xzf rollback-risk-lint-aarch64-apple-darwin.tar.gz
./rollback-risk-lint-aarch64-apple-darwin/rollback-risk-lint --version
```

Put the extracted executable on your agent's PATH. Linux can use `sha256sum -c`;
Windows can compare `Get-FileHash` with the checksum and extract the zip.
To build a reviewed source checkout instead, run `cargo install --path . --locked`.
Provision the tool from a trusted release or revision, independently of the PR
being reviewed. Per-commit [Actions artifacts](https://github.com/YannLex/rollback-risk-lint/actions)
are retained for 30 days; release assets persist.

## Run a review agent

### 1. Capture the review scope and packet

Read the target repository's instructions and establish its merge policy. Resolve
the intended PR/MR's current base and head through your platform integration,
fetch both commits and enough history, and supply their immutable SHAs below.
Use private storage outside tracked target files for packets and receipts.

```bash
set -euo pipefail
umask 077
repo=/path/to/target-repository
base_sha=BASE_COMMIT_SHA
head_sha=HEAD_COMMIT_SHA
review_dir=$(mktemp -d "${TMPDIR:-/tmp}/rollback-review.XXXXXX")
rrl=(rollback-risk-lint --repo "$repo" --base "$base_sha" --head "$head_sha")
"${rrl[@]}" --agent-review > "$review_dir/packet.json"
```

Revision mode scans committed changes in `merge-base(BASE, HEAD)..HEAD`, reads Git
blobs directly, and disables external diff/textconv execution. It does not execute
the target application or include uncommitted edits. Missing refs/history or
unsupported input fail with exit 2. Packet generation exits 0 without granting ALLOW.

### 2. Investigate every item

Give the trusted review agent the packet and access to the scoped source. Its job:

- Trace changed values, predicates and control flow to persistence, publication,
  external calls or other effects. Check actual boundaries rather than relying
  on receiver names or configured declarations alone.
- Explain what survives rollback: write identity/scope, overwritten originals,
  old/new reader compatibility, retries, idempotency and in-flight/queued work.
  Inspect effective configuration and downstream consumers when relevant.
- Evaluate recovery evidence: retained originals, restore/replay tests,
  compensation or staged rollout controls. Distinguish an executed check from a
  proposed plan. Fix risks and regenerate the packet after code changes.
- Review the complete diff beyond the scanner's coverage, including skipped files,
  helper changes, schemas, configuration and indirect effects. An empty finding
  list still requires this assessment. Missing context remains incomplete.

The agent writes `receipt.json` containing the **entire original packet** and
exactly one resolution per item:

```json
{
  "packet": { "...": "entire packet.json object, unchanged" },
  "resolutions": [
    {
      "id": "coverage",
      "status": "verified",
      "evidence": ["Actual file:line references, downstream paths checked, check results and limits"]
    }
  ]
}
```

This is a format illustration, not a ready-to-accept receipt. Do not copy the
placeholder evidence or generate canned resolutions. `coverage` requires
`verified`. Every `finding:N`, `skip:N` and `gap:N` also needs a resolution:
`verified` (investigated and acceptable), `false_positive` (effect disproved),
`mitigated` (validated controls address the risk), or `blocked` (unresolved risk).
Nonblocked dispositions require nonempty evidence strings.

### 3. Recompute and validate

```bash
"${rrl[@]}" --agent-review --review "$review_dir/receipt.json" \
  > "$review_dir/decision.json"
```

| Result | Exit | Next action |
| --- | --- | --- |
| `ALLOW` | 0 | Continue to repository merge gates |
| `BLOCK` | 1 | Resolve the identified risk, regenerate and review again |
| `INCOMPLETE` | 1 | Supply missing evidence/context or review a fresh packet |
| Input/tool failure | 2 | Fix the failure; never interpret it as a pass |

Validation recomputes the packet and compares it with the receipt. A changed base,
head, effective policy or scan invalidates the old receipt. Missing, duplicate or
unknown assessments cannot allow a merge. `--fail-on never` cannot bypass this
validation. See [protocol details](docs/agent-review.md) for exact receipt semantics.

## Connect to automatic merge

Run this stage in trusted orchestration. Keep authoritative receipts in storage
that PR authors cannot replace. Treat target code, comments and PR-provided
receipts as review inputs, never as trusted instructions or approvals. Separate
review access from merge credentials; scanning does not need to execute PR code.

Configure a required rollback-review check produced by the trusted validator:
only ALLOW succeeds; BLOCK, INCOMPLETE and tool failures fail. Bind that check to
the reviewed commit and keep the packet, evidence and decision accessible to
reviewers under the repository's access policy. Re-run after head/base/policy
changes. A generic successful scanner job is not this review gate.

For GitHub, the following Bash example is a **final merge stage**, after the review
agent has written a trusted receipt and repository-specific merge authorization
has been established. It requires `gh`, `jq`, Git, the linter and fetched commits.
The example conservatively requires a nonempty set of required checks, all passing.

```bash
set -euo pipefail
repo=/path/to/target-repository
remote=OWNER/REPOSITORY
pr=PR_NUMBER
receipt=/private/review/receipt.json
base_sha=$(jq -er '.packet.base_sha' "$receipt")
head_sha=$(jq -er '.packet.head_sha' "$receipt")

# Recompute the packet; any non-ALLOW result stops this script.
decision=$(rollback-risk-lint --repo "$repo" --base "$base_sha" --head "$head_sha" \
  --agent-review --review "$receipt")
jq -e '.verdict == "ALLOW"' <<< "$decision" > /dev/null

# Ensure the intended PR still has the reviewed scope.
current=$(gh pr view "$pr" --repo "$remote" \
  --json baseRefOid,headRefOid,state,isDraft)
jq -e --arg base "$base_sha" --arg head "$head_sha" \
  '.state == "OPEN" and (.isDraft | not) and
   .baseRefOid == $base and .headRefOid == $head' <<< "$current" > /dev/null
checks=$(gh pr checks "$pr" --repo "$remote" --required --json bucket)
jq -e 'length > 0 and all(.[]; .bucket == "pass")' <<< "$checks" > /dev/null

# Choose the repository's permitted merge method; respect its protections/queue.
gh pr merge "$pr" --repo "$remote" --squash --match-head-commit "$head_sha"
```

The exact-head guard rejects a changed PR head at submission. It does not atomically
lock the base. Use required up-to-date checks or a merge queue, and configure
review revalidation for the queued candidate when its base changes. Queue entry
is not confirmation that a PR has merged. Do not bypass protections with `--admin`.
If you enable deferred `--auto` merging, the trusted review gate must remain
required and re-run for subsequent heads; a one-time local ALLOW is insufficient.

For GitLab, apply the same sequence through your MR integration: validate a fresh
receipt, match the current source and target revisions, check required pipelines
and approvals, then use the merge API's source-head SHA guard and protected merge
policy. Configure revalidation for target-branch changes or merge trains. The CLI
itself is platform-independent and provides no GitHub/GitLab merge adapter.

## Detection coverage and repository policy

Java, Python and Go revision analysis parses full changed files with Tree-sitter.
RRL010 compares changed functions around unchanged candidate effect calls, exposing
changes outside the small patch hunk. Other supported source/config formats use
lexical hunk rules.

| Rule | Signal |
| --- | --- |
| RRL001 | Destructive SQL: DROP, TRUNCATE, dropped columns |
| RRL002 | SQL UPDATE/DELETE |
| RRL003 | Idempotency/deduplication identifier changes |
| RRL004 | Likely persistence calls |
| RRL005 | Likely outbound/payment calls |
| RRL006 | Removed/changed protobuf assignments |
| RRL007 | Record selection/replacement changes in write components |
| RRL008 | Payload/eligibility changes in write components |
| RRL009 | Publication/routing/write-mode configuration changes |
| RRL010 | Changed Java/Python/Go function with an unchanged candidate effect call |

Declare known boundaries in the target repository's `.rollback-risk-lint.json`;
start with [the synthetic policy example](.rollback-risk-lint.example.json).
Effects specify `name`, `receiver`, `method` and `kind` (`persistence`, `message`,
`http`, `payment`); `pure` entries suppress guesses about local calls. Matching is
literal and case-sensitive. A receiver also matches `this.`/`self.` qualification;
`""` selects unqualified calls, `"*"` any receiver, and optional `path_suffix`
constrains paths. First matching effect wins; effects take precedence over pure
entries. Unknown fields/versions are errors. Declarations are knowledge to verify,
not compiler-resolved types or proof that a runtime effect occurs.

Revision mode loads policy from the **resolved base commit**, while the code diff
uses the merge-base. A head-only policy edit cannot disable its own checks. Use
`--config /trusted/policy.json` for a reviewed explicit override. Review policy
changes separately; never accept an untrusted head policy as that override.

Limits the review agent must cover:

- Correlation within a function is not control-flow or interprocedural data-flow
  proof. Unrelated edits can produce findings; helpers, aliases, reflection,
  deferred commits, changed signatures and renames can hide effects.
- Java functions are paired by enclosing names and syntactic parameter types;
  Python/Go use enclosing names, plus Go receiver types. Duplicate identities and
  syntax errors appear in `function_analysis.warnings`.
- Tests/fixtures are excluded by default and listed in `skipped_files`.
  `--include-tests` opts in; excluded files still need agent review coverage.
- Other hunk-rule languages include SQL, Kotlin, Rust, JavaScript/TypeScript,
  Ruby, PHP, C#, Scala, shell and protobuf; config rules cover YAML, properties,
  TOML, JSON and env files. Multiline/indirect behavior can be missed.
- Mode-only and rename-only diffs have no rule coverage. Binary, submodule,
  combined, non-UTF-8 and malformed input fail with exit 2. The tool cannot prove
  that a configuration was deployed or that a recovery plan works.

## Standalone scans and output

For investigation without the receipt protocol:

```sh
rollback-risk-lint --repo /path/to/project --base BASE_SHA --head HEAD_SHA \
  --format json --fail-on never
rollback-risk-lint --diff /path/to/change.patch --format json
```

Patch mode accepts complete Git-style unified diffs or `--diff -` for stdin.
When piping Git output, use `set -o pipefail` and
`git diff --no-ext-diff --no-textconv BASE...HEAD` so Git errors do not look like
empty changes. Patch mode has no full-function review and cannot generate an
agent-review packet. It uses an explicit `--config` or built-in defaults, not a
working-directory policy.

Scan-only exit codes differ from receipt validation: 0 means below the selected
finding threshold, 1 means the threshold was reached, and 2 means input/tool
failure. Default `--fail-on high` gates RRL001; `review` gates all signals; `never`
reports without failing on findings. None of these scan exits grants ALLOW.

JSON scan output includes `schema_version`, `findings`, `skipped_files`,
`policy_source`, `function_analysis` and `limitation`. Findings include old/new
locations, evidence, risk and requested recovery evidence; `related_evidence`
retains grouped locations. `boundary` and `function_name` identify function-level
candidates and configured/heuristic provenance. Removed code uses old-side
locations. Counts describe scan coverage, not detection accuracy.

## Contribute and evaluate

Read [AGENTS.md](AGENTS.md) before contributing. Run:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

For CLI/packaging changes, build a release binary and run
`python3 scripts/smoke.py /path/to/rollback-risk-lint`. CI runs native smoke tests
on all release platforms, covering Java/Python/Go analysis and receipt validation.

Use synthetic regression fixtures. Keep project-specific evaluations, patches,
PR/MR identifiers, packets and raw findings under ignored `docs/evaluations/` or
outside this repository. Apply the target repository's access policy to evidence;
never publish it in generic examples, release assets or CI logs. Packages contain
only the executable, generic README and license.
