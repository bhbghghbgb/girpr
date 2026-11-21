# Review checkpoints

A review-and-fix session reviews one batch at a time. Batches are sized by
amount of work so a reviewer is never overwhelmed; a batch that turns out larger
than expected gets split or its own review. These groupings are a starting
proposal — the orchestrator adjusts them based on the actual state and may call
a review at any time, especially to prune follow-ups that are running away.

| Checkpoint | Tasks | Why grouped |
|---|---|---|
| A (solo) | T01 | Dependency-inversion enabler (§F3, §F36) — changes edges across all four programs; review the payoff (§A2) before any downstream split |
| B | T02, T03 | Product low-level leaves (§F14, §F19) — check each split has a direct high-level consumer and keeps test-only support local |
| C | T04, T05 | Product mid-level (§F15, §F16) — cycle-sensitive (§F13); check narrow interfaces and that shared types were moved down, not duplicated |
| D | T06, T07, T08 | Product high-level (§F17, §F18, §F22) — largest product edits; check byte contracts (§A6), gircap boundary (§A5) and that narrow tests now link a smaller crate |
| E | T09, T10 | Test-link reduction (§F29, §F32) — check helpers are public-library + thin-shim (§A7), nextest filters updated (§F31), cross-tool independence kept (§F40) |
| F | T11, T12, T13 | girsync splits (§F24) — large; review cache/leaves together, then the planner+effective cycle group (§A10). Split further if the batch is heavy |
| G1 | T14, T15 | gircap http + emit/verify (§F25) — heavy deps and doc-table churn (§A5, §F39); split from blobs/manifest if large |
| G2 | T16, T17 | gircap blobs/manifest + small leaves (§F25) — check `zstd_measure` pair stays duplicated (§A13) and gircap stays logic-free |
| H | T18, T19 | Shared-crate splits (§F34, §F35) — small; T19 is an assessment that may be skipped. Check sled2redb no longer rebuilds the digest stream half |

Rules:

- Review the whole `pending_review` set at a checkpoint in **one** review
  session, passing the relevant report/task sections and the commit refs from
  `STATE.md`.
- The reviewer may fix or revert any task in the batch, set its final state, and
  record follow-ups; it does not implement follow-ups. It checks especially
  that a split actually removed a rebuild edge (§A2), that no extraction coupled
  two programs (§A5), changed a byte contract (§A6), broke a nextest filter
  (§A7/§F31), or pulled a `DO-NOT-TOUCH` item into a new crate.
- After every task is settled, run a **follow-up planning review** first to
  decide how to batch follow-ups and which to drop, then alternate implement and
  review sessions over those batches.
- If follow-up count keeps growing, the orchestrator may call an extra review
  session purely to reduce/prune the list. The orchestrator itself does not
  research the code.
