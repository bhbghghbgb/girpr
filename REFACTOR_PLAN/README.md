# REFACTOR_PLAN — package guide

This directory is a **temporary planning package**. It and
`docs/09-shared-code-refactor-findings.md` are deleted when the refactor is
finished. No code, test, or permanent documentation may reference either. Any
rationale that must survive is written self-contained at its call site.

## Layout

| File | Role |
| --- | --- |
| `README.md` | This guide: structure, ordering, coverage, checkpoints, rules. |
| `ORCHESTRATION.md` | The orchestration protocol. Read before orchestrating. |
| `STATE.md` | The committed checklist. Single source of truth for progress. |
| `tasks/T##-*.md` | One implementation plan per task. A task file is the only plan an implementing session needs. |

## How a task file is used

A task file contains **only the implementation plan** for that task. It does not
restate the research. It names:

- the goal and the boundary (the leaf/crate to add or widen),
- the exact report line ranges to read (`docs/09-shared-code-refactor-findings.md:<lines>`),
- guardrails: program policy that must stay at call sites,
- a proposed commit breakdown (one task may be several commits),
- validation guidance and a done condition.

An implementing session is given its task file **by path** plus those specific
report line ranges. It is not given the whole report or the whole package. It
validates every claim against the current tree before editing: if the task is
stale, already done, or blocked, it stops and the task is recorded as skipped.
It may improvise where a better boundary is found; the goal is the optimization,
not literal compliance with the proposed commits.

## Ordering and dependencies

Tasks run one at a time in index order. The order groups work by boundary so a
later task can build on an earlier leaf. The important dependencies are:

- `T01` before `T02` (range/tiling uses the arithmetic primitives).
- `T03` before `T04` (both widen `girpr-digest`).
- `T05` before `T06` (retry driver sits above the shared status predicate).
- `T15` before `T18` (report planes adopt the shared `OutputFormat`).
- `T16`/`T18` touch the three report planes; do them before capture-specific DTO work (`T21`).

If a dependency turns out to be unnecessary, the implementing agent may say so;
the orchestrator records the decision in `STATE.md` rather than reordering blindly.

## Coverage map

Every finding in the report is assigned once. `X` findings are umbrellas; their
area findings point at the same task. A finding marked *non-candidate* has no
task and must not be "fixed" as a side effect.

