# T18 — Low-risk micro-optimization sweep

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §8 **MIC-02** (lines 574–582) — glob visibility is
  re-matched per file; carry the parent directory's visibility down the `WalkDir`
  traversal and add a fast suffix check for the common `--include "*.ext"` shape.
- §8 **MIC-03** (lines 583–589) — deep directory comparators recompute
  `components().count()` per comparison; map to `(depth, PathBuf)` once, use
  `sort_unstable_by_key`, strip depth.
- §8 **MIC-06** (lines 600–604) — the retry loop's `Arc<Mutex<Option<S>>>` is
  uncontended; keep a local `Option<S>` and pass state by value.
- §8 **MIC-07** (lines 605–609) — `write_line` issues two `write_all` calls per
  record; combine line and newline into one preallocated buffer.
- §4 **MEM-02** (lines 309–319) — `classify_purge_path_visible` allocates a `String`
  per walked path to probe three `HashSet<String>`s, and `PurgeKeys::new` materializes
  three full keyed copies. `HashSet<String>` supports borrowed lookup via
  `Borrow<str>`: on non-Windows use `set.contains(rel)` with no allocation; only fold
  on Windows. A cheap membership hash is optional. This shares the purge/path
  hot paths with MIC-02/03, so handle it in this sweep.
- Keep items: visibility semantics, report output, §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/repair-plan/src/path_scope.rs` (`is_visible` ~84–89), `src/repair/purge.rs`
  (~120, 136, sort ~189–194), `app/repair-plan/src/plan.rs` (sort ~500).
- `app/fetch/src/retry.rs` (state slot ~159–170), `crates/report-core/src/lib.rs`
  (`write_line` ~105–108).
- `src/repair/purge.rs` (~426, 438–443), `app/repair-plan/src/purge_values.rs`
  (`PurgeKeys::new` ~84–86), `app/util-path/src/path.rs` (~226–232).
- Locate current code before editing; the list above is a hint.

## Implementation plan
1. Confirm each finding in the current tree; skip any sub-change already done or
   where the premise does not hold.
2. Propagate parent visibility through the walk and add the suffix fast path,
   preserving visibility decisions exactly (MIC-02).
3. Precompute `(depth, PathBuf)` and use unstable sorts where safe; keep the same
   ordering (MIC-03).
4. Replace the uncontended `Arc<Mutex<Option<S>>>` with a local `Option<S>`
   threaded by value (MIC-06).
5. Combine `write_line`'s writes into one buffer without changing the emitted bytes
   or record atomicity (MIC-07).
6. Replace allocating purge-key probes with borrowed `HashSet<str>` lookups on
   non-Windows and avoid the throwaway `String` per walked path (MEM-02); keep the
   Windows fold behavior and the same membership decisions.
7. Deliver in one or a few commits; this is a sweep, not a redesign.

## Constraints / validation
- Visibility and sort decisions must be byte-for-byte identical; add or keep tests for
  glob and deep-directory cases.
- Report records stay identical (MIC-07) and the data plane is unchanged.
- Do not change any §12 non-finding.

## Done when
The micro hot spots (MIC-02/03/06/07) and the purge-key allocations (MEM-02) are
improved with identical behavior and tests passing. If any change cannot be shown
behavior-preserving, **stop** and record `skipped` for that sub-item with a one-line
reason.
