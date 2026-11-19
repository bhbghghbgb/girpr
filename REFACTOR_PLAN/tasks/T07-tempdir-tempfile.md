# T07 — `girpr-tempdir` `TempDir` → `tempfile`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A10 (lines 304–331), §D
  (lines 504–509).
- Assumptions to validate: §I.13 (lines 853–855), §I.14 (lines 856–858).

## Scope
- Package: `crates/tempdir`, mainly `src/lib.rs` (`ROOT`, `root`, `SEQ`,
  `unique_path`, `TempDir::*`, `Drop`).

## Implementation plan
1. Keep a thin policy wrapper: the `<temp>/girpr-test/` root, recognizable
   `<prefix>-<tag>-…` naming, and the shared-root behavior.
2. Delegate creation and cleanup to `tempfile::Builder` (`tempdir_in` /
   `TempPath` for `at`); remove the manual pid + nanos + atomic-sequence counter.
3. `absent()` stays custom (a deliberately uncreated path). Keep
   `test_support.rs` unchanged.
4. Treat the girsync option builders / heavy dev dependency (§I.14) as a
   possible **follow-up**, not part of this task, unless the implementing
   session finds they cannot be separated.

## Constraints / validation
- Many integration tests and error-message assertions assume the
  `<prefix>-<tag>-<pid>-<nanos>-<seq>` name shape — validate the blast radius
  (§I.13). The name-shape and drop-cleanup tests (§A10) become obsolete; other
  tests stay.
- `tempfile` is a new dependency; cleanup with open handles is best-effort in
  both implementations.

## Done when
Creation/cleanup is delegated, the root policy and `absent()` remain, and the
affected tests are updated or removed.
