# T17 — Sync scoped subscriber + injectable console writer

Priority P2 · `tools/sync` (girsync) / production code · checkpoint CP8 ·
report finding E2

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L673–L694` (§E2).
* Report caveat: `docs/09-test-scale-reduction-research.md:L821–L824`
  (verify rayon event threading before downshifting output.rs:703).
* Report assumptions: `docs/09-test-scale-reduction-research.md:L843–L851`.
* This plan: `OPTIMIZATIONS_PLAN/tasks/T17-sync-subscriber-writer.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Remove ~9 more spawns by asserting tracing output in-process with a scoped
subscriber and an injected console writer. Builds on T16 (E1).

`src/logging.rs:96–180` `init_tracing` calls global `try_init()` (:148–154) and
hardwires the console writer to `std::io::stderr` (:105). Nine tests in
`tests/output.rs` (:352, :438, :487, :536, :587, :627, :703, :915, :994, plus
:306) spawn the binary to read log output.

## Implementation plan

Code anchors (validate before editing): `src/logging.rs:96–180`, `:105`,
`:148–154`, the nine `tests/output.rs` tests listed above.

Steps (one or two commits):

1. Extract
   `pub fn subscriber_for(plan: &LogPlan, console_writer: impl MakeWriter + 'static)
   -> impl Subscriber` out of `init_tracing`.
2. Tests use
   `tracing::subscriber::with_default(subscriber_for(...), || run_captured(cli, plan))`
   and keep the `LogGuard` alive for the closure. `Report::emit`'s
   `tracing::enabled!(DEBUG)` gate respects the scoped default, so field-by-field
   log assertions remain valid.

## Acceptance criteria

* Identical layer stack, filters, JSON format and file path — only installation
  scope and console writer differ.
* ~9 spawns removed; combined with T16, `CARGO_BIN_EXE_girsync` shrinks to ~2–3
  smoke tests.
* All log-field assertions preserved. Affected suites pass.

## Guardrails

* **Medium risk:** the non-blocking file writer's `WorkerGuard` must outlive the
  closure; `with_default` is thread-scoped, so rayon worker events would not
  inherit it. Verify the `scan progress` / `effective done` events asserted at
  `output.rs:703` are main-thread **before** downshifting that one — otherwise
  fall back to a process smoke and report why.
* Sequence after T16. Report follow-up ideas without implementing them.
