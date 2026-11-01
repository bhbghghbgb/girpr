# T26 - P12: decide whether `PathRoot::join` may cache leaf resolution

Report: `../../OPTIMIZATIONS_REPORT.md` lines 548-570 (P12).

Dependencies: T17 (single-writer guarantee). This is a **decision-gated** task:
doing nothing, with a measured reason, is a successful outcome.

## Goal

`PathRoot::join` resolves the joined path to its nearest existing ancestor and
canonicalizes it on every call, then compares with the cached base. That is the
containment check working as designed, but it is a filesystem walk per file per
phase; the same file can be resolved several times per run.

A cache assumes a directory's resolved form does not change during the run (a
symlink swap), which is a TOCTOU window the current code deliberately does not
have. The report says a cache is defensible only if the install is trusted
during the run (single-writer after C2); otherwise leave it. Do not turn this
into a lexical check.

## Decision gate

1. Measure first. Use the existing `--scan`/`--check-only` code path with a
   synthetic plan (or a real large install) and time the run with and without
   a local instrumentation counter around `resolve_existing_prefix`. Record:
   - number of `join` calls per phase;
   - wall-clock share of path resolution (aim for a rough percentage, not a
     microbenchmark).
   If path resolution is below roughly 2-3% of wall clock on the large-install
   case, **stop**: report "no change; measured share too small" and skip the
   task. That is the default outcome.
2. If it is material, implement only the conservative cache described below.
   If the orchestrator/user has not approved weakening the TOCTOU property,
   stop and record the decision instead of implementing.

## Conservative design (only if the gate passes)

- Cache **directory prefixes**, not the leaf. A leaf that exists is still
  canonicalized and compared, so a symlink at the leaf cannot be missed.
- Key: the unresolved parent path (`joined.parent()`), value: its resolved
  form. A per-root `Mutex<HashMap<PathBuf, Arc<PathBuf>>>` is acceptable;
  contention is on a short critical section and the values are immutable.
- On a cache hit, join the leaf and:
  - if the leaf does not exist, return the lexical joined path after the
    containment comparison on the resolved parent (the existing missing-tail
    behavior);
  - if the leaf exists, canonicalize the leaf itself and compare it with the
    cached resolved base. If it is a symlink that escapes, refuse exactly as
    today.
- On a cache miss, run the current `resolve_existing_prefix` and store the
  resolved parent.
- Document the assumption in `PathRoot`: "the install is single-writer for the
  duration of the run; a directory symlink swapped mid-run may not be observed
  by a cached prefix. The leaf is never cached."
- Provide a way to build an uncached `PathRoot` for tests, or keep the cache
  behind `join_cached` so the strict `join` remains available.

## Where to start

- `src/util/path.rs`: `PathRoot` (around line 210), `join`/`join_rel` (around
  lines 232-248), `resolve_existing_prefix` (around line 284), the containment
  tests (around lines 445-560).
- Callers that share a root: `src/repair/scan.rs` (`scan_plan`/`scan_one`),
  `src/repair/phases/check.rs`, `src/repair/file/mod.rs`,
  `src/repair/scope/report.rs`.

## Tests (only if implemented)

- Cache hit and miss return the same path as the uncached join for: existing
  file, missing file, missing intermediate directories, a symlink inside the
  base, a symlink/junction that escapes, and a non-UTF-8 path.
- A leaf symlink is still resolved on a cache hit (the test the design exists
  to pass).
- A changed directory symlink mid-run is the documented limitation; do not
  write a test that asserts the unsafe behavior. If the team wants strict
  behavior, do not cache.
- `cargo test -p girpr` passes.

## Acceptance

- Either:
  - the measurement is recorded and the task is reported as "no change"; or
  - a cache is added with the leaf still resolved, the assumption documented,
    and all containment tests passing.
- No lexical-only shortcut.
- `cargo test -p girpr` passes.

## Commit plan

Only if implemented:
1. `perf(path): cache resolved directory prefixes under the run root`
2. `test(path): pin cache parity and leaf containment`

If skipped, no commit; the return records the measurement and the decision.

## Notes

- This is the one task where "skip with evidence" is explicitly preferred over
  a change. The containment guarantee is security-relevant.
- Do not combine this with T30's rel-path helpers; they answer different
  questions.
