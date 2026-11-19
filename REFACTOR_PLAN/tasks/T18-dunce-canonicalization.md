# T18 — girsync path canonicalization → `dunce`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A7 (lines 234–256).
- Assumption to validate: §I.1 (lines 809–816).

## Scope
- Package: `tools/sync`, `src/commands/sync/plan/link.rs`
  (`without_verbatim`, incl. `\\?\UNC\`), `src/effective/side.rs`
  (`display_path`), `src/util.rs` (`canonical_or_raw`, `same_path`).

## Implementation plan
1. Use `dunce::canonicalize` consistently so symlinks resolve without the
   Windows `\\?\` prefix.
2. Remove the three hand-rolled verbatim-prefix strippers. Do not mix with
   `std::fs::canonicalize` — that recreates the prefix mismatch.

## Constraints / validation
- User-facing error paths lose `\\?\` without post-processing.
- `dunce` does **not** replace girpr's `resolve_existing_prefix` — that is T19,
   not this task (§I.1).
- Tests named in §A7 (lines 249–251), including the UNC rebuild test and message
  assertions.

## Done when
The three strippers are gone, all canonicalization goes through `dunce`, and
message tests pass.
