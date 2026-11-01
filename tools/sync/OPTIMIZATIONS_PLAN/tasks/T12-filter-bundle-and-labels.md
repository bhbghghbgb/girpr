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

- Replace the `includes` / `excludes` / `case_sensitive` triple with one
  `Filter` value that owns the compiled patterns and a cached `MatchOptions`.
- Build the `SideRequest` labels in `cmd_sync` before the call instead of
  borrowing `format!` temporaries.

## Why

The triple travels as three parameters or three `CommonOpts` fields, and
`is_excluded` rebuilds `MatchOptions` on every call. Bundling gives the rule one
home, makes a future subtree-exclusion memo possible, and removes a repeated
per-entry allocation. The label temporaries compile but read as if the call
borrows a dangling string; `compare` and `compare-self` already pre-build
theirs.

## Files to read

- `src/filter.rs` — `compile_patterns`, `is_excluded`, `matches_or_ancestor`
- `src/config/common.rs` — `CommonOpts` and `TryFrom<CommonArgs>`
- `src/effective/phase_a.rs` — the filter call
- `src/effective/record.rs` — the filter call
- `src/commands/{update,compare,compare_self,sync}.rs` — span fields and
  `CasePairing::from(...)`
- `src/commands/sync/mod.rs` — `SideRequest` labels
- `src/planner/pairs.rs` — `SideRequest::label`

## Constraints

- This is a **pure bundling refactor**: the set of excluded paths must be
  identical, including the ancestor rule for an excluded directory.
- `Filter` is a plain data holder, not a trait. No generic filter strategy.
- Keep the public-ish module path minimal; `Filter` can live in `filter.rs`
  and be re-exported from `config` if that reads better.
- `MatchOptions` must be built from the run's `case_sensitive` once, not per
  call. `require_literal_separator: false` and
  `require_literal_leading_dot: false` stay.
- Do not change case-mode semantics, the rename pass, or the planner's
  `CasePairing`.
- Span fields must still show the include/exclude patterns and the case mode.

## Implementation

1. In `filter.rs`:
   ```rust
   #[derive(Debug, Clone)]
   pub struct Filter {
       includes: Vec<Pattern>,
       excludes: Vec<Pattern>,
       case_sensitive: bool,
       opts: MatchOptions,
   }
   impl Filter {
       pub fn new(includes: Vec<Pattern>, excludes: Vec<Pattern>, case_sensitive: bool) -> Self;
       pub fn is_excluded(&self, rel: &str) -> bool;
       pub fn includes(&self) -> &[Pattern];
       pub fn excludes(&self) -> &[Pattern];
       pub fn case_sensitive(&self) -> bool;
   }
   ```
   Move the body of `is_excluded` onto the impl, using `self.opts`. Keep a free
   `is_excluded(rel, includes, excludes, case_sensitive)` wrapper only if tests
   or external callers need it; otherwise migrate them.
2. In `CommonOpts`, replace the three fields with `pub filter: Filter`. In
   `TryFrom<CommonArgs>`, compile the patterns and build the filter once.
   Update every `common.includes` / `common.excludes` / `common.case_sensitive`
   use to `common.filter.includes()` / `.excludes()` / `.case_sensitive()`
   (or keep a `CommonOpts::case_sensitive()` convenience if it reduces churn).
3. `phase_a.rs` and `record.rs` call `common.filter.is_excluded(&rel)`.
   Remove their local `includes`/`excludes`/`case_sensitive` bindings.
4. In `commands/sync/mod.rs`, before `plan_pairs`:
   ```rust
   let src_label = format!("folder {}", src.display());
   let dst_label = format!("folder {}", dst.display());
   ```
   and pass `&src_label` / `&dst_label` in `SideRequest`. Do the same check in
   `compare` and `compare_self`; they should already have locals, so only
   normalise the spelling if it differs.
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

1. `refactor(sync): bundle include/exclude/case into a Filter value`
2. `refactor(sync): build side labels before plan_pairs`
3. `test(sync): pin filter matching through the bundled value`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync filter::
cargo test -p girsync effective::
cargo test -p girsync
```

## Acceptance

- The same paths are excluded for every include/exclude/case combination.
- `MatchOptions` is constructed once per run.
- `SideRequest` labels are locals, not temporaries.
- No command changes its filtering or case pairing.
- Span/log field names are unchanged.

## Reviewer focus

- Did the bundling accidentally make the case mode local to filtering and not
  reach `CasePairing`?
- Is the ancestor rule still applied to every exclude?
- Is `MatchOptions` cached on the right value (per run, not per filter call)?
- Are the labels alive for the whole `plan_pairs` call?
- Did any log field disappear or get renamed?
