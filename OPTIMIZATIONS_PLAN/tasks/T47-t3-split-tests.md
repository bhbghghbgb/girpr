# T47 - T3: split the large integration test

Report: `../../OPTIMIZATIONS_REPORT.md` lines 1118-1133 (T3).

Dependencies: T46. Do this last, after the behavior changes and the new
regression/golden tests, so the split is mechanical and does not conflict with
those edits.

## Goal

`tests/repair_offline.rs` is very large (roughly 187 KiB). A single huge file
makes it expensive to find the case that pins a behavior, and a change to one
area recompiles all of it. It also hides duplicate fixtures across the other
test files. Split it by contract area, keeping the test bodies unchanged.

## Where to start

- `tests/repair_offline.rs`: the whole file; use its module-level helper and
  test names to group.
- `tests/common/mod.rs`: shared fixtures.
- `tests/repair_machinery.rs`, `tests/artifact_bridge.rs`,
  `tests/cli_contract.rs`: the smaller files; do not move tests that already
  live in the right place.
- `Cargo.toml`: `[[test]]` entries only if a split file needs explicit
  features; integration test files are auto-discovered.

## Steps

1. Inventory the tests and helpers in `repair_offline.rs`. Group by contract:
   - plan / `--print-plan` / `--plan-out`;
   - purge (before/after, scope, ceilings);
   - scan / check-only / reparability;
   - in-place / temp / discard writer paths;
   - post-phase (deprecated, audio, config bump);
   - output/data plane (if not already in `cli_contract`).
2. Create files such as `tests/repair_plan.rs`, `tests/repair_purge.rs`,
   `tests/repair_scan.rs`, `tests/repair_writer.rs`, `tests/repair_post.rs`.
   Keep the existing `tests/common` helpers in `common/mod.rs`; add a
   `#[path = "common/mod.rs"] mod common;` or the existing pattern in each new
   file.
3. Move test functions **verbatim** where possible. Do not rename tests unless
   two collide; if they do, keep both and make the names specific to the area.
   Do not rewrite assertions, fixtures, or comments in this task.
4. Deduplicate fixtures only when the same helper is byte-identical in two
   files; put it in `tests/common/mod.rs`. Do not merge tests with different
   intent even if they look similar.
5. Keep the process-level tests (`CARGO_BIN_EXE_girpr`, broken pipe, exit
   codes) together in a `tests/process.rs` or the file that owns the contract.
6. Ensure every test still runs: `cargo test -p girpr -- --list` before and
   after should have the same test names (or a documented rename). Compare the
   counts.
7. Do not change the implementation or the test behavior. This is file
   organization.

## Tests

- `cargo test -p girpr` before and after; the same tests pass.
- `cargo test -p girpr -- --list` shows the same set (except documented
  renames).
- Compile time is not asserted; the gain is navigation and incremental
  recompilation.
- If a test was accidentally platform-gated in the move, run it on this host
  and note the gate.

## Acceptance

- The large file is split by contract area, with test bodies unchanged.
- Shared fixtures live in `tests/common`.
- Every test still runs once.
- `cargo test -p girpr` passes.

## Commit plan

1. `test: split the offline repair suite by contract area`
2. `test: move shared fixtures into the common module`

## Notes

- Splitting tests is mechanical churn and merge-conflict-prone. Do it after
  the behavior fixes, not before.
- Do not "improve" a test while moving it. A later task can change behavior;
  this one only changes where the code lives.
- Keep the file names descriptive; a reader should guess the contract from the
  name.
