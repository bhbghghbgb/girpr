# Review checkpoints

A review-and-fix session reviews one batch at a time. Batches are sized by **amount of
work**, so a reviewer is never overwhelmed; a batch that turns out larger than expected
gets split or its own review. These groupings are a starting proposal — the orchestrator
adjusts them based on the actual state and may call a review at any time, especially to
prune follow-ups that are running away.

| Checkpoint | Tasks | Why grouped (work size / risk) |
|---|---|---|
| A | T01, T02 | Two small root production mechanics (§R1, §R5, §R7); verify ordered log emission, the never-cancel rule and bounded concurrency survive |
| B | T03 | High-risk salvage-order heuristic (§R6) — own review to re-check the byte-level safety argument; a validated skip or revert is correct |
| C | T04 | Large root test-server rewrite (§R2, §X1) — verify attempt-count/cancellation/timeout assertions were adapted, not dropped |
| D | T05, T06 | Root CLI and process test helpers (§R3, §R4, §LEAF-2, §X3) — verify the async process seam stays and help coverage is preserved via snapshots |
| E | T07, T16, T17 | Small changes to shared leaves (§R8, §LEAF-10, §LEAF-7, §SYNC-2, §LEAF-6) — verify unrelated consumers still build and aliases/padding survived |
| F | T08, T10, T32 | App production text/wire helpers (§APP-1, §APP-3, §APP-4, §APP-9, §APP-10, §APP-12) — verify policy layer, error wording changes and optional BOM skip |
| G | T09 | Cross-product URL work (§X2, §APP-2, §APP-5, §CAP-3, §CAP-4) — large and coordination-heavy; verify one `game_ids[]` spelling everywhere and regenerated URL pins |
| H | T11, T12, T13 | App protocol/system mechanics (§APP-7, §CAP-5, §APP-8, §LEAF-4, §APP-6, §LEAF-3) — verify FNV byte order, frozen dump keys and Windows junction behavior |
| I | T14, T15 | Two large wiremock test-server rewrites (§APP-11, §LEAF-1, §X1) — verify retained thin servers where truncation/hooks are needed and journals translate correctly |
| J | T18, T19, T20 | Small leaf changes (§LEAF-9, §SYNC-4, §LEAF-8, §LEAF-5) — verify scratch cleanup, first-violation order and globset separator policy |
| K | T21, T22, T23 | Sync production utilities (§SYNC-1, §SYNC-3, §SYNC-5) — verify two-byte layout/canonical order, exact ns cache value and escape refusal |
| L | T24, T25 | Cache-json stream plus benchmark harness (§SYNC-6, §SYNC-7, §SYNC-8) — verify meta-first/duplicate rules, error wording and that only the bench declaration changed |
| M | T26 | Low-confidence walker replacement (§SYNC-9) — own review of the behavior-diff evidence; a skip/revert is correct without it |
| N | T27 | Large capture mock-server rewrite (§CAP-1, §X1) — verify journals/goldens regenerated and the malformed raw helper untouched |
| O | T28, T29, T30 | Capture mechanics (§CAP-2, §CAP-6, §CAP-7) — verify no `preserve_order`, preserved error vocabulary and LF-only rows |
| P | T31 | Capture cosmetics and optional differ (§CAP-8–§CAP-11) — verify fixture tags for the semver change and that a skipped optional item is recorded as such |

## Rules

- Review the whole `pending_review` set at a checkpoint in **one** review session,
  passing the relevant report/task sections and the commit refs from `STATE.md`.
- The reviewer may fix or revert any task in the batch, set its final state, and record
  follow-ups; it does not implement follow-ups. It checks especially that no
  non-finding or pinned contract changed (report lines 795–855, `DO-NOT-TOUCH.md`),
  assumptions were validated against current code (§A1–A2, lines 15–18), tests of
  intended function were adapted not deleted (§A5, lines 26–28), no `preserve_order`
  was enabled (§A7, lines 33–34), deterministic/pinned outputs survived, and no
  committed artifact references this package or the report.
- A validated `skipped` outcome for a stale, unsafe, low-confidence or low-payoff
  finding is correct and is not a failed review; do not pressure such a task into a
  change.
- After every task is settled, run a **follow-up planning review** first to decide how
  to batch follow-ups and which to drop, then alternate implement and review sessions
  over those batches.
- If follow-up count keeps growing, the orchestrator may call an extra review session
  purely to reduce/prune the list. The orchestrator itself does not research the code.
