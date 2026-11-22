# `REFACTOR_PLAN` — shared-crate ownership alignment plan

This is the planning package for the ownership research recorded in
`docs/shared-crate-ownership-findings.md` (referred to everywhere below as
**the report**).

The report classifies all 35 `crates/` members by program family (§F1–F2) and
identifies which are genuinely shared and which are single-project (§F3–F4). It
contains only assumptions, findings and explicit non-findings — no task list,
sequencing or implementation steps (lines 3–4). This package adds only the
implementation plans and the rules for carrying them out; every task points back
to report sections instead of copying them.

The goal of this round is to make crate **location, name, prefix and lint
policy** agree with actual ownership:

- the three girsync-only `girpr-sync-*` crates become girsync-owned — renamed
  `girsync-*` and moved under `tools/sync/` (§F3, §F7);
- the thirteen girpr-only crates move into one girpr-owned subtree at the
  workspace root, keeping their `girpr-` names and the workspace lint table
  (§F4, §F7);
- the nineteen genuinely shared crates stay in `crates/` with unchanged names
  (§A6, §F5, §F9);
- gircap's three already-correct crates under `tools/capture/` are not touched
  (§F9).

> This directory and the report are **deleted when the work is finished**.
> Implementation code, tests, comments, manifests, doc tables and commit
> messages must therefore never refer to `REFACTOR_PLAN/` or to
> `docs/shared-crate-ownership-findings.md`. Every plan file points outward to
> the report only so an implementing session can find the goal; no shipped
> artifact depends on those files.

## Layout

```text
REFACTOR_PLAN/
  README.md                      this file — index, structure, how to read a task
  ORCHESTRATION.md               the orchestration plan (state machine, checkpoints)
  STATE.md                       the committed check-list state (single source of truth)
  tasks/
    T01-sync-cache-ownership.md       move/rename `girpr-sync-cache`
    T02-sync-leaves-ownership.md      move/rename `girpr-sync-leaves`
    T03-sync-test-support-ownership.md move/rename `girpr-sync-test-support`
    T04-girpr-subtree-foundations.md  establish the girpr subtree + low-level leaves
    T05-girpr-subtree-product-crates.md move the mid-layer product crates
    T06-girpr-subtree-test-crates.md  move the two test-only crates
    T07-ownership-consistency-sweep.md final classification and seam sweep
  guidance/
    CONVENTIONS.md               commit/state conventions, pointer rules, skip/follow-up rules
    DO-NOT-TOUCH.md              findings that stay; what is not a task
    REVIEW-CHECKPOINTS.md        how tasks are grouped into review batches
```

## How to read a task file

Every `tasks/Txx-*.md` has the same shape:

- **Goal / pointers** — the exact report sections (§ and line numbers) that are
  the detailed brief, plus the assumptions to validate first and any keep items.
- **Scope** — the crate(s), manifests and doc files named by the report. These
  are *hints*; the implementing session locates them in the current tree before
  editing.
- **Implementation plan** — a short outline of the intended change. It is not
  exhaustive and not carved in stone; the implementing session does the detailed
  research and validation. A task may be delivered in more than one commit.
- **Constraints / validation** — the caveats that must survive (consumer edges,
  wire format, test-only inversions, lint tables, …).
- **Done when** — the acceptance check, including what to do when validation
  fails.

The implementing session is expected to validate every premise against the
current code. Report items are static claims taken on one tree (§A1–A7, lines
8–39); line numbers may have drifted. If a task is stale, contradicted by the
code, would move or rename a genuinely shared crate (§A6, §F5, §F9), would drop
a direct consumer edge or wire vocabulary (§F6), would collapse a test-only
inversion (§F6), or cannot be bounded safely, the session **stops** and the task
is recorded as `skipped` with a one-line reason. If a better ownership boundary
presents itself, the session may improvise as long as the goal — ownership and
layout agree — is met.

Recurring constraints drawn from the report that apply to nearly every task:

- **A2 (lines 14–18):** `sled2redb` belongs to the girsync family; a crate whose
  consumers are only girsync + sled2redb is girsync-owned, not shared. Dev
  dependency edges count (A3, lines 19–23).
- **A4 (lines 24–29):** a project-owned crate lives under its owning project's
  directory, carries that project's name prefix, and uses the tools-only lint
  table when the owner is a dev tool. This is the convention the move must
  satisfy.
- **A6 (lines 33–35):** renames are consistency consequences of A4. No genuinely
  shared crate is renamed, and the report does not assume one.
- **A7 (lines 36–39):** a physical move also changes workspace `members`,
  dependent path dependencies, `Cargo.lock` package entries and path text in
  READMEs and doc comments. Missing any of these is an incomplete task.
- **F6 (lines 125–139):** dependency-direction facts that must survive the move:
  the direct sled2redb → cache edge, the cache's pinned `postcard` digest
  vocabulary, the `girpr-test-support` → root test-only inversion, and the
  `girpr-sync-test-support` → girsync inversion.
- **F8 (lines 150–162):** the full blast radius, including the thin
  `tests/common/mod.rs` shims in the root, girsync and sled2redb test trees.

## Task order

Task IDs define the recommended pick-up order used by the orchestrator (T01
first). The order runs the three self-contained girsync ownership moves first,
then establishes the girpr-owned subtree before moving its dependents
(foundation → mid-layer → test-only), then finishes with a consistency sweep.
This is a recommendation, not a hard rule; `STATE.md` is authoritative for what
remains. The girpr subtree's directory name is deliberately not fixed by the
report: T04 chooses it and the orchestrator records the choice in `STATE.md`
before T05/T06 are dispatched.

## Lifecycle at a glance

1. Orchestrator reads `STATE.md`, picks the next `todo` task.
2. A single implementation session handles that one task only and receives
   pointers (report sections + its task file), not a recitation of the plan.
3. In a **separate commit**, the orchestrator moves the task to `pending_review`.
4. When the accumulated `pending_review` set reaches a review checkpoint sized
   by amount of work, one review-and-fix session reviews the batch, fixes or
   reverts, and the orchestrator updates `STATE.md`
   (`completed` / `reverted` / `skipped`) and records any non-blocking follow-ups.
5. After all tasks are settled, a planning review first decides how to batch
   follow-ups and which to drop, then alternate implement and review sessions
   over those batches.
6. Final cleanup deletes this directory and the report; no reference may survive.

See `ORCHESTRATION.md` for the full state machine, `CONVENTIONS.md` for the exact
commit and state-update format, and `DO-NOT-TOUCH.md` before editing anything.
