# T28 — `cache/open.rs` publish protocol restructure

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §E5 (lines 594–619).

## Scope
- Package: `tools/sync`, `src/cache/open.rs` (~3,644 lines): `open_db`,
  `backup_db`/`snapshot_old`, `retain_copies`, `copy_aside`, `publish_copy`,
  `publish_file`, `link_file`, `publish_dir`, claim/hold helpers,
  `retain_siblings`.

## Implementation plan
1. Extract a `backup/` submodule with a `Publisher` trait.
2. Move the partial+rename mechanics behind it, leaving the cross-process claim
   protocol and registries behind a narrow interface.
3. Delegate individual mechanics: `atomicwrites`/`tempfile` for the file-copy
   path and `tempfile` for working names. Do **not** add `fslock` over std
   `try_lock`.

## Constraints / validation
- The module docs enumerate one-syscall gaps that must survive: "a name in a
  copy list never holds half a copy" and fail-fast semantics (§E5).
- Tests named in §E5 (lines 613–615): `tests/migration_lock.rs` and inline tests
   pinning publish/retention names — the safety net for any restructure.
- Highest payoff, highest risk. If the safety net is insufficient in the current
   tree, stop and record as `skipped`.

## Done when
The protocol is restructured behind the trait, invariants are preserved, and the
safety-net tests pass. (Largest/riskiest task — own review checkpoint.)
