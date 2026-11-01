# P4-01 — One acceptable-state table

Report: `../../OPTIMIZATIONS_REPORT.md` lines 682-693 (D1).

Dependencies: P1-10.

## Goal

Remove the duplicate definition of "which matrix statuses A17 accepts".
`select::MatrixStatus::is_acceptable` and
`emit::selfcheck::MatrixRow::ok_status` currently encode the same four strings.

## Where to start

- `src/select/mod.rs`: `MatrixStatus` (around line 121), `is_acceptable`
  (around line 153), `id`, `Display`, `status_tests`.
- `src/emit/selfcheck.rs`: `MatrixRow::ok_status` (around line 791), `MatrixRow`.
- `src/emit/closeout.rs` uses `MatrixRow::ok_status(&m.status)`.

## Steps

1. Add `MatrixStatus::parse(&str) -> Option<Self>` (or `from_id`) in `select`,
   with the four `id()` spellings. `is_acceptable` can stay a `const matches!`
   or become `Self::parse(s).is_some()` if it takes a status value; prefer
   keeping the enum method and making `ok_status` call `parse`.
2. Implement `emit::selfcheck::MatrixRow::ok_status(status: &str) -> bool` as
   `crate::select::MatrixStatus::parse(status).is_some_and(|s| s.is_acceptable())`.
   `emit` already depends on `select`; do not invert the dependency.
3. Keep `MatrixRow::ok_status`'s public signature so `closeout`/verify callers
   are unchanged.
4. Remove the literal `matches!` list from `selfcheck`.
5. Existing tests that compare the two definitions should still pass; add a
   test that an unknown status is rejected by both.

## Tests

- `parse` round-trips every `MatrixStatus::id()`.
- `parse("coveredby")`, `parse("")`, `parse("CAPTURED")` return `None` (the
  wire vocabulary is exact).
- `ok_status("captured")` etc. true; `ok_status("coveredby")` false.
- The existing agreement test still passes.
- If a fifth state is added later, only `select` changes; state that in the
  doc comment.

## Acceptance

- One source of truth for acceptable matrix states.
- A17 behavior and wire spellings unchanged.
- `cargo test -p gircap` passes.

## Commit plan

1. `refactor(select): add a status parser for the matrix vocabulary`
2. `refactor(selfcheck): delegate ok_status to the matrix model`
3. `test(select,selfcheck): pin the shared acceptable-state rule`

## Notes

- Do not move `MatrixStatus` into `emit`; the dependency direction matters.
- Do not change the status spellings or A17's all-four policy.
- Do not reference the report or plan in code comments.
