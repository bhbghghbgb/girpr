# T04 — Parallelize purge walk/stat/delete while keeping deterministic log order

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §2 **S-05** (lines 173–185) — purge's walk,
  per-candidate `metadata()` and delete loop all run single-threaded inside
  `spawn_blocking_traced`; a purge of tens of thousands of orphans is dominated by
  serialized `metadata()` and `remove_file()` syscalls.
- Change suggested in the report: parallelize per-candidate stat and delete with
  bounded scoped threads (or `rayon`, already in the tree), collecting
  `(candidate, Result<...>)` into index slots so log lines emit in plan order.
- **Assumptions** at report lines 183–184 — `remove_file` on distinct paths is safe
  concurrently; the empty-directory pass must run after all deletes.
- Keep items: deterministic purge log order, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/purge.rs` (single `WalkDir` ~82, classify + `e.metadata()` ~137–161,
  delete loop ~254–291, empty-dir removal ~299–313).
- `spawn_blocking_traced` usage and any rayon/scoped-thread dependency in scope.
- Locate current structures before editing; the list above is a hint.

## Implementation plan
1. Confirm the purge hot path is single-threaded in the current tree and that
   candidates are independent paths. If the walk itself must stay serial, parallelize
   only the stat and delete stages.
2. Use bounded scoped threads or `rayon` to stat and delete candidates concurrently;
   write results into preallocated index slots.
3. Emit log lines in plan order after the parallel work to keep deterministic output.
4. Run the empty-directory pass strictly after all deletes.
5. Deliver in several commits if useful: stat stage, delete stage, ordered logging.

## Constraints / validation
- Log order must remain deterministic and in plan order (lines 179–183).
- Distinct-path concurrent deletes only; empty-directory pass after all deletes.
- Preserve error counting and purge-plan totals (do not double-count or lose errors).
- Do not change data-plane output or any §12 non-finding.

## Done when
Purge stat/delete are bounded-parallel, log lines stay in plan order, and purge tests
pass. If concurrent deletion or ordered emission cannot be guaranteed, **stop** and
record `skipped` with a one-line reason.
