---
id: T12
title: Filter bundle and pre-built side labels
batch: E
checkpoint: E
report: [L419-L435, L495-L511]
design_notes: "Filters are treated as nonexistent on both sides; an excluded directory excludes its subtree"
depends_on: [T09, T10]
size: M
risk: low
---

# T12 — Filter bundle and pre-built side labels

## Goal

- Replace the `includes` / `excludes` pair with one `Filter` value that owns
  the compiled patterns and a cached `MatchOptions`.
- Build the `SideRequest` labels in `cmd_sync` before the call instead of
  borrowing `format!` temporaries.

`case_sensitive` **stays a separate `CommonOpts` field.** See "Why" below.

## Why

`is_excluded` takes three parameters and rebuilds `MatchOptions` on every call.
The include/exclude pair travels as two `CommonOpts` fields and two arguments
into every filter call site. Bundling the pair gives the rule one home, makes
a future subtree-exclusion memo possible, and removes a repeated per-entry
allocation.

`case_sensitive` is deliberately **not** part of the bundle. It is a run-wide
mode, not a filter property: it selects the cache's case flag, the planner's
`CasePairing`, the rename pass, and `ensure_mkdir_targets_free`. Folding it
into a `Filter` would make the case mode look like it belongs to globbing
alone, and every one of those other uses would have to reach through the
filter to get it back.

The label temporaries compile but read as if the call borrows a dangling
string; `compare` and `compare-self` already pre-build theirs.

## Files to read

- `src/filter.rs` — `compile_patterns`, `is_excluded`, `matches_or_ancestor`
- `src/config/common.rs` — `CommonOpts` and `TryFrom<CommonArgs>`
- `src/effective/phase_a.rs` — the filter call
- `src/effective/record.rs` — the filter call
- `src/commands/update.rs`, `src/commands/compare.rs`,
  `src/commands/compare_self.rs`, `src/commands/sync/mod.rs` — span fields and
  `CasePairing::from(...)`
- `src/planner/pairs.rs` — `SideRequest::label`
- `tests/common/mod.rs` — the `opts()` helper that builds a `CommonOpts`

## Constraints

- This is a **pure bundling refactor**: the set of excluded paths must be
  identical, including the ancestor rule for an excluded directory.
- `Filter` is a plain data holder, not a trait. No generic filter strategy.
- **`case_sensitive` stays on `CommonOpts`.** `Filter` reads it when it builds
  its `MatchOptions`; it does not own it.
- `MatchOptions` must be built from the run's `case_sensitive` once, not per
  call. `require_literal_separator: false` and
  `require_literal_leading_dot: false` stay.
- Do not change case-mode semantics, the rename pass, or the planner's
  `CasePairing`.
- Span fields must still show the include/exclude patterns and the case mode.
- `CommonOpts` is constructed by struct literal in about forty places under
  `tools/sync/tests/` (most of them via the `opts()` helper's
  `..opts()` spread). Replacing the two fields with one ripples into those
  literals; update them in the same commit, or the tree will not build.

## Implementation

1. In `filter.rs`:
   ```rust
   #[derive(Debug, Clone)]
   pub struct Filter {
       includes: Vec<Pattern>,
       excludes: Vec<Pattern>,
       opts: MatchOptions,
   }
   impl Filter {
       pub fn new(includes: Vec<Pattern>, excludes: Vec<Pattern>, case_sensitive: bool) -> Self;
       pub fn is_excluded(&self, rel: &str) -> bool;
       pub fn includes(&self) -> &[Pattern];
       pub fn excludes(&self) -> &[Pattern];
   }
   ```
   Move the body of `is_excluded` onto the impl, using `self.opts`. Keep a free
   `is_excluded(rel, includes, excludes, case_sensitive)` wrapper only if
   tests or external callers need it; otherwise migrate them.

2. In `CommonOpts`, replace `includes` and `excludes` with
   `pub filter: Filter`. Leave `case_sensitive` where it is. In
   `TryFrom<CommonArgs>`, build the filter once:

   ```rust
   let filter = Filter::new(
       compile_patterns(&a.include)?,
       compile_patterns(&a.exclude)?,
       a.case_sensitive,
   );
   ```

   Update every `common.includes` / `common.excludes` use to
   `common.filter.includes()` / `.excludes()`. Update the test literals in
   `tools/sync/tests/` in the same commit.

3. `phase_a.rs` and `record.rs` call `common.filter.is_excluded(&rel)`. Remove
   their local `includes` / `excludes` bindings.

4. In `commands/sync/mod.rs`, before `plan_pairs`:
   ```rust
   let src_label = format!("folder {}", src.display());
   let dst_label = format!("folder {}", dst.display());
   ```
   and pass `&src_label` / `&dst_label` in `SideRequest`. Do the same check in
   `compare` and `compare_self`; they already have locals, so only normalise
   the spelling if it differs.

5. Add tests in `filter.rs`:
   - an excluded directory still excludes a descendant, and `linkage` is not
     covered by `link`;
   - include-only-a-descendant keeps the descendant and excludes its sibling;
   - case mode changes matching exactly as before;
   - the cached `MatchOptions` matches the old per-call options on a table of
     patterns/paths.

6. Update span fields in the four commands to read `common.filter.*`; keep the
   same field names so log consumers do not change.

## Commit plan

1. `refactor(sync): bundle the include/exclude patterns into a Filter value`
2. `refactor(sync): build side labels before plan_pairs`
3. `test(sync): pin filter matching through the bundled value`

## Verification

```text
cargo fmt --all -- --check
cargo nextest run -p girsync filter
cargo nextest run -p girsync effective::
cargo nextest run -p girsync
```

## Acceptance

- The same paths are excluded for every include/exclude/case combination.
- `MatchOptions` is constructed once per run.
- `case_sensitive` is still a `CommonOpts` field, and the cache, planner,
  rename, and mkdir paths still read it from there.
- `SideRequest` labels are locals, not temporaries.
- No command changes its filtering or case pairing.
- Span/log field names are unchanged.

## Reviewer focus

- Did `case_sensitive` end up inside `Filter` after all?
- Is the ancestor rule still applied to every exclude?
- Is `MatchOptions` cached on the right value (per run, not per filter call)?
- Are the labels alive for the whole `plan_pairs` call?
- Did any log field disappear or get renamed?
- Were the test literals updated in the same commit?
