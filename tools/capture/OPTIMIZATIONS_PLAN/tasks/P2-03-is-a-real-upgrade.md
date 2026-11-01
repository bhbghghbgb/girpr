# P2-03 — Remove or use `is_a_real_upgrade`

Report: `../../OPTIMIZATIONS_REPORT.md` lines 446-457 (B4) and 695-708 (D2).

Dependencies: P1-01, P2-01.

## Goal

Remove the production-shaped invariant with no production caller. Today
`PathDiff::is_a_real_upgrade` is used only by tests.

## Decision

Remove `PathDiff::is_a_real_upgrade`. The same-tag case is already refused by
P1-01; a different-tag pair with no changed paths is still a legitimate
metadata/observation capture, so adding a new refusal would change behavior
without a product decision. Keep the "something changed" predicate in the tests
that need it, expressed against `counts`.

Do not add a new guard in this task.

## Where to start

- `src/manifest/diff.rs`: `is_a_real_upgrade` (around line 224) and its tests
  (around lines 1125, 1286, 1296).
- Search the tree for any other caller; there should be none outside tests.

## Steps

1. Delete the method.
2. In the tests that used it, assert the counts directly
   (`counts.added > 0 || counts.removed > 0 || counts.changed > 0`) or add a
   local helper in the test module. Do not put the helper back on the
   production type.
3. If a test's real intent is "same content, different tag", rename the test to
   say that and assert the counts.
4. Do not change `diff_field`, `PathDiff` serde shape, or any document.

## Tests

- `cargo test -p gircap` passes with the method removed.
- The converted tests still fail if the underlying count logic regresses.

## Acceptance

- No dead production concept remains.
- No behavior change.
- `cargo test -p gircap` passes.

## Commit plan

1. `refactor(manifest): remove the unused real-upgrade predicate`
2. `test(manifest): assert the changed/added/removed counts directly`

## Notes

- Do not add a same-tag guard here; P1-01 owns it.
- Do not reference the report or plan in code comments.
