# `REFACTOR_PLAN` — test-run optimization plan

This is the planning package for the test-run optimization research recorded in
`docs/10-test-optimization-research.md` (referred to everywhere below as **the report**).

The goal of the work is to reduce the cost of building and running the workspace test
suite: fewer process spawns under nextest, fewer expensive cache opens and fixture
rebuilds, fewer test binaries to link, and narrower rebuild fan-out after an edit. The
report already holds the measurements, findings, trade-offs and assumptions. This
package deliberately contains **only implementation plans and the rules for carrying
them out**; every task points at report sections instead of copying them.

> This directory and the report are **deleted when the work is finished**.
> Implementation code, tests, comments, manifests, doc tables and commit messages must
> therefore never refer to `REFACTOR_PLAN/` or to
> `docs/10-test-optimization-research.md`. Every plan file points outward to the report
> only so an implementing session can find the goal; no shipped artifact depends on
> those files.

## Layout

```text
REFACTOR_PLAN/
  README.md                     this file — index, structure, how to read a task
  ORCHESTRATION.md              the orchestration plan (state machine, checkpoints)
  STATE.md                      the committed check-list state (single source of truth)
  tasks/
    T01-...md ... T21-...md      one implementation plan per task
  guidance/
    CONVENTIONS.md              commit/state conventions, pointer rules, skip/follow-up rules
    DO-NOT-TOUCH.md             findings that must stay; what is not a task
    REVIEW-CHECKPOINTS.md       how tasks are grouped into review batches
```

## How to read a task file

Every `tasks/Txx-*.md` has the same shape:

- **Goal / pointers** — the exact report sections (§ and line numbers) that are the
  detailed brief, plus the assumptions that must be validated first.
- **Scope** — the packages/files named in the report. These are *hints*; the
  implementing session locates them in the current tree before editing.
- **Implementation plan** — a short outline of the intended change. It is not
  exhaustive and not carved in stone; the implementing session does the detailed
  research and validation. A task may be delivered in more than one commit.
- **Constraints / validation** — the caveats that must survive (no sleeps, tiny
  fixtures, pinned nextest filters, byte contracts, cross-program independence, …).
- **Done when** — the acceptance check, including before/after measurement.

The implementing session is expected to validate every premise against the current
code. Report items are static claims taken on a particular host and tree (§1, §6);
line numbers may have drifted. If a task is stale, contradicted by the code, would
regress a deliberately kept policy, or would not actually reduce the relevant cost,
the session **stops** and the task is recorded as `skipped` with a one-line reason. If
a better optimization presents itself, the session may improvise as long as the goal
— cheaper test execution or rebuild — is met.

Two recurring tests appear across tasks:

- **A1 (report §2, lines 46–52):** nextest runs one OS process per `#[test]`. Merging
  test functions saves process spawns; adding parameterized cases adds them. A
  process-wide cache or static fixture never helps across tests.
- **A2/A3 (report §2, lines 54–65):** editing a file invalidates its target and all
  downstream dependents. A split pays off only when a high-level consumer depends on
  the smaller crate directly; a facade that still re-exports both halves is not a win.

## Task order

Task IDs define the recommended pick-up order used by the orchestrator (T01 first).
The order front-loads the highest-value findings from report §4 (lines 471–484), then
continues through build configuration, remaining cross-cutting items, root unit and
integration tests, leaf crates, the sync side and the capture side. It is a
recommendation, not a hard rule; `STATE.md` is authoritative for what remains.

## Lifecycle at a glance

1. Orchestrator reads `STATE.md`, picks the next `todo` task.
2. A single implementation session handles that one task only and receives pointers
   (report sections + its task file), not a recitation of the plan.
3. In a **separate commit**, the orchestrator moves the task to `pending_review`.
4. When the accumulated `pending_review` set reaches a review checkpoint sized by
   amount of work, one review-and-fix session reviews the batch, fixes or reverts, and
   the orchestrator updates `STATE.md` (`completed` / `reverted` / `skipped`) and
   records any non-blocking follow-ups.
5. After all tasks are settled, a planning review first decides how to batch
   follow-ups and which to drop, then alternate implement and review sessions over
   those batches.
6. Final cleanup deletes this directory and the report.

See `ORCHESTRATION.md` for the full state machine, `CONVENTIONS.md` for the exact
commit and state-update format, and `DO-NOT-TOUCH.md` before editing anything.
