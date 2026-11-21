# T04 — Pure log-announcement helper to drop cli_contract spawns

## Goal / pointers
- `docs/10-test-optimization-research.md` §F18 (lines 297–306) — `cli_contract.rs`
  still spawns the binary ~24 times; 14 are log-announcement cases. Fix: extract the
  announcement logic from `src/main.rs` into a pure helper unit-tested in-process
  (`init_tracing` is already public and tested), keep one binary log smoke, and point
  post-parse logs at a scratch `--log-file` so `girpr_<ts>.log` files stop accumulating
  in `target/`.
- Assumptions to validate: §A1 (46–52), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` and `CONVENTIONS.md`.

## Scope
- `src/main.rs` (the announcement block), `src/lib.rs` / `src/logging.rs` (`init_tracing`
  and its tests), and the 14 logging cases in `tests/cli_contract.rs`.
- The scratch `--log-file` handling for post-parse logs. Locate the exact cases in the
  current tree before editing.

## Implementation plan
1. Validate §F18: enumerate the 14 announcement cases and confirm they are pure
   functions of arguments/config rather than process-boundary behavior.
2. Extract the announcement into a pure callable surface with in-process unit tests that
   assert the same announcement text as the current cases; keep the binary path calling
   it so shipped behavior is unchanged.
3. Convert the 14 cases to in-process assertions; keep one binary log smoke.
4. Route post-parse logs in those tests to a scratch `--log-file` so nothing is written
   beside the binary under `target/`.
5. Update nextest filters if a named case moves (§A10); verify CLI goldens are unchanged.

## Constraints / validation
- The announcement bytes must not change — they are part of the CLI log contract
  (§6). A pure helper must be the single source for both binary and test paths.
- Preserve one process-level log smoke (§A12). Do not remove the "log is a process
  artifact" coverage addressed in T03 without keeping a replacement here.
- Do not pull in new dependencies or couple to gircap.

## Done when
The 14 announcement cases assert the pure helper in-process, one binary log smoke
remains, no `girpr_<ts>.log` files are produced under `target/`, CLI log bytes and
goldens are unchanged, and the suite passes with fewer spawns.
