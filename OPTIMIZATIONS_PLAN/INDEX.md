# Optimization plan index

This directory is a temporary execution plan for `OPTIMIZATIONS_REPORT.md` in
the repository root. It is deleted, together with the report, when the work is
finished. Nothing in permanent code, tests, comments, docs, or commit messages
may refer to this directory, the report, a task id, or a review batch id.

- `INDEX.md` - this file: task map, dependencies, review checkpoints.
- `GUIDE.md` - rules shared by implementers, reviewers, and the orchestrator.
- `ORCHESTRATION.md` - the orchestrator protocol and session prompt templates.
- `STATE.md` - the committed checklist/state file. The orchestrator owns it.
- `tasks/*.md` - one implementation task per file.

## Ordering

The checklist follows the report's **Recommended overall ordering**
(`OPTIMIZATIONS_REPORT.md` lines 1152-1190). The report's twelve overall steps
are expanded into ordered tasks below. Where a finding is not named in that
overall list, it is inserted into the nearest step and called out here:

- `C7` (path-key folding) joins step 1 with the other small correctness fixes.
- `C9` (lock recovery) is part of the `C2` lock redesign task (`T17`).
- `P5` (sync I/O in async tasks) follows `P4`, as the report requires.
- `P12` (PathRoot leaf resolution) joins the step-8 cleanup, with a
  decision gate because it weakens a containment guarantee if done carelessly.
- `A3`, `A4`, `A5` join the step-7 API/behaviour decisions with `A1`/`A2`.
- `D4`, `D5` are not named in the overall list; they follow `D2` inside the
  step-9 structural refactors.
- `B7` is included as a conditional task. Its default outcome is "do not
  change without a measured win"; it may be dropped with a reason.
- `P14` and the deliberate choices in lines 1192-1216 are **not tasks**. Do not
  change them.

## Task map

`Batch` is the review checkpoint that reviews the task. Dependencies are
ordering constraints, not a suggestion to combine sessions.

| Task | Finding | Report lines | Depends on | Batch |
|---|---|---|---|---|
| T01 | C1 closed stdout pipe | 73-93 | - | RC-01 |
| T02 | C5 oversized JSON number | 165-183 | - | RC-01 |
| T03 | C6 meter underflow | 185-203 | - | RC-01 |
| T04 | C7 Windows path key | 205-225 | - | RC-01 |
| T05 | C3 partial audio target | 121-142 | - | RC-02 |
| T06 | C4 non-atomic `--plan-out` | 144-163 | - | RC-02 |
| T07 | C8 temp-file naming | 227-244 | - | RC-02 |
| T08 | P1 metadata concurrency | 276-299 | - | RC-03 |
| T09 | P2 manifest concurrency | 301-327 | - | RC-03 |
| T10 | P4 blocking phases / check parallelism | 358-384 | - | RC-04 |
| T11 | P5 runtime workers for file tasks | 386-408 | T10 | RC-04 |
| T12 | P6 verify hashing | 410-434 | - | RC-05 |
| T13 | P7 temp reuse handles | 436-457 | - | RC-05 |
| T14 | P8 chunk sink seek | 459-478 | - | RC-05 |
| T15 | P10 shared local chunk map | 502-521 | - | RC-05 |
| T16 | P3 reuse `--scan` verdicts | 329-356 | T12, T15 | RC-06 |
| T17 | C2 + C9 single-writer lock | 95-119, 246-264 | - | RC-07 |
| T18 | A1 public surface | 839-858 | T17 | RC-07 |
| T19 | A2 `--jobs` semantics | 860-878 | T11 | RC-07 |
| T20 | A3 `--dry-run` post-phase | 880-902 | - | RC-08 |
| T21 | A4 emission ownership | 904-923 | T01 | RC-08 |
| T22 | A5 exit and cleanup ownership | 925-943 | T21 | RC-08 |
| T23 | P9 log buffering | 480-500 | T22 | RC-09 |
| T24 | P11 purge/scope allocations | 523-546 | - | RC-09 |
| T25 | P13 meter fast path | 572-591 | T03 | RC-09 |
| T26 | P12 PathRoot leaf resolution | 548-570 | T17 | RC-09 |
| T27 | D1 per-file facts | 619-643 | T15, T16 | RC-10 |
| T28 | D3 one atomic writer | 672-688 | T05, T06, T07 | RC-11 |
| T29 | D2 one retry scaffold | 645-670 | T09 | RC-11 |
| T30 | D4 rel-path helpers | 690-706 | - | RC-12 |
| T31 | D5 verdict remap | 708-723 | T16, T27 | RC-12 |
| T32 | D7 documentation sweep | 745-770 | T09, T16, T20 | RC-13 |
| T33 | D6 test-only API / dead parameter | 725-743 | T20 | RC-13 |
| T34 | D8 function decomposition | 772-792 | T27, T28, T29 | RC-14 |
| T35 | D9 parameter structs | 794-811 | T13, T29 | RC-15 |
| T36 | D10 named exit codes | 813-830 | T22 | RC-15 |
| T37 | B1 tokio features | 952-969 | - | RC-16 |
| T38 | B2 drop `regex` | 971-989 | - | RC-16 |
| T39 | B3 use `futures-util` | 991-1005 | - | RC-16 |
| T40 | B4 workspace default members | 1007-1025 | - | RC-16 |
| T41 | B5 release profile | 1027-1042 | - | RC-16 |
| T42 | B6 workspace lints | 1044-1058 | - | RC-16 |
| T43 | B7 md5 upgrade (conditional) | 1060-1076 | - | RC-16 |
| T44 | T1 failure-path regressions | 1082-1100 | T01-T31 | RC-17 |
| T45 | T2 phase timing | 1102-1116 | T44 | RC-17 |
| T46 | T4 golden output ordering | 1135-1150 | T44, T45 | RC-18 |
| T47 | T3 split the large integration test | 1118-1133 | T46 | RC-18 |

