# Review checkpoints

A review-and-fix session reviews one batch at a time. Batches are sized by
amount of work so a reviewer is never overwhelmed; a batch that turns out larger
than expected gets split or its own review. These groupings are a starting
proposal — the orchestrator adjusts them based on the actual state and may call
a review at any time, especially to prune follow-ups that are running away.

| Checkpoint | Tasks | Why grouped |
|---|---|---|
| A | T01–T06 | Small, low-risk, mostly independent quick wins |
| B | T07–T12 | Shared infrastructure across packages |
| C | T13–T15 | The three JSON renderers (§A1) — review together for cross-tool consistency |
| D | T16–T17 | Retry/backoff pair (§A2) — compare the two semantics |
| E | T18–T22 | Path canonicalization, glob, atomic publish, status/URL |
| F | T23–T24 | HTTP mock servers (§A5) |
| G | T25–T26 | Medium girsync changes |
| solo | T27 | Cache wire codec (§E1) — format-breaking, own review |
| solo | T28 | `cache/open.rs` publish protocol (§E5) — largest and riskiest, own review |
| H | T29–T30 | INI changes (§A4) — both depend on the same open assumption |
| I | T31–T35 | Misc/low-priority tail — split further if any grows |

Rules:

- Review the whole `pending_review` set at a checkpoint in **one** review
  session, passing the relevant report/task sections and the commit refs from
  `STATE.md`.
- The reviewer may fix or revert any task in the batch, set its final state, and
  record follow-ups; it does not implement follow-ups.
- After every task is settled, run a **follow-up planning review** first to
  decide how to batch follow-ups and which to drop, then alternate implement and
  review sessions over those batches.
- If follow-up count keeps growing, the orchestrator may call an extra review
  session purely to reduce/prune the list. The orchestrator itself does not
  research the code.
