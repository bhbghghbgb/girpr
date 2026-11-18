# T19 — Richer `CompareOutcome`, delete duplicated pipelines

Priority P2 · `tools/sync` (girsync) / production code · checkpoint CP9 ·
report finding E3

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L696–L719` (§E3).
* Report constraint (merge harnesses, not cases):
  `docs/09-test-scale-reduction-research.md:L826–L841`.
* Report optional follow-up (lazy/compare table-izing):
  `docs/09-test-scale-reduction-research.md:L717–L719` — seeded as follow-up F03,
  **not** part of this task.
* This plan: `OPTIMIZATIONS_PLAN/tasks/T19-sync-compare-outcome.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Remove ~250–300 lines of duplicated test pipeline by exposing the production
compare computation instead of re-implementing it in each harness.

Six reimplementations of classify → open_side → plan_pairs → resolve_side →
diff_maps: `tests/lazy/main.rs:96–163`, `tests/hash_mode/main.rs:206–243` and
`:250–302`, `tests/stat_trust/main.rs:173–211`, `tests/helpers.rs:44–72`,
`tests/compare.rs` (`effective_maps`). `src/commands/compare.rs:97–175` computes
exactly this pipeline but returns only `i32` (noted explicitly at
`lazy/main.rs:92–95`).

## Implementation plan

Code anchors (validate before editing): all six reimplementation sites plus
`src/commands/compare.rs:97–175`.

Steps (one or two commits):

1. Factor the body into public
   `compare_detailed(opts, log) -> Result<CompareOutcome>` with
   `CompareOutcome { diff, src_stats, dst_stats, code }` (and a
   `compare_self_detailed`), with `cmd_compare` calling it and rendering.
2. Test harnesses become one shared module reading the production computation.

## Acceptance criteria

* Assertions read the production computation instead of a test copy — strictly
  stronger.
* ~250–300 lines of duplicated pipeline removed; one shared harness.
* The grouped suites still test their genuinely different mechanisms — merge
  their **harness**, not their cases.
* Affected suites pass. Production rendering for `cmd_compare` unchanged.

## Guardrails

* Do **not** table-ize `tests/lazy/compare.rs`'s ~15 near-identical tests here —
  that is follow-up F03, to be decided separately.
* Low–medium risk: preserve side labels and the re-threading of `ScanMode`,
  `RwSide`, `serial_rw()`. Report follow-up ideas without implementing them.
