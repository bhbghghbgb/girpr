# T10 — `girpr-path-scope-semantics` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F13 (lines 282–293) — "a filtered-out
  path is nonexistent" is one rule at four-plus call sites with an opposite mechanic in
  girsync; candidate boundary owns "apply scope to the cleaned `/` rel before any sink; a
  hidden path is kept by purge and absent from the plan" (`girpr-filter` already shared).
- Assumptions to validate: §A2 (29–36), §A5 (57–67), §A9 (88–97), §A14 (129–135).

## Scope
- New leaf (proposed `girpr-path-scope-semantics`). Sources: product `src/repair/scope/mod.rs`
  (invariant), `phases/plan.rs`, `purge.rs`, `phases/post.rs`, `phases/metadata.rs`,
  `file/mod.rs`; girsync `tools/sync/src/commands/sync/mod.rs` and its README.
  Locate before editing.

## Implementation plan
1. Validate §F13: confirm the invariant declaration and re-enforcements, and that girsync
   hides filtered entries from the effective map but must still stat the disk for blockers.
2. Define the leaf owning the two-sided rule over the cleaned `/` relative path; keep
   `girpr-filter` as the shared filter and do not duplicate it.
3. Migrate all callers directly (§A2); keep girsync's blocker stat behavior local/ intact.
4. Move through §A14 seams. Deliver in commits: leaf, product migration, girsync check, seams.

## Constraints / validation
- High-risk (§F13): widening the girsync map to retain hidden blockers would let the
  planner delete a path the filter promised to leave alone — do not do this.
- The rule also decides the `config.ini` bump (§F13); published metadata bytes must stay
  identical (§A5).
- Path spelling/case predicates are already leaves (§N13); do not re-propose them.

## Done when
The two-sided existence rule lives in the leaf, all callers use it, girsync still stats
blockers before the first change, and metadata/config bytes are unchanged. If the
girsync side cannot be kept safe, scope the task to the product only or **stop** and record
`skipped`.
