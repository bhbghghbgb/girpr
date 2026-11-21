# T03 — Structured repair diagnostics to drop log-only spawns

## Goal / pointers
- `docs/10-test-optimization-research.md` §F17 (lines 284–295) — 10 `repair_machinery`
  cases spawn `CARGO_BIN_EXE_girpr` only to assert implementation-detail log strings
  (`ascending=`, `salvaged=`, trim/reuse, chunk stats, per-chunk trace records). Fix:
  expose a structured `RepairDiagnostics` alongside `Summary` through the existing
  test-support surface (or a conditional diagnostics record that leaves `cli_contract`
  goldens unchanged), convert those 10 cases to `run_lib`, keep one binary smoke.
- Assumptions to validate: §A1 (46–52), §A10 (108–110), §A12 (118–121), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` and `CONVENTIONS.md`.

## Scope
- `tests/repair_machinery.rs` (`Rig::run`, `run_lib`, the 10 log-asserting cases),
  the root test-support surface that wraps `Summary`, and the production repair entry
  point that would produce the structured diagnostics.
- `tests/cli_contract.rs` golden outputs (must remain unchanged if a conditional
  record is used). Locate the exact cases and fields in the current tree before editing.

## Implementation plan
1. Validate §F17: enumerate which spawns exist solely for log text and which carry a
   genuine process-boundary assertion; list the exact fields those assertions need.
2. Add a structured diagnostics value alongside `Summary` on the in-process path, or a
   gated record that yields the same data without changing CLI log bytes. Keep the
   shipped behavior and goldens identical.
3. Convert the 10 log-only cases to `run_lib` and assert the structured fields instead
   of parsing log files; keep one binary smoke that proves "log is a process artifact".
4. Remove the now-unused spawn scaffolding (spawn helper, log reads, temp trees) for
   those cases; update nextest filters only if a named case moves (§A10).
5. Deliver in commits separating the production/test-support surface from the test
   migration, since this is an L-effort change.

## Constraints / validation
- Production API change: it must not alter shipped output bytes, exit codes or CLI
  contract goldens (§6). A conditional diagnostics record is acceptable only if it is
  inert outside tests.
- Preserve one process-level smoke for the "log is a process artifact" property — do
  not remove all binary coverage (§A12).
- Do not make the structured surface depend on gircap or another program.
- Every converted case must keep the semantic assertion, not merely stop checking it.

## Done when
The 10 log-only cases run in-process against structured diagnostics, one binary smoke
remains, CLI goldens and shipped bytes are unchanged, and the repair suite passes with
fewer process spawns, log files and temp trees.
