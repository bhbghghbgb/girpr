# T05 — Canonicalize + `\\?\` stripping helpers

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F8 (lines 177–183) — repeated
  canonicalize + verbatim-prefix stripping and the fail-closed prefix routine.
- Assumptions to validate: §A3 (29–35), §A2 (21–27).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§N6 for the `\\?\`-related
  "hand-rolled code that should stay" items — do not cross into them).

## Scope
- Candidate home: `girpr-fs-names` (T03/T04) or a dedicated path leaf.
- Report names: `tools/sync/src/plan/safety.rs`, `tools/sync/src/commands/sync/mod.rs`,
  `tools/sync/src/apply/copy.rs`, `tools/sync/src/apply/phases.rs`,
  `tools/sync/src/plan/link.rs`, `tools/capture/src/verify/list.rs`,
  `tools/capture/src/verify/bundle.rs`, `src/util/path.rs`
  (`resolve_existing_prefix`, lines 450–477). Locate before editing.

## Implementation plan
1. Validate which `dunce::canonicalize` call sites also strip the `\\?\`
   verbatim prefix and confirm the shared helper contract needed.
2. Add `canonical_no_verbatim` (canonicalize then strip `\\?\`) and, if the
   product routine is genuinely the same shape, `canonical_existing_prefix` to
   the path leaf.
3. Migrate the girsync and capture call sites to the shared helper. Keep girpr's
   fail-closed longest-existing-prefix routine **unchanged** unless its behavior
   is provably identical — §F8 frames it as a separate variant, not a duplicate
   to delete.
4. Update any doc/comment that names the old call sites.

## Constraints / validation
- No resolution behavior change; symlink-escape detection and fail-closed
  semantics in girpr must survive (§N6).
- Do not migrate call sites that rely on `std::fs::canonicalize` returning the
  verbatim path; verify each site before switching.
- A new gircap leaf dep requires §A2 doc tables to be updated.

## Done when
`dunce`-based canonicalize + strip is sourced from the shared leaf at the named
call sites, girpr's prefix routine is untouched, and all path/safety tests pass.
