# T20 — Glob filtering → `globset` / shared crate

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A8 (lines 258–276).
- Assumption to validate: §I.6 (lines 831–833).

## Scope
- Packages: root `girpr`, `src/repair/scope/filter.rs` (`MATCH_OPTIONS`,
  `PathScope`, `compile_patterns`); `tools/sync`, `src/filter.rs`
  (`compile_patterns`, `Filter`, `matches_or_ancestor`, `ExcludeMemo`),
  `src/lib.rs` re-export, `tests/public_surface.rs`.

## Implementation plan
1. Choose either a shared leaf crate `girpr-filter` (preserves semantics
   exactly) or migrate both filters to `globset`
   (`GlobBuilder::literal_separator(false).case_insensitive(true)`), which
   compiles once and is faster (§A8 candidate).

## Constraints / validation
- With `globset`, verify `*` still crosses `/`, `**` semantics, dotfile rules,
   "exclude beats include" and subtree rules (§I.6).
- The public re-export type changes — `tools/sync/tests/public_surface.rs` pins
   `girsync::glob::Pattern` (§A8 tests).
- Tests named in §A8.

## Done when
Both filters delegate to the chosen approach and all filter-semantics tests
pass.
