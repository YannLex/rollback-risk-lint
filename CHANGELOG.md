# Changelog

## 0.4.0

- Add revision-bound packets for trusted coding review agents.
- Require complete-diff coverage and per-finding/skip/gap evidence dispositions.
- Recompute packet inputs before returning ALLOW, BLOCK or INCOMPLETE.
- Reject stale, duplicate, unknown and incomplete receipts; never merge directly.
- Document trusted receipt handling and exact-head protected merge handoff.

## 0.3.0

- Add trusted, versioned JSON declarations for effect boundaries and local calls.
- Compare Java/Python/Go functions around unchanged candidate effect calls.
- Expose declared/heuristic provenance, boundary locations and function-analysis gaps.
- Pair Java overloads by syntactic parameter types and retain nested effect calls
  inside logging/interpolated expressions.
- Keep all new findings review-only; this is not interprocedural data-flow proof.

## 0.2.0

- Exclude conventional test and fixture files by default; add `--include-tests`.
- Use receiver tokens and nearby local declarations to reduce ambiguous-call noise.
- Group old/new sink evidence while retaining all locations in `related_evidence`.
- Add review rules for record replacement, payload/eligibility changes in write
  components, and publication/routing/write-mode configuration.
- Keep repository-specific evaluations local and ignored; packages contain only
  the executable, generic README and license.

Contextual rules are bounded heuristics, not type resolution or data-flow proof.
Upstream orchestration and unrecognized write construction can still be missed.
