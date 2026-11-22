# T25 — `girpr-path-identity` crate

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F30 (lines 458–467) — `path_key` has
  two deliberate fold rules and three disagreeing identity questions; candidate boundary
  is a crate with three named question types and parameterized folds. Confidence high,
  risk high.
- Assumptions to validate: §A9 (88–97), §A5 (57–67), §A2 (29–36), §A14 (129–135), §N13 (750–752).

## Scope
- New crate (proposed `girpr-path-identity`). Sources: girsync `tools/sync/src/util.rs`
  (fold case on every host), product `crates/util-path/src/path.rs` (fold only on Windows),
  `tools/sync/DESIGN-NOTES.md` (three rules), plus canonicalization spellings spread across
  `girpr-fs-names`, `girpr-util-path`, `girpr-lock` and girsync's direct `dunce` dep.
  Locate before editing.

## Implementation plan
1. Validate §F30: confirm the two fold rules and the three named questions (map key,
   same-open-place, on-disk entry spelling).
2. Define the crate with three explicit question types and caller-supplied folds — no
   default fold that silently changes which files are touched.
3. Migrate callers directly (§A2), keeping each host's fold policy. Move through §A14
   seams where relevant.
4. Commit crate, migrations, seams.

## Constraints / validation
- High-risk (§F30): a single default fold silently changes which files a repair or mirror
  touches. Folds are keepers (§A9); do not unify them.
- Do not re-propose already-shared path spelling/case predicates, digest spelling or
  glob-compile options (N13). Preserve canonicalization spellings byte-for-byte (§A5).

## Done when
The three identity questions are named in a shared crate with parameterized folds, every
caller keeps its fold rule, and no path/mirror behavior changed.
