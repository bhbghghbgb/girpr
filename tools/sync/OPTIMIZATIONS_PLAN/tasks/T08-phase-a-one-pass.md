---
id: T08
title: Phase A one tree pass
batch: C
checkpoint: C
report: [L246-L278]
design_notes: "An included descendant pulls its ancestor directories into the map; filters must answer the same set"
depends_on: []
size: M
risk: medium
---

# T08 — Phase A one tree pass

## Goal

Reduce `scan_stat_only` from two full passes over the live tree to one pass
plus an ancestor fix-up over the included set. The effective map must be
identical: every included path, every ancestor directory pulled in by an
included descendant, and the same prune set.

## Why

The current first loop builds `live_set`/`dirs`; the second loop re-checks
membership and builds the map. The second pass exists because an ancestor
pulled in by a later entry must be visible to an earlier entry — but the walk is
parent-first, so the ancestor case is the only ordering dependency. An explicit
fix-up over included descendants is clearer and cheaper than re-testing every
entry.

## Files to read

- `src/effective/phase_a.rs` — `scan_stat_only` (the two loops, `live_set`,
  `dirs`, `alt_index`, the prune)
- `src/scan.rs` — `walk_live` ordering and `LiveEnt`
- `src/filter.rs` — `is_excluded`
- `src/effective/entry.rs` — `SideEntry::dir`, `put_dir_row`
- Existing include/ancestor tests in `tests/` and `src/effective/`

## Constraints

- **The map, `ScanStats`, and the prune set must be identical.** This is a
  construction-order change, not a filtering change.
- A later-included descendant must still pull in ancestors that were already
  skipped because they did not match `--include` themselves.
- An excluded directory still excludes its whole subtree via
  `is_excluded`/`matches_or_ancestor`; do not re-derive that rule.
- Alternate-case adoption and the stat-only row writes are unchanged.
- Do not rely solely on `walkdir` parent-before-child order without a test and
  a comment; the fix-up is what makes the dependency explicit.

## Implementation

1. Before the single loop, build a borrowed directory index:
   `let dirs: HashMap<&str, &LiveEnt> = live.iter().filter(|e| e.is_dir)...`.
   Keep it keyed by `&str` so no path strings are copied.
2. In the one loop:
   - skip `is_excluded`;
   - insert the entry's own key into `live_set` and the map;
   - for a directory, write its dir row as today;
   - for a file, walk its `/`-separated ancestors and record the ones that are
     in `dirs` and not yet in `live_set` in a small `pending_ancestors` set.
3. After the loop, for each pending ancestor:
   - insert it into `live_set` and the map as `SideEntry::dir(is_link)` using the
     `LiveEnt` from `dirs`;
   - write its dir row through the same batch handle (deduplicated, once);
   - repeat upward if the ancestor's own ancestors were not inserted (a
     pending set drained until empty, or just walk every prefix of every
     included file again and insert on miss).
   The simplest correct shape is a second pass over **included file rels only**
   (not the whole tree), walking prefixes and inserting missing directory
   entries; that is the “ancestor fix-up over included entries” this task
   intends.
4. Keep the log line and `ScanStats` counts computed from the final map, not
   from intermediate counters, so they cannot diverge.
5. Move the prune over `cached_keys` after the fix-up, unchanged.
6. Add tests:
   - `--include 'sub/*.txt'` pulls `sub` into the map even though `sub` itself
     does not match the include;
   - a nested include pulls every ancestor up to the root;
   - an excluded directory still hides its entire subtree;
   - `ScanStats.files`, `dirs`, `live`, and `pruned` match the old
     implementation's values on a fixture with included descendants.
   Prefer comparing maps/counters to asserting insertion order.

## Commit plan

1. `perf(sync): build the phase A map in one tree pass`
2. `test(sync): pin ancestor pull-in for a narrowed include`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync effective::
cargo test -p girsync filter::
cargo test -p girsync
```

## Acceptance

- One full traversal of `live`; the fix-up visits only included file paths.
- Every include/ancestor/exclude case produces the same map and prune set.
- Directory rows are written once per directory.
- Alternate-case adoption is unchanged.
- `ScanStats` is identical for a dry run and a real run.

## Reviewer focus

- Is there any entry whose inclusion depended on a *descendant* seen later?
- Can the fix-up miss an ancestor that the old first loop added?
- Are dir rows written twice for a directory that is both included and pulled
  in?
- Does the prune still see the final `live_set`?
- Did `is_excluded` semantics change?
