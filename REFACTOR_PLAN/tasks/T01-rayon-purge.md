# T01 — Purge parallel map → rayon

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §R1 (lines 140–156) — `MAX_PURGE_THREADS`
  and `parallel_map` in the purge stage are a hand-rolled bounded parallel map;
  `rayon`'s `par_iter().map(f).collect::<Vec<_>>()` is that helper.
- Validate assumptions first: §A1 (lines 15–16), §A2 (17–18), §A4 (22–23).
  Keep items: `guidance/DO-NOT-TOUCH.md` (in particular the `src/repair/file/**`
  boundary at report lines 824–826 — this task stops at `purge.rs`).

## Scope
- `src/repair/purge.rs` — `MAX_PURGE_THREADS` and `parallel_map` (`:28–96`) and the
  two call sites (`:245` stat, `:376` delete).
- The root `Cargo.toml` dependency list; `parallel_delete_emits_purge_lines_in_plan_order`
  (`:910–940`) as the acceptance test. Locate current call sites before editing.

## Implementation plan
1. Confirm the helper's only contract is "overlap independent `metadata()` /
   `remove_file()` calls, collect results in input order, emit logs on the calling
   thread". If a caller relies on slot mechanics or worker count, stop.
2. Delete both helpers and import `rayon::prelude::*`; express each call site as
   `par_iter().map(f).collect::<Vec<_>>()`, preserving the surrounding log emission.
3. If the eight-thread ceiling is actually required, use
   `ThreadPoolBuilder::new().num_threads(8)` with `pool.install(...)`; otherwise let
   rayon size the pool. Add `rayon` as a direct dependency (already in `Cargo.lock`).
4. Run the purge tests; keep the ordered-emission assertion.

## Constraints / validation
- `T: Sync, R: Send` must hold; work stealing must not change the ordering that matters
  to the log sweep (§R1 lines 154–155). No test pins worker count or slot mechanics.
- Do not touch `src/repair/file/**` or any other correctness-critical byte movement.
- Re-check the version/feature set rather than quoting the report's figure.

## Done when
Both helpers are gone, the two call sites use rayon with results collected in input
order, the workspace builds and the purge tests pass. If the premise is stale or a
keeper would change, **stop** and record `skipped` with a one-line reason.