## Review checkpoints

A checkpoint is reached when every task in the batch is `pending_review` (or
the remaining tasks in it are `skipped`, with the orchestrator's reason). Batch
sizes are deliberately uneven: a single large refactor gets its own review,
and a group of tiny build changes is reviewed together.

| Batch | Tasks | Why this size |
|---|---|---|
| RC-01 | T01-T04 | Four small, independent correctness fixes; one contract review. |
| RC-02 | T05-T07 | Three "never publish a partial file" changes; one writer review. |
| RC-03 | T08-T09 | Metadata and manifest concurrency are the same class of risk. |
| RC-04 | T10-T11 | Blocking work and the runtime that runs it; review together. |
| RC-05 | T12-T15 | Four hot-path I/O/CPU/memory changes with narrow blast radii. |
| RC-06 | T16 | One large cross-module change: scan verdict reuse and its guard. |
| RC-07 | T17-T19 | Lock semantics plus the two API decisions that depend on it. |
| RC-08 | T20-T22 | Dry-run behaviour, one emitter, one exit path. |
| RC-09 | T23-T26 | Logging and allocation cleanups, including the P12 decision gate. |
| RC-10 | T27 | D1 touches many modules and tests; review alone. |
| RC-11 | T28-T29 | The two structural helpers (atomic write, retry). |
| RC-12 | T30-T31 | Two small deduplications. |
| RC-13 | T32-T33 | Documentation honesty and removal of test-only production API. |
| RC-14 | T34 | Function decomposition; review for accidental behaviour change. |
| RC-15 | T35-T36 | Signature and exit-code refactors across call sites. |
| RC-16 | T37-T43 | Independent build/dependency changes; one build review. |
| RC-17 | T44-T45 | Regression tests and the timing seam they use. |
| RC-18 | T46-T47 | Output-contract golden test, then the mechanical test split. |

After RC-18, the follow-up phase starts (see `ORCHESTRATION.md`).

## Non-tasks

- `P14` (lines 593-609): chunks stay sequential per file. No task, no change.
- The "Deliberate choices" section (lines 1192-1216): keep every listed choice.
- `B7` may end as a recorded decision with no code change. That is a successful
  outcome, not a skipped task, when the benchmark in the task file says so.
