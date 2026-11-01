# P4-02 — Remove remaining dead fields

Report: `../../OPTIMIZATIONS_REPORT.md` lines 695-708 (D2).

Dependencies: P2-03, P3-02, P4-01.

## Goal

Remove the dead surface that earlier tasks did not already handle. By the time
this task runs:

- `PathDiff::is_a_real_upgrade` is removed by P2-03;
- `server::error_shape`'s `extra` is removed by P3-02;
- the two comparators are documented/separated by P2-01 (do not unify them
  here);
- the remaining item is `http::probe::ErrorProbes::other_manifest_prefix`,
  populated by `run` but never read.

## Where to start

- `src/http/probe.rs`: `ErrorProbes` (around line 442),
  `other_manifest_prefix`, and any construction.
- `src/run/mod.rs` (around line 230) where it is populated.
- Search for any future-intent comment; if the field is deliberately reserved,
  document that instead of removing it. The report says removing it is better
  than leaving a field that looks meaningful unless it is intended.

## Steps

1. Confirm with `rg` that no other code reads `other_manifest_prefix`.
2. Remove the field from `ErrorProbes` and the assignment in `run`.
3. If the field was serialized or part of a public struct, check the document
   shape. `ErrorProbes` is internal probe input, not a published document; if
   it turns out to be serialized, stop and return the tradeoff rather than
   changing an artifact.
4. Remove any now-unused import or parameter exposed only for that field.
5. Do not remove `by_cost` or `cost_then_field_then_path`; P2-01 keeps both
   with distinct meanings.
6. Do a final `rg` for the other D2 items and note in the return that they were
   handled by their own tasks.

## Tests

- `cargo test -p gircap` passes.
- Probe tests compile without the field.
- No artifact/document change; if a test asserted the field, remove only that
  assertion.

## Acceptance

- No dead probe field remains.
- No behavior or artifact change.
- `cargo test -p gircap` passes.

## Commit plan

1. `refactor(http): drop the unused probe manifest prefix`

## Notes

- This is intentionally a small cleanup; do not expand it into comparator
  unification.
- Do not reference the report or plan in code comments.
