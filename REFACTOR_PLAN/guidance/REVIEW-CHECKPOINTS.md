# Review checkpoints

A review-and-fix session reviews one batch at a time. Batches are sized by **amount of
work**, so a reviewer is never overwhelmed; a batch that turns out larger than expected
gets split or its own review. These groupings are a starting proposal — the orchestrator
adjusts them based on the actual state and may call a review at any time, especially to
prune follow-ups that are running away.

| Checkpoint | Tasks | Why grouped (work size / risk) |
|---|---|---|
| A | T01, T02 | Two girsync-owned production crates (§F3, §F7, §F8); verify the direct sled2redb edge and `postcard` wire vocabulary survive the cache move, and that both lint tables switch correctly |
| B | T03 | The test-only inversion crate (§F3, §F6, §F8) — one focused review of the `→ girsync` inversion, both dev-dep edges and the two `tests/common/mod.rs` shims |
| C | T04 | Subtree establishment plus six low-level leaves (§F4, §F7, §F8) — large: verify the chosen subtree is recorded, names/lints unchanged, and every path dependency and doc seam moved |
| D | T05 | Five mid-layer product crates (§F4, §F6, §F8) — large on its own: verify `repair-plan → fetch` stays inside the project and feature wiring is intact |
| E | T06, T07 | Two small test-only crates plus the final classification sweep (§F4, §F5, §F7, §F8, §F9) — verify the `test-support → root` inversion, that no shared crate was touched, and that no plan/report references survived |

## Rules

- Review the whole `pending_review` set at a checkpoint in **one** review session,
  passing the relevant report/task sections and the commit refs from `STATE.md`.
- The reviewer may fix or revert any task in the batch, set its final state, and record
  follow-ups; it does not implement follow-ups. It checks especially that no shared crate
  was moved or renamed (§A6, §F5, §F9), every consumer edge and the wire vocabulary
  survived (§F6), the test-only inversions survived (§F6), lint tables follow the owner
  (§F7), every §F8 seam was updated, gircap's crates are untouched (§F9), and no
  committed artifact references this package or the report.
- A validated `skipped` outcome for a stale or unsafe move is correct and is not a failed
  review; do not pressure such a task into a change.
- After every task is settled, run a **follow-up planning review** first to decide how to
  batch follow-ups and which to drop, then alternate implement and review sessions over
  those batches.
- If follow-up count keeps growing, the orchestrator may call an extra review session
  purely to reduce/prune the list. The orchestrator itself does not research the code.