| Finding | Report lines | Task |
| --- | --- | --- |
| X1 | 72-86 | T16 |
| X2 | 88-100 | T17 |
| X3 | 102-114 | T18 |
| X4 | 116-124 | T05 |
| X5 | 126-140 | T07 |
| X6 | 142-149 | T11 |
| X7 | 151-159 | T09 |
| X8 | 161-174 | T01, T02 |
| X9 | 176-187 | T03 |
| X10 | 189-200 | T12 |
| X11 | 202-215 | T13 |
| X12 | 217-236 | T19, T20 |
| X13 | 238-247 | T14 |
| X14 | 249-259 | T24 |
| R-01 | 265-274 | T07 |
| R-02 | 276-283 | T07 |
| R-03 | 285-293 | T10 |
| R-04 | 295-298 | T11 |
| R-05 | 299-308 | T15 |
| R-06 | 310-313 | T09 |
| R-07 | 315-322 | T19 |
| R-08 | 324-332 | T19 |
| R-09 | 334-340 | T19 |
| R-10 | 342-347 | T07 |
| R-11 | 349-355 | T19 |
| R-12 | 357-365 | T20 |
| R-13 | 367-373 | T20 |
| R-14 | 375-381 | non-candidate |
| APP-01 | 387-390 | T11 |
| APP-02 | 392-401 | T08 |
| APP-03 | 403-405 | T05 |
| APP-04 | 407-411 | T03 |
| APP-05 | 412-420 | T09 |
| APP-06 | 422-426 | T13 |
| APP-07 | 428-433 | T12 |
| APP-08 | 435-442 | T02 |
| APP-09 | 444-448 | T19 |
| APP-10 | 450-454 | T19 |
| APP-11 | 456-459 | T19 |
| APP-12 | 462-469 | T10 |
| APP-13 | 472-479 | T06 |
| APP-14 | 481-486 | T20 (optional helper only) |
| APP-15 | 488-492 | T24 |
| APP-16 | 494-499 | T15 |
| APP-17 | 501-506 | T02 |
| APP-18 | 508-512 | T15 |
| APP-19 | 514-519 | T15 |
| LF-01 | 525-533 | T04 |
| LF-02 | 535-541 | T04 |
| LF-03 | 542-549 | T01 |
| LF-04 | 551-555 | T03 |
| LF-05 | 557-561 | T14 |
| LF-06 | 563-567 | T19 |
| LF-07 | 569-573 | T03 |
| LF-08 | 575-582 | T13 |
| LF-09 | 584-590 | T15 |
| LF-10 | 592-598 | T02 |
| LF-11 | 600-606 | T15 |
| LF-12 | 608-613 | T04 |
| LF-13 | 615-619 | T17 |
| LF-14 | 621-626 | T17 |
| LF-15 | 628-635 | T03 |
| SYNC-01 | 640-644 | T16 |
| SYNC-02 | 646-650 | T16 |
| SYNC-03 | 652-657 | T18 |
| SYNC-04 | 659-663 | T17 |
| SYNC-05 | 665-670 | T20 |
| SYNC-06 | 672-677 | T14 |
| SYNC-07 | 679-684 | T19 |
| SYNC-08 | 686-690 | T15 |
| SYNC-09 | 692-696 | T20 |
| SYNC-10 | 698-703 | T18 |
| SYNC-11 | 705-710 | T04 |
| SYNC-12 | 712-716 | T23 |
| SYNC-13 | 718-721 | T24 |
| SYNC-14 | 723-728 | already correct — no task |
| CAP-01 | 733-739 | T12 |
| CAP-02 | 742-746 | T21 |
| CAP-03 | 748-750 | T05 |
| CAP-04 | 752-756 | T07 |
| CAP-05 | 758-763 | T02 |
| CAP-06 | 765-771 | T12 |
| CAP-07 | 773-779 | T13 |
| CAP-08 | 781-786 | T13 |
| CAP-09 | 788-794 | T21 |
| CAP-10 | 796-799 | T17 |
| CAP-11 | 801-808 | T16 |
| CAP-12 | 810-815 | T16, T18 |
| CAP-13 | 817-821 | T22 |
| CAP-14 | 823-827 | T22 |
| CAP-15 | 829-835 | T20 |
| CAP-16 | 836-842 | T20 |
| CAP-17 | 844-849 | T12 |
| CAP-18 | 851-857 | T22 |
| CAP-19 | 859-862 | T24 |
| CAP-20 | 864-869 | non-candidate |
| Non-findings / keepers | 873-910 | do not change |

The report's LF-14 entry cross-references X14, but that is a pointer to the
manifest finding, not a grouping: LF-14 is handled with the other `girpr-log`
internals in T17, while X14 itself is T24.

## Checkpoints

A review-and-fix session runs after each checkpoint, before the next task starts.
Batch size is deliberately small so the reviewer is not overwhelmed.

| Checkpoint | After task | Reviews tasks |
| --- | --- | --- |
| CP1 | T04 | T01–T04 |
| CP2 | T07 | T05–T07 |
| CP3 | T11 | T08–T11 |
| CP4 | T14 | T12–T14 |
| CP5 | T16 | T15–T16 |
| CP6 | T18 | T17–T18 |
| CP7 | T20 | T19–T20 |
| CP8 | T22 | T21–T22 |
| CP9 | T24 | T23–T24 |

A checkpoint may also be inserted early if `STATE.md` shows an unusual amount
of work or a runaway follow-up list. See `ORCHESTRATION.md`.

## Non-candidates — do not create tasks

- R-14 minor duplications with no credible extraction.
- SYNC-14 areas where sharing is already correct.
- CAP-20 Python/Rust class-set gap (deliberate measurement instrument).
- The "Non-findings and explicit keepers" section of the report (873-910).
- APP-14's duplicated canned-server wrappers are only worth an optional
  `script_bodies` helper; the stated intent to keep them local is respected.

## Rules for every session

1. Validate against the current code before editing; findings are static
   readings and may be stale.
2. Stop and report instead of forcing a task that is blocked or already done.
3. Program contracts are policy: published bytes, `--help` text, exit codes,
   field order, golden URLs and error wording are not converged by accident.
4. Suggest follow-ups rather than implementing them unless your session is the
   designated follow-up implementer.
5. Never reference these plan files or the report from code or permanent docs.
