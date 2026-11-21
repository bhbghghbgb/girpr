# Review checkpoints

A review-and-fix session reviews one batch at a time. Batches are sized by amount of
work so a reviewer is never overwhelmed; a batch that turns out larger than expected
gets split or its own review. These groupings are a starting proposal — the
orchestrator adjusts them based on the actual state and may call a review at any time,
especially to prune follow-ups that are running away.

| Checkpoint | Tasks | Why grouped |
|---|---|---|
| A | T01 | Structural: 14 root binaries → 3–4 roots plus `mod common;` change; re-measure links before anything else touches those files (§F1) |
| B | T02 | Dominant wall/I/O cost; validate the memory-cache seam, registry sharing and forget helper (§F30) |
| C | T03, T04 | Both remove root-integration spawns by exposing in-process surfaces; T03 is large (production API) — split if heavy (§F17, §F18) |
| D | T05 | Cross-cutting wiremock replacement across four sites and four manifests; check the mock stays dependency-free (§F2, §F11, §F25) |
| E | T06, T07 | Small build-config changes; a failed linker trial is a legitimate revert (§F4, §F3) |
| F | T08 | Large, cycle-sensitive util split; verify the consumer set actually shrinks (§F6, §F10) |
| G | T09, T10, T11 | Small rebuild edges, ignored-binary gating and dev-dep hygiene (§F6, §F8, §F9, §F14) |
| H | T12, T13 | Root unit/integration test cleanups with multiple commits; check assertions survive (§F12–F16, §F19, §F20, §F23) |
| I | T14, T15 | Integration-test grouping and cross-file assertion dedup; keep process-only coverage (§F21, §F22) |
| J | T16, T17 | Leaf-crate test cleanups; check merge granularity and no byte-contract drift (§F26–F28) |
| K | T18, T19 | Sync minor performance; T18 may be pre-satisfied by T02 — split or skip accordingly (§F31, §F32, §F33) |
| L | T20, T21 | Capture-side test cleanup; check read-only-only switching and no `DO-NOT-TOUCH` drift (§F35, §F36, §F38) |

## Rules

- Review the whole `pending_review` set at a checkpoint in **one** review session,
  passing the relevant report/task sections and the commit refs from `STATE.md`.
- The reviewer may fix or revert any task in the batch, set its final state, and record
  follow-ups; it does not implement follow-ups. It checks especially that a merge
  preserved the semantic assertion, that no test named in `.config/nextest.toml` was
  silently removed, that fixtures were not grown and sleeps not added, and that a
  `DO-NOT-TOUCH` item was not pulled into the change.
- After every task is settled, run a **follow-up planning review** first to decide how
  to batch follow-ups and which to drop, then alternate implement and review sessions
  over those batches.
- If follow-up count keeps growing, the orchestrator may call an extra review session
  purely to reduce/prune the list. The orchestrator itself does not research the code.
