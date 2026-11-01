# T15 - P10: share the local chunk map instead of cloning it

Report: `../../OPTIMIZATIONS_REPORT.md` lines 502-521 (P10).

Dependencies: none.

## Goal

`repair::plan_phase` builds `Arc::new(local_map.clone())` for `scan::scan_plan`
while keeping the owned map for later phases. The map has one entry (plus
index vectors) per reusable chunk; the clone duplicates every `ReuseEntry` and
the `by_key` indices just before the plan is asserted. Both consumers only
read.

Make `local_map` an `Arc<LocalChunkMap>` from its producer through `FileCtx`
and the scan. This is a mechanical signature change, but it touches the file
context and tests.

## Where to start

- `src/repair/mod.rs`: `PlanPhase.local_map` (around line 120); the
  `local_reuse_map` call (around line 196); the `Arc::new(local_map.clone())`
  scan argument (around line 319); estimate calls (around lines 356-358); the
  destructure and `repair_all` call (around lines 443, 466, 579).
- `src/repair/file/mod.rs`: `repair_all` signature (around line 194),
  `FileCtx` (around line 383), test constructors (around line 853).
- `src/repair/scan.rs`: `scan_plan` already takes `Arc<LocalChunkMap>`;
  `scan_one` and `est_download_bytes_by_file` take `&LocalChunkMap`.
- `src/repair/file/inplace.rs` and `temp.rs`: `ctx.local_map.get(...)` works
  through `Arc` deref; no change needed if the field type changes.

## Steps

1. In `plan_phase`, wrap the map once:
   `let local_map = Arc::new(manifest::local_reuse_map(...).await);`.
   Change `PlanPhase.local_map` to `Arc<LocalChunkMap>`.
2. Pass `Arc::clone(&local_map)` to `scan_plan`; remove the `.clone()` of the
   map. Keep the map for the later phases.
3. Change `execute_phase` to move the `Arc` into `file::repair_all` and into
   `FileCtx.local_map`. `repair_all` takes `Arc<LocalChunkMap>`.
4. Deref where a `&LocalChunkMap` is expected:
   `plan.est_download_bytes_by_file(&local_map)`,
   `s.est_download_bytes_by_file(..., &local_map)`.
5. Update test constructors of `FileCtx`/`repair_all` to wrap their maps in
   `Arc`. Do not change the maps' contents.
6. Do not change `LocalChunkMap`'s type or the reuse lookup. This task is
   ownership only.

## Tests

- The full `cargo test -p girpr` suite, especially `repair_offline` and
  `repair_machinery`, must pass.
- If a test can observe the map's contents, assert the same reuse decisions as
  before; do not assert pointer identity.
- No new timing test.

## Acceptance

- No clone of the local chunk map remains on the `--scan` path.
- `local_map` is shared by `Arc` from production through the scan and repair.
- Reuse decisions and estimates are unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(repair): share the local chunk map by Arc`
2. `test(repair): update the file-context constructors`

## Notes

- Do not introduce a new lock or `RwLock`; the map is immutable after it is
  built.
- T27/D1 may later move more per-file facts into the plan. Keep this change
  separate so a revert is easy.
