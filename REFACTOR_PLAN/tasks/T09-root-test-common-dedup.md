# T09 — De-duplicate root `tests/common/mod.rs` inlining

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F29 (lines 349–363) —
  `tests/common/mod.rs` (1,438 lines: offline HoYoPlay/Sophon mock and fixture
  builders) is `mod common;`-inlined by 11 of 14 root integration binaries,
  ~15.8k lines recompiled per lib change.
- Assumptions to validate: §A7 (63–70), §A11 (96–101), §A2 (27–33), §F31 (377–383).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (fake servers §F40).

## Scope
- `tests/common/mod.rs`, the 11 root integration binaries that inline it,
  `tests/common/run_lock.rs` (220) and `tests/common/digest.rs` (included at
  `common/mod.rs:252`).
- `crates/log` `test-support` feature and `crates/tempdir::test_support` as the
  §A7 precedent for a public helper with thin shims.
- Locate all `#[path]`/`mod common;` sites in the current tree before editing.

## Implementation plan
1. Validate §F29: list every binary that inlines the helper and confirm the
   mock/fixture code is safe to share (it must not couple programs — §A5).
2. Move the shared mock/fixture builders into a public library surface (an
   existing test-support crate or a new logic-free helper crate) and replace
   each `mod common;` inline with a thin per-consumer shim that imports it
   (§A7: one process per test).
3. Keep the four fake HTTP servers distinct (§F40); do not merge their
   contracts. Only the fixture/mock mechanics that are truly identical move.
4. Update `.config/nextest.toml` filters if any fully-qualified test path or
   binary name changes (§F31). If gircap gains a leaf dep, update §A5/§F39.

## Constraints / validation
- Test helpers must be a public library surface + thin shims; a helper reached
  by inlining for every test is the cost being removed (§A7).
- Do not make one program's test depend on another's (§A5). Keep the capture
  mock's refusal to borrow from the product's (§F40).
- No byte/behaviour change in production code (§A6); this task is test-only.
- §A11: moving `#[cfg(test)]` code has limited payoff — confirm the target is
  the inlined *integration* helper, not a shipped lib test tail.

## Done when
The shared mock/fixture code is compiled once in a library and reached through
thin shims, no program's test depends on another's, and all root integration
tests pass with unchanged nextest filters.
