# `REFACTOR_PLAN` — optimization & correctness implementation plan

This is the planning package for the research recorded in
`docs/OPTIMIZATION_REPORT.md` (referred to everywhere below as **the report**).

The report is a static study of one tree. It contains findings, priorities,
assumptions and a list of deliberately-kept non-findings — but **no task list,
sequencing or implementation steps** (report lines 14–18, §11 lines 832–888,
§12 lines 890–905). This package adds only the implementation plans and the rules
for carrying them out; every task points back to report sections instead of
copying them.

The goal of this round is to turn the report's findings into validated code
changes, grouped so that related work lands together and high-risk changes get
their own review:

- **Scheduling / parallelism** — right-size the Tokio runtime, move file I/O off
  async workers, overlap independent phases (§2, lines 107–198).
- **Disk I/O, hashing & paths** — remove double reads, redundant stats and
  repeated canonicalization (§3, lines 200–293).
- **Memory & allocation** — bound decoder buffers, reuse scratch buffers, stop
  zero-filling read buffers (§4, lines 295–345).
- **Network correctness** — timeouts, status checks, retry classification,
  decode ceilings and blob-id validation (§5, lines 347–413).
- **Correctness / fail-fast / diagnostics** — exit classes, stale verdicts,
  skipped errors, TOCTOU windows (§6, lines 415–513).
- **Logging** — cut per-chunk log cost without changing the data plane (§7,
  lines 515–560).
- **Micro-optimizations** — bounded, low-risk sweeps (§8, lines 562–637).
- **Dev tools** — girsync, gircap and sled2redb hot paths (§9, lines 639–806).
- **Build surface** — the one build finding worth validating (§10, lines 808–829).

> This directory and the report are **deleted when the work is finished**.
> Implementation code, tests, comments, manifests, docs and commit messages must
> therefore never refer to `REFACTOR_PLAN/` or to
> `docs/OPTIMIZATION_REPORT.md`. Every plan file points outward to the report
> only so an implementing session can find the goal; no shipped artifact depends
> on those files.

## Layout

```text
REFACTOR_PLAN/
  README.md                          this file — index, structure, how to read a task
  ORCHESTRATION.md                   the orchestration plan (state machine, checkpoints)
  STATE.md                           the committed check-list state (single source of truth)
  tasks/
    T01-runtime-sizing-and-blocking-offload.md    S-01, S-02
    T02-intra-file-chunk-parallelism.md            S-03
    T03-parallel-plan-network-phases.md            S-04
    T04-parallel-purge.md                          S-05
    T05-plan-scan-stat-overlap.md                  S-06
    T06-repair-reuse-io.md                         IO-01..IO-04, COR-04
    T07-path-join-canonicalization.md              IO-05, IO-06
    T08-post-audio-cache.md                        IO-07, IO-08
    T09-stream-decode-bounds.md                    MEM-01, NET-05, NET-06
    T10-stream-blocking-offload.md                 NET-07
    T11-http-client-and-fetch-micro.md             NET-01..04, MEM-05, MIC-04/05/12
    T12-reuse-map-validation.md                    COR-03, MIC-01/08/11
    T13-hash-buffer-reuse.md                       MEM-03/04, MIC-09/10
    T14-exit-code-classification.md                COR-01
    T15-scan-and-check-correctness.md              COR-02, COR-05, COR-08
    T16-scope-and-purge-diagnostics.md             COR-06, COR-07, COR-09
    T17-logging-cost.md                            LOG-01..03
    T18-misc-micro-sweep.md                        MIC-02/03/06/07, MEM-02
    T19-gircap-http-pipeline.md                    TOOL-01/07/08/09/10
    T20-gircap-decode-and-serialize.md             TOOL-03/04/11/12
    T21-gircap-index-correctness.md                TOOL-05/06/13/17/19/20
    T22-gircap-parallelization.md                  TOOL-14/15
    T23-gircap-hash-errors.md                      TOOL-21
    T24-girsync-and-sled2redb.md                   TOOL-02/16/18
    T25-build-surface.md                           B-01
  guidance/
    CONVENTIONS.md               commit/state conventions, pointer rules, skip/follow-up rules
    DO-NOT-TOUCH.md              non-findings and contracts that stay unchanged
    REVIEW-CHECKPOINTS.md        how tasks are grouped into review batches
```

