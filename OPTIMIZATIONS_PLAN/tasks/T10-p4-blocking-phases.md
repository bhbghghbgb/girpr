# T10 - P4: move blocking phase work off workers and parallelize check

Report: `../../OPTIMIZATIONS_REPORT.md` lines 358-384 (P4).

Dependencies: none.

## Goal

Four synchronous `std::fs` loops run directly on async workers:
`purge::plan_purge` (walk + stat), `post::move_audio_cache` (walk +
rename/copy), `check::verify_plan` (read + hash), and
`scope::report::DiskPlan::new` (stat loop). While they run, timers, network
progress, and other file tasks are stalled. Worse, `verify_plan` is a plain
`for` loop, so `--check-only` ignores `--io-threads`.

Move the phase-level loops to the blocking pool and make `--check-only` verify
with the same bounded, `spawn_blocking`-based shape the scan already uses.
Keep results in plan order even when tasks complete out of order.

## Where to start

- `src/repair/mod.rs`: `plan_phase` purge planning (around line 253) and
  `DiskPlan::new` (around line 400); `execute_phase` check-only call (around
  line 531).
- `src/repair/phases/post.rs`: `run` calls `purge::plan_purge` (around line
  60) and `move_audio_cache` (around line 58).
- `src/repair/phases/check.rs`: `verify_plan` and `check_one`.
- `src/repair/scan.rs`: `scan_plan` is the existing bounded blocking shape to
  reuse; `futures::stream::iter(...).map(...).buffer_unordered(jobs)`.
- `src/repair/scope/filter.rs`: `PathScope` needs `Clone` for the post call.
- `src/repair/scope/report.rs`: `DiskPlan::new` is sync and has no result.

## Steps

### Phase wrappers

1. In `plan_phase`, run `purge::plan_purge` with `tokio::task::spawn_blocking`.
   Move owned inputs into the closure: `game_dir.to_path_buf()`, the `Arc`
   plan, the keep/blacklist sets, and a cloned `PathScope`. Await the join
   result; a panic is a bug, so map `JoinError` to a write/metadata failure
   rather than silently returning an empty plan.
2. In `plan_phase`, run `DiskPlan::new` with `spawn_blocking` using the `Arc`
   plan, a cloned `PathRoot`, `game_dir`, `ctx.repair_mode`, and `ctx.jobs`.
   Handle the `JoinError` the same way.
3. In `post::run`, change the `plan` parameter to `Arc<RepairPlan>` (callers
   already hold an `Arc`), and run its `plan_purge` call on the blocking pool.
   Do the same for `move_audio_cache`: clone the two configured dir strings,
   the `game_dir`, the scope, and the `dry_run` flag into a `spawn_blocking`
   closure. Keep `move_audio_cache` itself synchronous so its unit tests stay
   direct.
4. Derive/implement `Clone` for `PathScope` if it is not already cloneable.
   The patterns are immutable, so cloning the compiled globs is cheap and
   safe.

### Parallel check

5. Change `check::verify_plan` to an `async fn` that accepts:
   - the game dir and an `Arc<PathRoot>`/`Option` form;
   - `Arc<RepairPlan>`;
   - the `Arc<ScanReport>` when `--scan` supplied one;
   - `jobs: usize`;
   - the `Summary` (for the final fold) and `OutputFormat` (for the existing
     progress emission, until T21 centralizes it).
   `PlanPhase::scan` may need to become `Option<Arc<ScanReport>>` so the check
   and the repair loop can share it without a deep clone.
6. For each file index, spawn a blocking task that resolves the path and calls
   `check_one` with either the scan verdict or a fresh read. Bound the stream
   with `jobs.max(1)`. Collect `(index, CheckOutcome)` pairs.
7. Sort/collect the outcomes in plan order. Then run the existing logging,
   `clean`/`bad`/`damaged_chunks` counting, and `SUMMARY` counter updates in
   that order. This keeps failure log order deterministic.
8. Emit progress after the ordered fold when the interval has elapsed, exactly
   as the current loop does. Do not emit from the blocking tasks. T21 will
   replace this with a single ticker.
9. Update the call site in `execute_phase` to `.await` the new function and to
   pass `ctx.jobs` and the `Arc` values. `--check-only` still returns `0`/`4`
   and writes nothing.

## Tests

- `--check-only` with `jobs = 1` and `jobs = 4` must produce the same
  `files_skipped`, `files_failed`, damaged-chunk count, exit code, and warning
  set. Add this as a test if the existing suite does not already run both.
- A scanned check (`--check-only --scan`) must still avoid reading file data
  and still report a changed file as damaged (presence/length checks remain).
- Keep the existing check tests (absent file, scanned mismatch, unreadable
  path). They call `check_one`; if its signature changes, update only the
  call, not the assertion.
- Purge/post/disk-plan tests must pass; a `spawn_blocking` wrapper must not
  change their decisions.

## Acceptance

- No phase-level walk/stat/hash loop runs directly on an async worker.
- `--check-only` uses `--jobs` concurrent file checks, bounded.
- Results and logs remain in plan order.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(purge): plan and move audio on the blocking pool`
2. `perf(plan): build the disk plan on the blocking pool`
3. `perf(check): verify files with bounded blocking tasks`
4. `test(check): pin parallel check parity and ordering`

## Notes

- `spawn_blocking` moves work off the runtime; it does not make it free. Keep
  the bound at `jobs` and do not nest another unbounded pool inside a task.
- Do not parallelize chunks within a file; that is the deliberate P14 choice.
