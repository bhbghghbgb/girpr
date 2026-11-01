# T17 - C2 + C9: one lock for every mutating run, and fast recovery

Report: `../../OPTIMIZATIONS_REPORT.md` lines 95-119 (C2) and 246-264 (C9).

Dependencies: none. This is deliberately late in the ordering because it changes
a concurrency contract and the tests that pin it.

## Goal

Two problems in the current in-install lock file:

- **C2:** a scoped run skips the lock when `--include/--exclude` hides
  `girpr.lock`, so two scoped runs (or a scoped and an unscoped run) can write
  the same admitted files concurrently.
- **C9:** a lock left by a killed run blocks every later run for up to 24 hours,
  even though its body names a pid that is gone.

Redesign the lock so every mutating run takes it, independently of the path
scope, and so a dead owner is recovered promptly. Keep read-only modes
(`--check-only`, `--dry-run`, `--print-plan`) lock-free.

## Preferred design

Use an **OS-level advisory lock** on a file outside the install, keyed by the
canonical game directory:

1. Compute a stable key from the canonical `game_dir` (a small FNV-1a over the
   path bytes; on Windows fold ASCII case, because the filesystem does). Do not
   use `DefaultHasher`'s unspecified algorithm.
2. Lock file: `std::env::temp_dir().join(format!("girpr-{key:016x}.lock"))`.
   Create it with read/write, then take an exclusive OS lock on the open handle
   (`std::fs::File::try_lock` on the toolchain's stable file-locking API).
   Keep the `File` inside `Lock`; the OS releases it on process death.
3. On refusal, read the body (`pid=`, `started=`) and name the holder. Because
   an exclusive lock is held by a live process, there is no mtime staleness
   rule and no 24-hour wall. If the body is unreadable, say so; the lock still
   excludes.
4. On `Drop`, unlock and remove the file best-effort. A leftover unlocked file
   is harmless: the next `try_lock` succeeds.

If the pinned toolchain does not expose stable file locking, use the documented
fallback instead of inventing a Windows-only dependency:

- create the temp lock with `create_new(true)`;
- on `AlreadyExists`, read `pid=` and probe liveness with the cheapest
  best-effort mechanism the platform allows (for example `kill(pid, 0)` on
  Unix); if the pid is gone, take over once;
- if liveness cannot be probed, keep a **short** documented stale window (one
  hour, not 24) and log the remaining time in the refusal;
- never retry an undeletable stale lock in a loop.

Record which design was used in the module docs and the README.

## Where to start

- `src/repair/lock.rs`: the whole module; `LOCK_FILE`, `STALE_AFTER`,
  `Lock::acquire`, `acquire_with`, `is_stale`, `holder_pid`, tests.
- `src/repair/mod.rs`: the lock gate in `execute_phase` (around line 492).
- `README.md`: the lock paragraph; remove the "a path outside the scope does
  not exist" justification for skipping the lock.
- Tests that assert a scoped run takes no lock (search `LOCK_FILE`,
  `scope.is_visible`, and the scoped-run fixtures in `tests/`).

## Steps

1. Replace the lock path with the temp-dir/key design. Keep `Lock::path()` for
   diagnostics, but it now names the temp file.
2. Take the lock whenever the run is mutating:
   `!ctx.readonly() && !ctx.check_only && !ctx.print_plan`. Remove the
   `scope.is_visible(lock::LOCK_FILE)` condition entirely. `--purge-only` and
   `--dry-run` are covered by the existing `readonly`/`check_only` rules; make
   the predicate explicit and test it.
3. Keep the in-install `girpr.lock` only as a **migration** concern: do not
   create it. If an old file exists, leave it alone; the purge/scope rules
   already treat it as an ordinary unexpected file, and a new run must not
   delete it behind the user's back.
4. Make the refusal actionable: include the lock path, the holder pid when
   known, and (fallback design only) how long until takeover. Keep the error
   class a usage failure (exit `1`), as today.
5. Add an injection seam for the lock path or the lock operation so the tests
   can build two `Lock`s for the same key without racing the global temp
   directory. Do not expose it as production API.
6. Update the lock tests:
   - two acquisitions for the same game dir: the second is refused;
   - two different game dirs do not conflict;
   - dropping releases it;
   - a scoped run (`PathScope` with an include that excludes `girpr.lock`)
     still takes the lock;
   - the fallback's dead-pid takeover, if the fallback is used.
7. Update the README: the lock lives outside the install, every mutating run
   takes it, and a crashed run is recovered without a wall-clock wait.

## Tests

- `cargo test -p girpr`, especially the lock module and the scoped-run
  integration tests.
- A lock test must not depend on a real 24-hour mtime; use the injected lock
  path/operation or an explicit old timestamp.
- If the advisory-lock API is not available, the fallback tests must still be
  deterministic (inject the liveness probe).

## Acceptance

- Every run that mutates the install takes the lock, regardless of scope.
- Read-only modes still take no lock and write no lock file.
- A dead owner no longer blocks for 24 hours; the chosen recovery is documented.
- No new dependency is added for a Windows-only named object.
- The README and lock module agree.
- `cargo test -p girpr` passes.

## Commit plan

1. `fix(lock): key the single-writer lock outside the install`
2. `fix(lock): take the lock for scoped runs and recover a dead owner`
3. `test(lock): pin scope-independent locking and recovery`
4. `docs: describe the new lock location and recovery`

## Notes

- This is a concurrency contract, not a micro-optimization. Do not combine it
  with A1/A2 code changes in the same commit; those are separate tasks.
- Do not put the lock back inside `game_dir`: that is the C2 contradiction.
- Do not rely on `Drop` alone for release if the OS handle can do it; a killed
  process runs no destructors.
