# T01 — Right-size the Tokio runtime and move repair file I/O off async workers

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §2 **S-01** (lines 109–125) — `--jobs 1024` builds
  1025 Tokio OS threads because `worker_threads(available, jobs) =
  available.max(jobs.saturating_add(1))`; size the runtime independently of the file
  window and keep `jobs` as the `buffer_unordered` bound.
- §2 **S-02** (lines 127–141) — repair file tasks use `tokio::spawn` and do long
  synchronous `std::fs` work on runtime workers; offload the sync portions to
  `spawn_blocking` (or `tokio::fs`), keeping only `stream_chunk_to` async.
- §11 **assumption 1** (lines 837–840) — S-01/S-02 are a pair: move repair work off
  async workers before shrinking the runtime; otherwise `jobs` tasks can starve
  timers/network. Validate with `--jobs 1/4/64` on SSD and HDD where possible.
- §5 **NET-07** (lines 404–413) is the same class in the fetch stream and is handled
  by T10; keep the two consistent (see `guidance/DO-NOT-TOUCH.md`).
- Keep items: §12 (lines 890–905), data-plane contract (lines 527–530),
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/main.rs` (runtime construction ~91–98; worker-thread test ~190–195).
- `src/cli.rs` (`MAX_JOBS`, `worker_threads` ~26–38).
- `src/repair/file/mod.rs` (`tokio::spawn(spawn(idx))` ~418–420, `repair_one_file`
  ~477–534), `inplace.rs` (~237, 562, 606), `temp.rs` (~96–184).
- Locate the current call sites before editing; the list above is a hint.

## Implementation plan
1. Confirm S-01/S-02 in the current tree and confirm assumption 1 holds. If repair
   work cannot be moved off workers, do not shrink the runtime — stop.
2. Size the runtime independently of `jobs` (e.g. `available.max(2) + 1`, or
   `available.max(2)`); keep `jobs` solely as the concurrency bound.
3. Offload the synchronous scan/stage/verify/copy portions of each file task to
   `spawn_blocking` or `tokio::fs`, keeping only `stream_chunk_to` async.
4. Update the `worker_threads(1, jobs) > jobs` test to match the new policy.
5. Deliver in several commits if useful: runtime sizing, offload, test update.

## Constraints / validation
- No async future inside a repair task may need a worker while all file tasks are
  blocked; verify `reqwest` futures are `Send` and awaitable outside the blocking task.
- Do not shrink the runtime first (assumption 1); the two changes land together.
- Keep the `REPORT`/`PLAN`/`PROGRESS`/`SUMMARY` output unchanged (lines 527–530).
- Do not change tuned profiles or any §12 non-finding.

## Done when
The runtime size is independent of `--jobs`, synchronous repair file work runs off
async workers, and tests pass with `--jobs 1/4/64`. If the pairing assumption fails
or the change would starve timers/network, **stop** and record `skipped` with a
one-line reason.
