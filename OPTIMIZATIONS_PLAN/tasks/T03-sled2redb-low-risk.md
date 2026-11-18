# T03 — sled2redb low-risk merges: stale-file fixture + destination/temp case

Priority P0 · `tools/sled2redb` · checkpoint CP1 · report findings D3, D4

## Pointers (give these to the implementing session)

* Report finding D3: `docs/09-test-scale-reduction-research.md:L529–L538`.
* Report finding D4: `docs/09-test-scale-reduction-research.md:L540–L549`.
* Report synthesis: `docs/09-test-scale-reduction-research.md:L804–L805`
  (pattern 4), `L812–L814` (Wave 1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T03-sled2redb-low-risk.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Two small reductions in `tools/sled2redb/tests/convert.rs` that drop sled fixture
builds and full conversions without losing a distinct assertion.

1. **D3 — "refuse to overwrite" needs a stale file, not a conversion** (:262–275,
   line :270). A full conversion creates `dst` whose result is never inspected.
   `sled_to_redb` only checks `dst.exists()` (`convert.rs:151`) and
   `dst.is_dir()` (:277) before writing the temp sibling. Replace with
   `std::fs::write(&dst, b"stale")` to exercise the same refuse branch.
2. **D4 — fold `a_successful_conversion_creates_the_destination_and_no_temporary`**
   (:416–432) into the round-trip test (:196–260). Both assert
   `(files, dirs) == (2,1)`, `dst.is_file()` and source untouched; the only
   unique assertion in :416–432 is `entry_names(&work)` (no leftover temp), also
   present at :312–316 and :356–364. Add it to the round-trip test, delete
   :416–432.

## Implementation plan

Code anchors (validate before editing): `tests/convert.rs:151`, `:196–260`,
`:262–275`, `:312–316`, `:331–365`, `:416–432`, and the `build_sled_db` helper
(:78–141).

Possible commits: one per bullet (two small commits).

Hygiene: remove the dead `let _ = &log();` at `tests/convert.rs:256` (D4).

## Acceptance criteria

* D3: refuse-to-overwrite branch still exercised; −1 sled fixture, −1
  conversion; byte-for-byte "preserve a real cache under force" (:331–365)
  untouched.
* D4: the no-leftover-temp assertion survives in the retained round-trip test;
  −1 test, −1 sled fixture, −1 conversion; the dead `let _ = &log();` is gone.
* Conversion behavior (production `convert.rs`) unchanged. Affected suite passes.

## Guardrails

* Do not remove conversions whose result is asserted (the byte-equality and
  flag-spelling properties at :515 are distinct — see the earlier D03 follow-up
  precedent). Validate each anchor. Test-only change. Report follow-up ideas
  without implementing them.
