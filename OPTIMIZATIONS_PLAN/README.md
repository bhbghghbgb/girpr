# OPTIMIZATIONS_PLAN — index & how-to

Temporary planning workspace for reducing the **test count** across this cargo
workspace. This directory is **not** shipped code: once the work is complete,
delete this whole directory and `docs/08-test-suite-reduction-audit.md` in a
final commit, and make sure no source file refers to either of them.

This is the hand-off package for an orchestrator that may be replaced between
steps. A replacement starts from the files here, not from any prior conversation.

## Files

| File | Role |
|---|---|
| `ORCHESTRATION.md` | The process: roles, per-task loop, review checkpoints, follow-ups, resumability, completion. **Read this first.** |
| `STATE.md` | The committed checklist and single source of truth. Owned and updated by the orchestrator; task files and the report are read-only. |
| `tasks/Tnn-*.md` | One implementation plan per task. Each is read-only and may describe several commits. Hand exactly one task file to exactly one implementing session. |

Goal, rationale and the *why* live in the read-only report:
`docs/08-test-suite-reduction-audit.md` (referred to below as "the report"). Task
order is seeded in `STATE.md`: the safe P0 merges/removes first (report steps 1–2),
then the medium-risk, verification-required P1 work (report step 3). Do **not**
reorder silently — record any change in `STATE.md`.

## Task roster

| ID | Task | Area | Priority | Checkpoint | Report |
|---|---|---|---|---|---|
| T01 | Merge `cli_contract.rs` helper/layout/parser self-tests | root integration | P0 | CP1 | §B.1 1–3 (L85–94) |
| T02 | Remove duplicated fixture-helper meta-tests | root unit | P0 | CP1 | §C.1 (L119–128) |
| T03 | Consolidate remaining low-risk root-unit cases | root unit | P0 | CP2 | §C.2–4 (L129–137) |
| T04 | Table-drive sync hash dedup/order asserts | girsync | P0 | CP2 | §D.2 (L154–156) |
| T05 | Remove capture no-op test; merge same-function pairs | gircap | P0 | CP3 | §E.1, §E.4 (L177–191) |
| T06 | sled2redb removes + usage-error parametrization | sled2redb | P0 | CP3 | §F.1–3 (L208–217) |
| T07 | `crates/log` + `tempdir` low-risk merges | log, tempdir | P0 | CP3 | §F.4–5 (L218–222) |
| T08 | `repair_plan` scope table + optional root-integration merges | root integration | P0 | CP3 | §B.3–4 (L107–115) |
| T09 | Consolidate `run_lock.rs` case clusters | root integration | P1 | CP4 | §B.2 (L101–106) |
| T10 | Consolidate `purge.rs` classifier early tests | root unit | P1 | CP4 | §C.5 (L138–142) |
| T11 | Trim `migration_lock.rs` clear duplicates | girsync | P1 | CP5 | §D.4 (L159–164) |
| T12 | Capture: remove redundant process test; fold assertion-free test | gircap | P1 | CP5 | §E.2–3 (L181–187) |
| T13 | Cross-binary log-file policy reduction | all binaries | P1 | CP5 | §A, §B.1 4–5, §D.1/3 (L50–73, L95–100, L152–158) |

The report's **"leave alone"** items (§C.6, §D.6, §E.6, the `repair_*` invariants,
sync race/guard suites, the 12 `#[ignore]`d tests) are deliberately **not**
tasks. Report items with no or minimal test-count effect are seeded as follow-ups
in `STATE.md` instead.

Review checkpoints (sized by amount of work, not task count):

| CP | Tasks | Theme |
|---|---|---|
| CP1 | T01–T02 | root CLI self-tests + root-unit fixture dedup |
| CP2 | T03–T04 | remaining root-unit merges + sync hash table |
| CP3 | T05–T08 | capture, sled2redb, small crates, root-integration merges |
| CP4 | T09–T10 | `run_lock.rs` + `purge.rs` (verification-required) |
| CP5 | T11–T13 | `migration_lock.rs`, capture medium, cross-binary log policy |

## How a task is handed off

Give an implementing session **only**:

1. its task file path (`OPTIMIZATIONS_PLAN/tasks/Tnn-*.md`);
2. the report line pointers in that file's **Pointers** block;
3. the "Implementing session" section of `ORCHESTRATION.md`.

Do **not** paste the plan or the report into the session. The session reads the
pointers, validates them against the current code, and implements. See
`ORCHESTRATION.md` for the full loop, the separate `pending review` commit, the
review batches and the follow-up rules.
