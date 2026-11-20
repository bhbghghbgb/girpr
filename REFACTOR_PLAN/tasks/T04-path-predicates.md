# T04 — Trailing-dot/space and path-key predicates

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F7 (lines 166–175) — duplicated
  trailing-dot/space and ASCII-fold rules with deliberate host policy.
- Assumptions to validate: §A3 (29–35), §A6 (52–60), §A2 (21–27).
- Related: T03 (`girpr-fs-names`); reuse that crate if it completed.

## Scope
- Report names: `src/util/path.rs`
  (`has_windows_trailing_dot_or_space`,
  `any_component_has_windows_trailing_dot_or_space`, `path_key`, `slash_rel`,
  component normalisation), `tools/capture/src/paths.rs` (`path_rule`),
  `tools/sync/src/util.rs` (`path_key`, `slash_rel`). Locate before editing.

## Implementation plan
1. Validate the deliberate differences in §F7: product folds case only on
   Windows while girsync folds unconditionally (case-insensitive pairing is a
   user option); `slash_rel` preserves `\` on POSIX in girsync but product
   rewrites it; product normalises `\` and `.` components while capture refuses
   both.
2. Add shared predicates for the common mechanics (trailing dot/space detection,
   ASCII case folding, forward-slash rendering) to `girpr-fs-names`, each with
   an **explicit host-policy parameter** rather than an implicit host check.
3. Route call sites through them with the parameters that reproduce their
   current behavior. Keep the two `path_key` behaviours side by side — do not
   merge them into one function.
4. Keep each tool's component normalisation/acceptance policy local (§A6).

## Constraints / validation
- The two `path_key` behaviours must not be merged (§F7, §A6).
- No path rendering or pairing behavior change; byte-stable and pairing tests
  must pass (§A3).
- A new gircap leaf dep requires §A2 doc tables to be updated.

## Done when
Common path predicates are sourced from the shared crate, host policy is an
explicit parameter, both `path_key` variants remain, and no path test changes.
