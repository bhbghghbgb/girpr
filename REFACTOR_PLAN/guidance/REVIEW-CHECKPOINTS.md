# Review checkpoints

A review-and-fix session reviews one batch at a time. Batches are sized by **amount of
work**, so a reviewer is never overwhelmed; a batch that turns out larger than expected
gets split or its own review. These groupings are a starting proposal — the orchestrator
adjusts them based on the actual state and may call a review at any time, especially to
prune follow-ups that are running away.

| Checkpoint | Tasks | Why grouped (work size / risk) |
|---|---|---|
| A | T01 | High-risk pairing (§2 S-01/S-02, lines 109–141): runtime sizing plus blocking-I/O offload across `main.rs`, `cli.rs` and the repair file tasks; verify the runtime is no longer tied to `jobs`, no file work remains on async workers, and the worker-thread test is updated |
| B | T02, T03 | Two parallelism changes (§2 S-03/S-04, 143–172): bounded intra-file fetch and concurrent plan-phase fetches; verify disjoint-range and ceiling-ledger assumptions (2) and that the three phases share one client/budget without changing plan output |
| C | T04, T05 | Purge parallelization plus duplicate stat removal (§2 S-05/S-06, 173–198): verify deterministic purge log order and that the plan/scan stat overlap is actually removed, not just reshuffled |
| D | T06 | Multi-file reuse I/O rewrite (§3 IO-01..IO-04, 202–251; assumptions 3–4): verify the shared scan verdict, single-pass reuse reads and single `layout()` computation, and that the RAM/write budgets and required TOCTOU guard survive |
| E | T07, T08 | Path/canonicalization and post-phase I/O (§3 IO-05..IO-08, 252–293): verify `join_validated` is only used for plan-derived rels (assumption 5), canonicalization is cached, and audio-cache mkdir/stream behavior is unchanged |
| F | T09, T10, T11 | Network + fetch correctness (§4 MEM-01, §5 NET-01..07, §8 MIC-04/05/12; assumptions 6–8): hardening of decode bounds and blob IDs, blocking offload, timeouts/status/retry classification and fetch micro-opts; verify the chosen caps/id checks match production (8) and recorded JSON is unchanged |
| G | T12, T13 | Digest and hash internals (§6 COR-03, §8 MIC-01/08/11, §4 MEM-03/04, §8 MIC-09/10): verify reuse offsets are validated, digests parsed once, scratch buffers reused safely and digests byte-identical |
| H | T14, T15, T16 | Correctness / fail-fast cluster (§6 COR-01/02/05/08/06/07/09; assumptions 9–10): verify the exit-class change is acceptable, stale-verdict handling is correct, tmp TOCTOU is closed, and the one deliberate `SUMMARY` extension keeps its shape |
| I | T17, T18 | Logging and micro sweep (§7 LOG-01..03, §8 MIC-02/03/06/07; assumptions 11–12): verify the data plane is unchanged, expensive fields are gated, the lossy-writer trade-off is deliberate, and report visibility/sort behavior is preserved |
| J | T19, T20 | gircap HTTP allocation + decode/memoization work (§9 TOOL-01/03/04/07/08/09/10/11/12; assumptions 14–15): verify recorded JSON stays byte-identical, memory is bounded and measured, and artifact bytes are deterministic |
| K | T21, T22, T23 | gircap correctness, parallelization and error reporting (§9 TOOL-05/06/13/14/15/17/19/20/21; assumptions 16–17): verify symlink dedup contract, deterministic write order, thread-safe interior mutability, and honest declared-size errors |
| L | T24, T25 | Dev-tool I/O and build surface (§9 TOOL-02/16/18, §10 B-01; assumption 13): verify girsync's `postcard` digest vocabulary and atomic redb publish survive, and that the `chrono` removal is validated with `cargo tree -i chrono` |

## Rules

- Review the whole `pending_review` set at a checkpoint in **one** review session,
  passing the relevant report/task sections and the commit refs from `STATE.md`.
- The reviewer may fix or revert any task in the batch, set its final state, and record
  follow-ups; it does not implement follow-ups. It checks especially that no
  non-finding or pinned contract was changed (§12, lines 890–905; data plane 527–530),
  that assumptions hold, that pairings are consistent, and that no committed artifact
  references this package or the report.
- A validated `skipped` outcome for a stale or unsafe change is correct and is not a
  failed review; do not pressure such a task into a change.
- After every task is settled, run a **follow-up planning review** first to decide how
  to batch follow-ups and which to drop, then alternate implement and review sessions
  over those batches.
- If follow-up count keeps growing, the orchestrator may call an extra review session
  purely to reduce/prune the list. The orchestrator itself does not research the code.
