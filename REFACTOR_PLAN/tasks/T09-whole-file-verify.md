# T09 — Whole-file presence/length/digest gate

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Express the presence → length → digest decision tree as one pure gate in
`app/chunk-verify`, and adopt it in the repair and fetch verify paths with an
injectable digest source.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:151-159` — X7, parallel gates.
- `docs/09-shared-code-refactor-findings.md:310-313` — R-06, two repair gates.
- `docs/09-shared-code-refactor-findings.md:412-420` — APP-05, two verdict enums.

## Boundary / approach

Add `verify_whole_file(expected_size, expected_digest, source) -> WholeFileVerdict`
to `app/chunk-verify`. Each program maps the verdict to its own error type
(`ManifestVerifyError`, `DigestVerdict`).

## Guardrails

- The scanned-digest reuse contract (`--check-only --scan`) stays local.
- XXH64-vs-MD5 row choice, exit codes and warnings stay local.
- Fatal-vs-retry policy stays local; the MD5-only comparison in
  `app/fetch/src/sophon/stream.rs` adopts the primitive without changing policy.
- This is a correctness-critical verify path: no silent change to what counts
  as a match.

## Commit breakdown

1. Add verdict type and pure gate with tests for absent/short/length-mismatch/digest-mismatch/match.
2. Adopt in repair `phases/check` and `file/temp`.
3. Adopt in `app/fetch/src/persistent` and the sophon MD5 comparison.

## Validation

- Verify-path tests in all three callers; exit-code contract tests.
- `cargo nextest` for chunk-verify, fetch and root repair.

## Done when

The decision tree has one implementation and each caller keeps its own next
action, error spelling and exit behavior.
