# OPTIMIZATIONS_PLAN — index & how-to

Temporary planning workspace for **reducing the scale** of the test suite across
this cargo workspace (fewer test functions, fixtures, files, process spawns and
synthetic payloads — not runtime). The goal and the *why* live in the read-only
report `docs/09-test-scale-reduction-research.md` ("the report"). The plans here
contain only the *how*.

This directory is **not** shipped code. When the work is complete, delete this
whole directory and the report in a final commit, and make sure no implementation
file refers to either of them.

This is the hand-off package for an orchestrator that may be replaced between
steps. A replacement starts from the files here, not from any prior conversation.

## Files

| File | Role | Read-only? |
|---|---|---|
| `README.md` | This index, the task roster and checkpoint table. | yes |
| `ORCHESTRATION.md` | The process: roles, per-task loop, review checkpoints, follow-ups, resumability, completion. **Read this first.** | yes |
| `STATE.md` | The committed checklist and single source of truth. Owned and updated by the orchestrator. | no |
| `tasks/Tnn-*.md` | One implementation plan per task. Each may describe several commits. Hand exactly one task file to exactly one implementing session. | yes |

Only `STATE.md` is mutable; the report, this index, `ORCHESTRATION.md` and the
task files are read-only references. Task order and review checkpoints are
seeded in `STATE.md`. Do **not** reorder silently — record any change there.

## How a session is handed off

Give an implementing or review session **only pointers**, never pasted prose:

1. its task file path (`OPTIMIZATIONS_PLAN/tasks/Tnn-*.md`);
2. the report line ranges in that file's **Pointers** block;
3. the relevant role section of `ORCHESTRATION.md` ("Implementing session" or
   "Review-and-fix session").

For a review session also pass the commit ranges recorded in `STATE.md`. The
session reads the pointers, validates them against the current code and works
from there. See `ORCHESTRATION.md` for the full loop, the separate
`pending_review` state commit, the review batches and the follow-up rules.

## Task roster

Ordered by the report's suggested sequencing (§L810–L824): P0 low-risk / safe
first, then P1 narrow production seams, then P2 wide high-payoff seams.

| ID | Task | Findings | Area | Priority | CP |
|---|---|---|---|---|---|
| T01 | `retry.rs` injectivity sampling → coprimality proof | C1 | root unit | P0 | CP1 |
| T02 | Root-unit low-risk merges: `plan.rs` layout + `clock_support` | C3, C4 | root unit | P0 | CP1 |
| T03 | sled2redb low-risk merges: stale-file fixture + destination/temp case | D3, D4 | sled2redb | P0 | CP1 |
| T04 | Small-crates table folds: `crates/log` + `crates/tempdir` | D5, D6, D7 | log, tempdir | P0 | CP1 |
| T05 | Capture: reduce full captures 3→1 (Ceiling + GzipThreshold variants) | B1, B5 | capture | P0 | CP2 |
| T06 | Capture: device-name flag refusal → config unit test | B2 | capture | P0 | CP2 |
| T07 | Capture: downscale `m7_assembly` megabyte bodies | B3 | capture | P0 | CP2 |
| T08 | Sync smaller findings: helper meta-tests + fixture trims | E6 | girsync | P0 | CP2 |
| T09 | `repair_machinery.rs` in-process `Summary` downshift | A1 | root integration | P1 | CP3 |
| T10 | `repair_plan.rs` audio-plan partial downshift | A3 | root integration | P1 | CP3 |
| T11 | Inject the MD5 buffer; shrink 513 KiB files | C2 | root unit / prod | P1 | CP4 |
| T12 | Pure `DiskPlan::from_figures` seam | C5 | root unit / prod | P1 | CP4 |
| T13 | Capture: merge mock / no-network full captures | B4 | capture | P1 | CP5 |
| T14 | `repair_purge.rs` plan-out + purge-only merges | A7 | root integration | P1 | CP5 |
| T15 | Parameterize the sync heartbeat period | E4 | girsync / prod | P1 | CP6 |
| T16 | Sync injectable record sink + `run_captured` | E1 | girsync / prod | P2 | CP7 |
| T17 | Sync scoped subscriber + injectable console writer | E2 | girsync / prod | P2 | CP8 |
| T18 | Sync injected `TestHooks` for ambient env hooks | E5 | girsync / prod | P2 | CP8 |
| T19 | Richer `CompareOutcome`, delete duplicated pipelines | E3 | girsync / prod | P2 | CP9 |
| T20 | `repair_scan.rs` estimate/mode downshift + merge | A2 | root integration | P2 | CP10 |
| T21 | `peak_heap.rs` measurement → scale-free invariant | A5 | root integration | P2 | CP10 |
| T22 | Expose `Args` + conversion record from sled2redb lib | D1 | sled2redb / prod | P2 | CP11 |
| T23 | Split `convert_into` at the sled-read / redb-write boundary | D2 | sled2redb / prod | P2 | CP11 |

Report items that are **not** tasks: `B6` (in-process capture seam), `D8`
(sled2redb help pin), and E3's optional `lazy/compare.rs` table-izing are seeded
as follow-ups in `STATE.md`. `E0` is the rationale for the sync work, not a task.
The report's "nothing new / non-findings / leave alone" sections (§A L193–L200,
§B L332–L340, §C L461–L466, §D L593–L607, and the constraints at L826–L841) are
deliberately not tasks.

## Review checkpoints

Sized by amount of work, not task count; large tasks sit alone or in pairs.

| CP | Tasks | Theme |
|---|---|---|
| CP1 | T01–T04 | P0 root unit + small crates + sled2redb low-risk |
| CP2 | T05–T08 | P0 capture bundle/body reductions + sync small findings |
| CP3 | T09–T10 | `repair_machinery` + `repair_plan` downshifts (large) |
| CP4 | T11–T12 | root-unit production seams (MD5, DiskPlan) |
| CP5 | T13–T14 | capture mock merge + `repair_purge` merges |
| CP6 | T15 | sync heartbeat parameterization |
| CP7 | T16 | sync record sink + `run_captured` (largest single change) |
| CP8 | T17–T18 | sync logging + `TestHooks` seams |
| CP9 | T19 | sync `CompareOutcome` pipeline dedup |
| CP10 | T20–T21 | `repair_scan` downshift + `peak_heap` invariant |
| CP11 | T22–T23 | sled2redb production seams |

The orchestrator may move a boundary if the actual amount of work demands it;
record the change in `STATE.md`.
