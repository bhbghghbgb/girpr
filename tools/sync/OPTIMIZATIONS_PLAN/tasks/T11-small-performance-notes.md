---
id: T11
title: Small performance notes
batch: D
checkpoint: D
report: [L390-L409]
design_notes: "The diff and the planner must agree about the short circuit; do not change the rw gates for performance"
depends_on: [T06]
size: S
risk: low
---

# T11 — Small performance notes

## Goal

Take the three small, low-risk wins in report §2.7 and explicitly leave the
rest:

1. remove the redundant sort between `HashPlan::pending()` and
   `resolve_folder`'s own pending list;
2. maintain the file count in `walk_live` while pushing entries instead of
   counting in a second pass;
3. bound the per-copy `Result` collection in `copy_files` while still reporting
   the **lowest-index** failure in plan order.

Leave `write_json`'s per-record field sort, `Record::fields_json`, the three
`RwRuntime` gates, and `Gate::release`'s `notify_all` alone. Those are
documented tradeoffs, not oversights.

## Why

The first two are pure local simplifications. The third matters because
`copy_files` currently collects one `Result<()>` per copied file only to find
the lowest plan-order failure; on a large tree the vector is tree-sized for a
value that is one error or none.

## Files to read

- `src/planner/plan.rs` — `HashPlan::pending`, `digest_count`
- `src/effective/phase_c.rs` — `resolve_folder`'s pending construction and its
  log line
- `src/scan.rs` — `walk_live`'s final count
- `src/commands/sync/apply/phases.rs` — `copy_files`'s `collect` and result loop
- `src/report/record.rs` — `write_json`, `fields_json` (read only; do not edit)
- `src/rw/gate.rs` — `release` (read only; do not edit)

## Constraints

- The failure reported by `copy_files` must remain the lowest-index failure in
  **plan order**, not the first worker to finish. That is what the current
  `Vec` + zip loop guarantees.
- `resolve_folder` must still hand the pool a deterministic, sorted starting
  order.
- A plan entry is always a file present in `phase_a.map`; do not weaken that
  invariant silently. If you rely on it, add a `debug_assert!` or a comment.
- `walk_live`'s `files`/`dirs` counters must match the current values exactly,
  including when the walk aborts.
- Do not add a dependency (no `criterion`, no smallvec) for any of this.

## Implementation

1. **Pending sort.** In `resolve_folder`, build `pending` from
   `plan.pending()` directly if the plan/map invariant holds:
   ```rust
   let pending = plan.pending();
   info!(pending = pending.len(), digests = plan.digest_count(), "resolve start");
   ```
   `pending()` is already sorted and already filters empty algorithm sets.
   Confirm `plan_one` only inserts file entries and that every key comes from
   `phase_a.map`. If a debug assertion is needed, assert
   `phase_a.map.contains_key(rel)` for each pending rel. Do not keep both
   sorts.
2. **Walk count.** Replace
   `let files = out.iter().filter(|e| !e.is_dir).count();` with a `files`
   counter incremented in the file branch, then `dirs = out.len() - files`.
   Keep the same `info!` fields and their values.
3. **Bounded copy errors.** Change the pool closure to `enumerate()` the plan
   and keep only the best failure:
   ```rust
   let failure: Mutex<Option<(usize, anyhow::Error)>> = Mutex::new(None);
   ```
   On `Err(e)`, lock and replace only when the new index is lower. After the
   pool, if `Some((idx, e))`, return
   `Err(e).with_context(|| format!("copy {}", plan.copy[idx].rel))`.
   Keep emitting `COPY` records and writing cache rows for successful copies
   exactly as today; do not abort the pool early, because the current run
   attempts every copy and reports the lowest-index failure afterwards.
   Keep `copied` for the success count; on a failure the count is not used.
4. Do **not** change `write_json`, `fields_json`, `RwRuntime::new`, or
   `Gate::release`. Add a one-line comment at each site only if it prevents a
   future “cleanup”; otherwise leave the code alone.
5. Tests:
   - `HashPlan::pending`'s existing test stays;
   - add a test that a plan whose entries are a subset of the map hashes exactly
     those paths, in sorted order (or keep the existing deterministic-order
     test);
   - add a copy failure test using the existing `GIRSYNC_TEST_FAIL_COPY` hook
     where two copies fail; assert the reported path is the plan-order first
     (lowest index), not the completion-order first.
   - walk counts are covered by `scan.rs`'s existing tests; assert `files` and
     `dirs` in the log line only if there is already a log-capture helper.

## Commit plan

1. `perf(sync): use the planner's sorted pending list`
2. `perf(sync): count walk files while pushing`
3. `perf(sync): keep only the lowest-index copy failure`
4. `test(sync): pin the lowest-index copy error`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync planner::
cargo test -p girsync scan::
cargo test -p girsync apply::
cargo test -p girsync
```

## Acceptance

- `resolve_folder` sorts the pending list once.
- `walk_live` reports the same file/dir counts without a second pass.
- `copy_files` stores at most one failure and reports the lowest plan index.
- No behaviour change in the copies that succeed.
- `write_json` and the rw gates are untouched.

## Reviewer focus

- Does `plan.pending()` really contain exactly the paths `resolve_folder`
  would have built?
- Is the failure comparison by plan index, not insertion time?
- Could replacing the failure drop an error with a better context?
- Are all copies still attempted, so the cache/COPY side effects match?
- Did the walk counter stay correct on an empty tree and a one-entry tree?
