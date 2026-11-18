# T09 — `repair_machinery.rs` in-process `Summary` downshift

Priority P1 · root product integration tests · checkpoint CP3 · report finding A1

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L56–L94` (§A1).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L790–L795`
  (pattern 1), `L815–L816` (Wave 2), `L846–L849` (assumptions).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T09-repair-machinery-downshift.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Remove ~20–25 process spawns and log files by reading the public `Summary`
in-process for the cases that only read counters.

`tests/repair_machinery.rs` has 28 test attributes and 35 `run(...)` invocations.
Every test goes through `Rig::run_at_level` (:181–244), spawning
`CARGO_BIN_EXE_girpr` (:218), forcing `--output json` (:206–207) and writing a
per-run log file into a second `TempDir` (:194, :208–215). `.log()` is called at
29 sites concentrated at :594–600 and :864–1129 plus :1598 — roughly 13–15 tests
genuinely need the binary's log file. The figures are also available from the
public `Summary` (`src/lib.rs:71,74–76`; `src/report/summary.rs:49–120,215–236,299`)
and `summary_record(&summary, code)`.

## Implementation plan

Code anchors (validate before editing): `tests/repair_machinery.rs:10–18`
(header claim), `:181–244` (`Rig::run`/`run_at_level`), the 28 tests (notably
:410, :450, :622, :693, :789, :830, :1144, :1184, :1215, :1249, :1317, :1350,
:1372, :1391, :1427, :1471, :1502, :1548), `src/lib.rs:71,74–76`,
`src/report/summary.rs`, and `tests/common/mod.rs` (`ctx_for`, `run`, `field`;
the pattern in `repair_writer.rs`/`repair_limits.rs`).

Steps (large task — multiple commits expected):

1. Add `Rig::run_lib(&self, flags: &[&str])` that builds a `RunCtx` through
   `common::ctx_for(...)`, maps the used flags (`--scan`, `--repair-mode`,
   `--max-ram-bytes`, `--max-write-bytes`, `--include`, `--fail-fast`) onto its
   public fields, calls `common::run(ctx)`, and returns `(Summary, RunExit)`.
2. Read values via `summary_record(...).text()` plus the existing `common::field`
   helper.
3. Convert the ~13–18 log-free cases one group at a time; leave the ~10–13
   log-asserting tests on `Rig::run`.

## Acceptance criteria

* Every asserted number, exit code, chunk-fetch set, on-disk byte count and scope
  assertion is preserved (compare converted cases to their originals).
* Keep at least one binary case exercising the deprecated `--io-threads` alias
  (:185–186); `--output json` / CLI wiring remains pinned in `cli_contract.rs`.
* ~20–25 fewer process spawns and log files; test count unchanged unless cases
  are also merged (not required here).
* Affected suite passes.

## Guardrails

* The ~13–18 convertible count is an estimate — confirm per case by reading each
  test body; if a case asserts a process-level fact, keep it on the binary.
* Do not drop a `.log()` assertion to make a case convertible. Report follow-up
  ideas (e.g. merging now-nearby cases) without implementing them.
