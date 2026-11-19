# T19 — girpr `resolve_existing_prefix` validation

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A7 (lines 240–241, 254–256).
- Assumption to validate: §I.1 (lines 809–816).

## Scope
- Package: root `girpr`, `src/util/path.rs` (`resolve_existing_prefix`, lines
  438–480; `contained_join` tests lines 937–1007).

## Implementation plan
1. Determine whether any library method can replace the
   canonicalize-nearest-existing-ancestor routine while **resolving symlinks**.
   Note: `path-absolutize::Absolutize::absolutize()` is purely lexical; the
   method that canonicalizes an existing prefix and appends the remainder is
   `canonicalize()` — validate it resolves symlinks. `dunce` does **not** replace
   this routine (§I.1).
2. If a method demonstrably preserves symlink-escape detection, adopt it;
   otherwise stop.

## Constraints / validation
- The `contained_join` tests require symlink-escape detection; any replacement
  must keep them passing.
- If no suitable library method exists, mark the task `skipped` with the reason
  (this finding was already flagged as an open assumption).

## Done when
Either the routine is replaced by a validated library method, or the task is
recorded as skipped with a pointer to §I.1.
