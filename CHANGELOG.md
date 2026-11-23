# Changelog

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
