# T25 — Hardlink identity → `same-file`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §E2 (lines 546–567).

## Scope
- Package: `tools/sync`, `src/hardlink.rs` (`Identity`, `identity_of`, Unix and
  Windows FFI), callers `src/commands/sync/apply/copy.rs` (lines 201–213) and
  `src/commands/sync/plan/safety.rs`, plus the Windows-only `windows-sys`
  dependency used only here.

## Implementation plan
1. Replace `Identity` + `identity_of` with `same-file`'s `FileId` /
   `is_same_file` (already in `Cargo.lock` via `walkdir` — zero new dependency);
   `file-id` is an alternative.
2. Keep `LinkInfo`: source link counts from `MetadataExt::nlink` on Unix and a
   minimal `nNumberOfLinks` read on Windows (or use `is_same_file` where the
   count guard is unnecessary).
3. Drop the now-unused Windows-only `windows-sys` dependency.

## Constraints / validation
- The "link count > 1" guard must remain conservative — a pure identity check
   would miss a dst hard link to a *different* src file (§E2 caveat, copy.rs
   lines 194–199).
- Tests named in §E2 and `tests/sync.rs`.

## Done when
The hand-rolled FFI is gone, the link-count guard is preserved, and tests pass.
