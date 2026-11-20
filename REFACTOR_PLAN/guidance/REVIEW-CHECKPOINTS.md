# Review checkpoints

A review-and-fix session reviews one batch at a time. Batches are sized by
amount of work so a reviewer is never overwhelmed; a batch that turns out larger
than expected gets split or its own review. These groupings are a starting
proposal — the orchestrator adjusts them based on the actual state and may call
a review at any time, especially to prune follow-ups that are running away.

| Checkpoint | Tasks | Why grouped |
|---|---|---|
| A | T01, T02 | Digest/identity primitives (§F2, F4, F5, F27) — small, foundational, review the type boundary together |
| B | T03, T04, T05 | Path and fs-name predicates (§F6–F8) — one related leaf family; check §A10 behavior is unchanged |
| C | T06, T07 | Filesystem identity + publish (§F9, F10) — medium risk, check lint/`unsafe` handling |
| D | T08, T09, T10 | Hashing sink, bounded zstd decode, retry leaf (§F3, F28, F15) — check ordering policies stay local (§A6) |
| E | T11, T12, T13, T14 | Logging/CLI surface (§F18–F23) — check no cross-program coupling and §A2 doc tables updated |
| solo | T15 | Report core (§F21) — byte-contract critical (§A3, §A8), own review |
| F | T16, T17 | Domain-model leaves (§F24, F25) — review the leaf boundary and load-bearing keys together |
| solo | T18 | API DTO leaf (§F26) — largest model task, both decoding strategies must survive, own review |
| G | T19, T20 | Test infrastructure (§F16, F17) — A5/A11 constraints, N7 fixtures excluded |

Rules:

- Review the whole `pending_review` set at a checkpoint in **one** review
  session, passing the relevant report/task sections and the commit refs from
  `STATE.md`.
- The reviewer may fix or revert any task in the batch, set its final state, and
  record follow-ups; it does not implement follow-ups. It checks especially
  that no extraction coupled two programs, changed a byte contract, or pulled a
  `DO-NOT-TOUCH` item into a leaf crate.
- After every task is settled, run a **follow-up planning review** first to
  decide how to batch follow-ups and which to drop, then alternate implement and
  review sessions over those batches.
- If follow-up count keeps growing, the orchestrator may call an extra review
  session purely to reduce/prune the list. The orchestrator itself does not
  research the code.
