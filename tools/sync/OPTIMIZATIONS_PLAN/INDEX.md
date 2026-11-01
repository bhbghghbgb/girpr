# girsync optimization plan — index

This directory turns `tools/sync/OPTIMIZATIONS_REPORT.md` into an ordered,
reviewable work queue. The report says **what** and **why**; the task files say
**how** and **in what order to commit**.

Everything in this directory, and the report itself, is temporary. It is
deleted after the last task and follow-up are reviewed. Nothing added to the
crate may refer to these files.

## Files

| file | purpose |
| --- | --- |
| `INDEX.md` | this file: ordering, batches, review checkpoints |
| `GUIDELINES.md` | rules every implementing and reviewing session must follow |
| `ORCHESTRATION.md` | orchestrator protocol, state machine, spawn contract, prompt templates |
| `STATE.md` | the committed checklist state; only the orchestrator edits it |
| `tasks/T01-…` … `tasks/T19-…` | one implementation plan per task |

## Ordering

The order is the report's **§7 Overall** order, with report entries that §7
omits inserted where the report's own within-category ordering puts them:

- `1.4` (cache meta fast path) goes after `1.2` and before `1.3`, as the
  correctness ordering says.
- `2.2` (phase A one pass) goes after `2.3` and before `2.6`, as the
  performance ordering says.
- `2.7` (micro) goes after `2.6`.
- `4.3` (one test-cache decision) goes before `4.1` inside the cache task.
- `4.4` (failure story) is folded into the backup-policy task because it is
  the same product decision and the same documentation.
- `3.6` is last in the clean-code group, after the changes it has to describe.

## Task queue and review checkpoints

Each row is one implementing session. A checkpoint is one review-and-fix
session covering the rows above it.

| batch | id | task | report | size | risk |
| --- | --- | --- | --- | --- | --- |
| A | T01 | By-name digest lookup and its test | §1.1, §5.1 | S | low |
| A | T02 | Plan-time early returns and prefix memoization | §2.5, §1.6, §5.4 | M | low–medium |
| **A** | | **review checkpoint A** | | | |
| B | T03 | One documented case fold | §1.2, §5.2 | M | medium |
| B | T04 | Shared one-side case index | §2.1 | L | medium |
| B | T05 | Cache meta open fast path | §1.4 | S | low–medium |
| **B** | | **review checkpoint B** | | | |
| C | T06 | Copy-path round trips | §2.4 | S | low–medium |
| C | T07 | Cache scan consolidation | §2.3, §5.3 | S | low–medium |
| C | T08 | Phase A one tree pass | §2.2 | M | medium |
| **C** | | **review checkpoint C** | | | |
| D | T09 | Backup policy and the failure story | §1.3, §4.2, §4.4, §5.3 | M | medium |
| D | T10 | Cache memory knob and one test-cache decision | §4.3, §2.6, §4.1 | S | low |
| D | T11 | Small performance notes | §2.7 | S | low |
| **D** | | **review checkpoint D** | | | |
| E | T12 | Filter bundle and pre-built side labels | §3.1, §3.5 | M | low |
| E | T13 | Decompose `copy_one` around its guards | §3.2 | M | medium |
| **E** | | **review checkpoint E** | | | |
| F | T14 | `sled2redb` uses clap | §3.4 | S | low |
| F | T15 | Named sync phases: decide, do not default to extracting | §3.3 | S | low |
| F | T16 | Reconcile documentation with the code | §3.6 | M | low |
| **F** | | **review checkpoint F** | | | |
| G | T17 | Converter atomicity and failure tests | §1.5, §5.5 | M | medium |
| G | T18 | Feature-gate `sled`, trim `windows-sys` features | §6 | S | low–medium |
| G | T19 | Benchmark harness over three tree shapes | §5.6 | S | low |
| **G** | | **review checkpoint G** | | | |

### Why these checkpoints

- **A** is two small, foundational tasks. Reviewing them together catches the
  lookup invariant and the early-return safety argument before later work
  depends on either.
- **B** is the case-fold/index pair plus the cache meta fast path. The first
  two must be reviewed together because the second consumes the first; the
  meta change is small and cache-local.
- **C** is three medium risk-sensitive tasks: copy safety, cache phase
  boundaries, and phase A inclusion. They are separate files but share the
  "same rows, same order, same refusals" acceptance test.
- **D** contains the one product decision (backups) and two smaller changes.
  It is a single batch because the backup task changes the I/O profile the
  cache knob and micro work are measured against.
- **E** is clean-code work in two files that later tasks touch. Splitting it
  from F keeps the copy-decomposition review focused.
- **F** is three low-risk, mostly mechanical tasks, including the deliberate
  no-op decision for `cmd_sync`.
- **G** is isolated: the converter, the feature gate, and the benchmark
  harness. None is on the mirror's critical path, so one final review is
  enough.

## Direct dependencies

The metadata block of each task is authoritative. This section is the readable
view; if the two disagree, the task file wins.

- T04 depends on T03 (the fold helper).
- T09 depends on T02 only for context (the plan-time early returns); it must
  not revert them.
- T11 depends on T06 (the copy-path `AlgoSet` and worker-result shape).
- T12 depends on T09 and T10 (both add fields to `CommonOpts`, which T12
  reshapes).
- T13 depends on T06 (the copy path after the round-trip work).
- T15 depends on T09 (the backup step it names in the phase table).
- T16 runs last in section 3 because it documents T01–T15.
- T17 is otherwise independent; if T09's atomic sibling claim is reusable, use
  it for the converter's temporary, otherwise add a local helper.
- T18 depends on T14 and T17 (the converter's CLI and file layout are settled
  before it is feature-gated).
- T19 is independent; it may be run before or after its batch, but it is
  scheduled last per §7 Overall.

## What is deliberately not scheduled

These are report items whose recommendation is "do not do this" or "leave it
alone". They are not tasks; a reviewer should reject a change that does them:

- merging `scan_stat_only` and `load_record_side_from`, or making `Side` a
  trait;
- changing lock order, `RwLimiter`, or the three-gate shape;
- making `FileRec.kind` an enum, adding `bitflags`, or unifying the cache and
  request algorithm vocabularies;
- a single-pass copy verify, or making `sync_all` optional in production;
- weakening `plan_pairs`' coverage check or `pair_verdict`'s `unverifiable`
  clause;
- folding the planner's and the diff's `CasePairing` values together;
- changing the `--hash-any-of` tie-break away from flag order;
- gating the diff's classification on `--why`;
- `RwRuntime`/`Gate` changes for performance.

The report's §8 is the long form of this list.

## Definition of done

1. Every task is `completed` or `reverted` in `STATE.md`, with a review record.
2. Every follow-up is `completed`, `reverted`, or explicitly `skipped` with a
   reason in the state.
3. `cargo fmt --all -- --check` and `cargo test -p girsync` pass (plus the
   feature-gated converter command in T18).
4. No source, test, or doc file mentions `OPTIMIZATIONS_REPORT.md`,
   `OPTIMIZATIONS_PLAN`, a task id, or this plan.
5. `git status` is clean after the final deletion commit that removes this
   directory and the report.
