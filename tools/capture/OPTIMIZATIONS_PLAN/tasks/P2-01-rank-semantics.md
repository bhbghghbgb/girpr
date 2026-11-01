# P2-01 — Separate scoring rank from admission rank

Report: `../../OPTIMIZATIONS_REPORT.md` lines 387-410 (B1), 876-895 (Phase 2).

Dependencies: P1-10, P1-12.

## Goal

Make `diff/candidates.json` say what its rank means. Today `candidates_document`
uses `manifest::diff::by_cost` and writes `rank`, but `select::candidates`
sorts with `cost_then_field_then_path` and also includes one-sided rows. For
equal `total_cost`, `moved`/`reused` can make the two orders disagree, so `rank`
is neither a scoring rank nor an admission rank.

## Decision

Publish both, with distinct names, without changing admission:

- `rank` remains the **scoring rank** from `by_cost` (the existing order and
  existing column name), documented as such.
- Add `admission_rank`: the one-based position of this common path in
  `select::candidates(diffs)`, the order the picker actually uses. One-sided
  rows are not in this document; `admission_rank` still reflects where this
  path sits among the full pool, so it is meaningful even when one-sided rows
  interleave.
- If a common path is absent from the pool (for example a folder excluded by
  P1-10), emit `null` for `admission_rank` and keep the row in the scoring
  table only if `by_cost` includes it. Prefer omitting excluded rows from the
  document entirely and documenting that the table is scored common paths.

Do not unify the comparators and do not change which files are captured.

## Where to start

- `src/select/mod.rs`: `candidates` (around line 393),
  `cost_then_field_then_path`, `candidates_document` (around line 874),
  `by_cost` import.
- `src/manifest/diff.rs`: `by_cost` (around line 696) and its tie-break.
- `tests/schema.rs`, `src/select/tests.rs`
  (`candidates_document_carries_the_columns_and_a_one_based_rank`).
- `docs/01-artifact-schema.md` for the `diff/candidates.json` columns/prose.

## Steps

1. Build the admission pool once in `candidates_document` with
   `candidates(diffs)`, and a `path -> position` map (first occurrence wins if
   duplicate paths exist; duplicates are malformed but do not panic).
2. Keep the `by_cost` enumeration for `rank`.
3. Add `admission_rank` to each row (`rank + 1` from the pool position). Use
   `serde_json::Value::Null` if absent.
4. Add `"admission_rank"` to the `columns` array in the right place (after
   `rank`, or before; keep the array and rows aligned).
5. Update the doc comment and permanent schema prose: `rank` is scoring order;
   `admission_rank` is selection order; one-sided rows are not in this table.
6. Add a test with two common paths of equal `total_cost` but different
   `moved`/`reused`, such that the two orders differ. Assert `rank` follows
   `by_cost` and `admission_rank` follows `candidates`.
7. Add a test that every common row has an `admission_rank` and that the ranks
   are unique for unique paths.

## Tests

- Existing rank test updated for the new column.
- Tie-break divergence test above.
- A one-sided row is still absent (or explicitly `null` if the chosen shape
  includes it; do not silently add rows).
- Schema test / docs check for the new column name.

## Acceptance

- The artifact explains both the score order and the admission order.
- Admission behavior is unchanged.
- `cargo test -p gircap` passes.

## Commit plan

1. `feat(select): publish admission rank beside the scoring rank`
2. `test(select): cover a cost tie where scoring and admission differ`
3. `docs(schema): define the two candidate ranks`

## Notes

- Do not rename or remove the existing `rank` column; consumers may rely on it.
- Do not change `by_cost` or `cost_then_field_then_path` in this task.
- Do not reference the report or plan in code comments.
