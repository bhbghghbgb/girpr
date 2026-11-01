# P1-01 — Refuse identical resolved version tags

Report: `../../OPTIMIZATIONS_REPORT.md` lines 56-74 (A1), 446-457 (B4), and
833-874 (ordering context). This task fixes the resolved-pair case only.

Dependencies: P0-01.

## Goal

Make "the two resolved tags differ" a precondition of a capture. Today
`config::capture_plan` rejects identical tags only when both flags are present,
before resolution; `VersionPair::select` can resolve `BranchHead` and
`FirstDiffTag` to the same string and the run proceeds.

## Where to start

- `src/config/mod.rs`: `VersionPair::select` (around line 88), `VersionRef`,
  `tag_shape_problem*`, and `capture_plan` (around line 328). The flag-level
  check is in `capture_plan` and must stay.
- `src/config/tests.rs` for the pure `select` tests.
- `src/manifest/diff.rs` (`PathDiff::is_a_real_upgrade`) is only context; do not
  change it here.

## Decision

Refuse with a clear error and no new opt-in flag. Same-tag recapture is not a
documented workflow, and adding a flag would enlarge the CLI for a case no
artifact needs. If the implementer finds a documented workflow that requires
same-tag recapture, stop and return the decision instead of adding a flag.

## Steps

1. In `VersionPair::select`, after both tags are resolved and their shapes are
   validated, compare the resolved `new_tag` and `old_tag`. Use the same
   case-folding rule as the flag check (`eq_ignore_ascii_case`) so the two
   entry points agree.
2. Return an error that names both sources and both resolved values, e.g.
   "the resolved pair is `main.tag` = X and `diff_tags[0]` = X; the pair must
   differ". Do not reuse the flag-specific wording.
3. Keep the pre-resolution flag check: it still fails earlier and gives a
   better message for explicit flags.
4. Do not compare build ids. Two tags with the same build id are a different
   (and possibly legitimate) observation; this task is only about tags.

## Tests

- Unit test: `BranchParams` with `tag == diff_tags[0]`, `BranchHead` +
  `FirstDiffTag` -> `select` returns `Err` naming both sources.
- Unit test: explicit equal flags still hit the existing `capture_plan` error.
- Unit test: a resolved pair that differs still succeeds, including a
  case-only difference if the existing rule allows it (match the flag rule).
- If an end-to-end exit-code test is cheap, assert the capture exits before any
  network or output directory work. Otherwise the unit tests are enough.

## Acceptance

- A resolved same-tag pair cannot reach capture.
- A valid resolved pair behaves exactly as before.
- No artifact bytes change for valid runs.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(config): refuse a resolved version pair whose tags are identical`
2. `test(config): cover resolved-tag equality from branch head and first diff tag`

## Notes

- Do not mention the report or task id in code comments.
- The error is a usage/shape error before capture; use the existing error type
  and exit path rather than inventing a new one.
