# T24 - P11: remove purge/scope per-pass allocations

Report: `../../OPTIMIZATIONS_REPORT.md` lines 523-546 (P11).

Dependencies: none. Keep this behaviour-preserving; the purge keep rules are a
deliberate choice and must not widen or narrow.

## Goal

`purge::plan_purge` rebuilds and re-keys three sets on every pass (up to twice
per run), `mark_kept_ancestors` allocates a `PathBuf` per ancestor level,
`keep_set()` allocates a fresh set per call, and `PathScope::is_visible`
rebuilds `MatchOptions` per path. Remove the avoidable per-pass/per-path work
without changing any classification.

## Where to start

- `src/repair/purge.rs`: `plan_purge` (around line 92), the three set builds
  (around lines 104-106), `mark_kept_ancestors` (around line 232), `rel_path`
  (around line 310), `keep_set` (around line 346).
- `src/repair/mod.rs`: purge planning (around line 253) and the post-phase call.
- `src/repair/phases/post.rs`: `plan_purge` call (around line 60).
- `src/repair/scope/filter.rs`: `match_options` (around line 32),
  `is_visible` (around line 79).

## Steps

1. Build the keyed keep inputs once per run, not once per purge pass. Add a
   small `PurgeKeys` (or pass a precomputed struct) containing the
   `path_key`-folded expected/server-keep/blacklist sets, and construct it in
   `plan_phase` where the plan is immutable. Use the same instance for the
   before-pass, the after-pass, and the `--print-plan` numbers.
   - `RepairPlan` is immutable after construction, so this cache cannot go
     stale within a run. State that assumption in the struct's doc.
   - If the post-phase re-plan can see different candidates, only the
     *expected* set (from the plan) is reused; the walked candidates are still
     recomputed. Do not reuse a candidate list.
2. Make `keep_set()` a `const`/`OnceLock`-backed set or a `&'static
   HashSet<String>`; it is a fixed two-element policy. Do not allocate it per
   call.
3. `PathScope::is_visible`: replace the per-call `match_options()` construction
   with a `const MATCH_OPTIONS: MatchOptions` (the fields are const-friendly)
   or a `OnceLock`. Keep the exact three flags.
4. `mark_kept_ancestors`: avoid allocating a `PathBuf` per level where the
   final set can be keyed by the same `PathBuf` the walk already owns. Use
   `path.ancestors()`/`parent()` and insert only when absent; if the set must
   own keys, keep one allocation per kept ancestor rather than one per level
   plus a clone. Do not change which directories are marked.
5. The `is_visible` pattern loop over every pattern is inherent to the `glob`
   crate. Do not replace it with a hand-rolled matcher in this task; the
   report calls that an option, not a drop-in.
6. Keep `rel_path`'s lossy `/` spelling. T30 will deduplicate it; do not change
   its output here.

## Tests

- Purge classification is unchanged for: expected file, server keep,
  blacklist, hidden by scope, unexpected file, kept directory ancestors,
  `*_tmp`/resume artifacts, and the two-element `keep_set` entries. The
  existing purge tests cover most; add a regression table if the cache changes
  call shape.
- `--purge-before` + `--purge-after` in one run uses the cached expected set
  and still deletes the same set. Compare the `PurgePlan` before/after.
- `is_visible` returns the same answers with the const options for include-only,
  exclude-only, both, and open scopes.
- Do not assert allocation counts; the gain is a code-review property.
- `cargo test -p girpr` passes.

## Acceptance

- The keyed keep sets are built once per run, not per purge pass.
- `keep_set()` and `MatchOptions` are not allocated per call/path.
- No purge classification, scope decision, or directory sweep changes.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(purge): build the keyed keep sets once per run`
2. `perf(scope): use const match options`
3. `perf(purge): stop allocating per ancestor level`
4. `test(purge): pin the cached-key classifications`

## Notes

- Caching the expected set couples the purge to the plan's lifetime. The plan
  is already immutable and shared by `Arc`; document it rather than adding a
  lock.
- Do not cache the walk. The filesystem can change between the before and
  after passes; only the plan-derived inputs are stable.
