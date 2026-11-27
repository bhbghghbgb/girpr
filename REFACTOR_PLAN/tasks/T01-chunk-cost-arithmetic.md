# T01 — Non-negative saturating byte arithmetic in `girpr-chunk-cost`

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Add pure primitives for folding signed byte offsets/lengths into saturating
`u64` values in `girpr-chunk-cost`, and adopt them at the sites that currently
re-derive the clamp-negatives-then-saturating-add rule.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:161-174` — X8, saturating folds and range algebra.
- `docs/09-shared-code-refactor-findings.md:542-549` — LF-03, ~20 duplicated sites.

Named example sites in X8: `tools/capture/src/manifest/diff.rs:424-651`,
`tools/capture/src/manifest/mod.rs:713-758`,
`tools/capture/test-support/src/lib.rs:1130-1138`,
`app/chunk-verify/src/lib.rs:69-82`, `app/repair-plan/src/plan.rs:63,119,133`,
`src/repair/scan.rs:742`, `src/repair/scope/report.rs:154,176,305`.

## Boundary / approach

Widen `girpr-chunk-cost` with `nonneg`, `saturating_nonneg_sum`, and
`checked_end`. Keep the existing `chunk-cost` helper intact.

## Guardrails

- Signed cursors must use `checked_end`, not the `u64` saturating helper.
- `u64` budget saturations that never came from an `i64` stay local.
- Capture's A11 violation strings and repair's gap-fill policy are T02's scope;
  do not change them here.
- No published bytes or exit codes may change.

## Commit breakdown

1. Add the three primitives with unit tests covering negative, zero,
   overflow and signed-cursor cases.
2. Adopt in `app/repair-plan` and `app/chunk-verify`.
3. Adopt in `tools/capture` manifest code and test-support.
4. Adopt in the root repair `scan`/`scope` sites.

## Validation

- Targeted `cargo nextest` for `girpr-chunk-cost` and each adopting crate.
- Add edge-case tests for `checked_end` overflow; byte-golden suites must be
  unchanged.

## Done when

Every site that folds an `i64` byte quantity into a `u64` total/budget uses the
leaf helper, with no behavior change for negative or overflowing inputs.