## How to read a task file

Every `tasks/Txx-*.md` has the same shape:

- **Goal / pointers** — the exact report sections and line ranges that are the
  detailed brief, plus the assumptions to validate first and any keep items.
- **Scope** — the files and call sites named by the report. These are *hints*;
  the implementing session locates them in the current tree before editing.
- **Implementation plan** — a short outline of the intended change. It is not
  exhaustive and not carved in stone; the implementing session does the detailed
  research and validation. A task may be delivered in more than one commit.
- **Constraints / validation** — the caveats that must survive (wire formats,
  output contracts, deterministic artifacts, assumptions, …).
- **Done when** — the acceptance check, including what to do when validation
  fails.

The implementing session is expected to validate every premise against the
current code. Report items are static claims taken on one tree (report lines
8–13); line numbers may have drifted. If a task is stale, contradicted by the
code, depends on an assumption that does not hold (§11, lines 832–888), would
change a deliberate non-finding (§12, lines 890–905) or a data-plane / artifact
contract, or cannot be bounded safely, the session **stops** and the task is
recorded as `skipped` with a one-line reason. If a better approach presents
itself, the session may improvise as long as the finding's goal is met.

Recurring constraints drawn from the report that apply to many tasks:

- **Static study (lines 8–18):** trust the current code over a stale pointer;
  priorities are H/M/L and categories are perf/mem/io/par/correct/fail/diag/build
  (lines 20–23).
- **Assumptions are gates (§11, lines 832–888):** every task names the assumptions
  that must be confirmed before editing. An unconfirmed assumption means stop,
  not guess.
- **Pairings:** S-01 and S-02 must land together (assumption 1, lines 837–840);
  NET-07 is the same class of problem as S-02 (report lines 404–413, 139);
  IO-01/02/03 share the reuse-verdict / double-read assumptions 3–4 (lines
  843–849).
- **Data plane is stable:** the `REPORT`/`PLAN`/`PROGRESS`/`SUMMARY` stdout
  contract must not be changed by logging work (LOG-01, lines 527–530). T16 is the
  only task that deliberately extends `SUMMARY`, and must keep its machine-readable
  shape stable.
- **Deterministic artifacts:** purge log order (S-05, lines 179–183), girsync's
  pinned `postcard` digest vocabulary (T24), and gircap's recorded request/response
  JSON and artifact bytes (assumptions 14–17, lines 875–888) must survive.
- **Non-findings stay (§12, lines 890–905):** the five "checked and deliberately
  not reported" items, the tuned Cargo profiles (lines 810–816) and B-02 (lines
  825–829) are keepers, not hidden tasks.

## Task order

Task IDs define the recommended pick-up order used by the orchestrator (T01
first). The order runs the high-impact scheduling fixes first, then I/O, memory,
network and correctness in the product binary, then logging/micro sweeps, then
the dev tools, and finally the one build-surface item. It is a recommendation,
not a hard rule; `STATE.md` is authoritative for what remains. A task that turns
out stale or unsafe is skipped, not forced.

## Lifecycle at a glance

1. Orchestrator reads `STATE.md`, picks the next `todo` task.
2. A single implementation session handles that one task only and receives
   pointers (report sections + its task file), not a recitation of the plan.
3. In a **separate commit**, the orchestrator moves the task to `pending_review`.
4. When the accumulated `pending_review` set reaches a review checkpoint sized
   by amount of work, one review-and-fix session reviews the batch, fixes or
   reverts, and the orchestrator updates `STATE.md` (`completed` / `reverted` /
   `skipped`) and records any non-blocking follow-ups.
5. After all tasks are settled, a planning review first decides how to batch
   follow-ups and which to drop, then alternate implement and review sessions
   over those batches.
6. Final cleanup deletes this directory and the report; no reference may survive.

See `ORCHESTRATION.md` for the full state machine, `CONVENTIONS.md` for the exact
commit and state-update format, and `DO-NOT-TOUCH.md` before editing anything.
