# T07 — Bounded concurrent join with panic recovery

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Extract the indexed bounded-concurrency window, slot restoration and panic
payload recovery into one logic-free async-join leaf, and adopt it in capture's
task driver and the repair phases.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:126-140` — X5, bounded join + panic payload.
- `docs/09-shared-code-refactor-findings.md:265-274` — R-01, four repair phases.
- `docs/09-shared-code-refactor-findings.md:276-283` — R-02, dispatcher capture.
- `docs/09-shared-code-refactor-findings.md:342-347` — R-10, downcast in three crates.
- `docs/09-shared-code-refactor-findings.md:752-756` — CAP-04, capture side.

## Boundary / approach

Add an `async-join` leaf with an indexed collection, `jobs.max(1)`, result
restoration by input slot, panic payload downcast with parameterized fallback and
task label, and an optional dispatcher-carrying `spawn_blocking` with a
thread-setup hook.

## Guardrails

- Failure meaning is policy and differs per caller: collect failures, fatal
  exit, conservative unknown, fail-fast.
- Capture catches panics inside buffered futures; the repair file driver uses
  `Handle::block_on` on the blocking pool. Cancellation semantics must not be
  collapsed.
- The scoped tracing dispatcher in `src/repair/file/mod.rs:580` must survive.
- "Capture at spawn time" is load-bearing and documented in three places; the
  shared helper needs a thread-setup hook (e.g. `DiagnosticsOwner` adoption).
- Fallback strings for panic payloads are asserted in tests.
- Do not drop the payload at `src/repair/phases/metadata.rs:125-129`.

## Commit breakdown

1. Add leaf with window, slot restoration and payload recovery + tests.
2. Adopt in `tools/capture/http/src/tasks.rs`, keeping `JoinOutcome` → `ApiError` mapping local.
3. Adopt in repair `scan`, `phases/check` and `persistent_exec` with their own failure policies.
4. Adopt in the file-task driver with the dispatcher/setup hook.
5. Adopt payload-preserving downcast in `metadata` and the other downcast sites.

## Validation

- Panic-path and failure-policy tests in each adopting phase.
- Log/dispatcher assertions; `cargo nextest` across root, capture and util-path.

## Done when

One leaf owns the window and payload recovery, and no caller has lost its
dispatcher, cancellation semantics or failure policy.
