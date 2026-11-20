# T20 — Shared generic test helpers

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F17 (lines 260–269) — the
  consolidation precedent and remaining generic duplication.
- Assumptions to validate: §A5 (45–50), §A11 (90–94), §A2 (21–27).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§N7 fake HTTP servers).

## Scope
- Candidate home: the shared test-support module used by T19 (e.g.
  `girpr-tempdir`'s `test_support` or a dedicated leaf test crate).
- Report names: `tests/common/digest.rs` (`md5hex`, overlapping T01),
  `tests/common/run_lock.rs` vs `tools/capture/tests/common/mod.rs` (fs2
  cross-process fixture locks), `tests/common/mod.rs` vs
  `crates/tempdir/src/test_support.rs` (`temp_dir`, `write_game_file`).
  Locate before editing.

## Implementation plan
1. Validate §F17 and §A11: `girpr-tempdir` already has a non-dev dependency on
   girsync and serde_json for `test_support`; be explicit about which shared
   helper lives where so no program's test comes to depend on another's.
2. Consolidate only the genuinely generic helpers: `md5hex` (coordinate with
   T01), the `fs2` cross-process fixture lock, and `temp_dir` / `write_game_file`.
   Follow the shim precedent from §F17.
3. Explicitly exclude the four fake HTTP servers and tool-specific fixture
   builders (§F17, §N7) — they encode different contracts and the spec forbids
   coupling the tools through fixtures (§A2).
4. Keep helpers public/re-exported per §A5 so nextest can use them.

## Constraints / validation
- No program's test may depend on another program's test (§A2, §A11).
- Do not share fake HTTP servers or domain fixtures (§N7).
- nextest one-process-per-test / no-doctest constraints must keep working (§A5).

## Done when
The generic helpers are consolidated behind shims, fake HTTP servers and
tool-specific fixtures remain independent, and all affected test suites pass
under nextest.
