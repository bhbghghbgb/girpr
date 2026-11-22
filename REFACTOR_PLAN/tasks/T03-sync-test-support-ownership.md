# T03 — Move and rename `girpr-sync-test-support` → `girsync-test-support`

## Goal / pointers
- `docs/shared-crate-ownership-findings.md` §F3 (lines 91–102) — consumed only by girsync
  and sled2redb (both dev-dependencies), so girsync-owned; the consistent identity is
  `girsync-test-support` (`girsync_test_support`) at `tools/sync/test-support/`.
- §F1 `sync-test-support` row (line 83) and §F2 (lines 85–89) — validate the classification.
- §F6 (lines 135–136) — this crate depends on girsync and is taken as a dev-dependency by
  girsync and sled2redb; that inversion must survive the move.
- §F7 (lines 141–148) — switch to the tools-only lint table.
- §F8 (lines 150–162) — the girsync and sled2redb `tests/common/mod.rs` shims (lines
  159–162) and all path text move with the crate.
- Assumptions to validate: §A2 (14–18), §A3 (19–23, dev-deps count), §A4 (24–29), §A6
  (33–35). Keep items: §F5 (116–123), `guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/sync-test-support/` in full → `tools/sync/test-support/`.
- Consumer manifests: `tools/sync/Cargo.toml` and `tools/sled2redb/Cargo.toml`
  dev-dependencies.
- Shims: `tools/sync/tests/common/mod.rs` and `tools/sled2redb/tests/common/mod.rs`.
- `crates/tempdir/` manifest and doc comments that name this crate as the reason it stays
  girsync-free.
- Workspace `members`, `Cargo.lock`, root `README.md`, `tools/sync/README.md`,
  `tools/sled2redb/README.md`, `.config/nextest.toml` comments. Locate all occurrences in
  the current tree before editing.

## Implementation plan
1. Validate §F3/§F6: confirm the only consumers are girsync and sled2redb dev-dep edges,
   and that the crate still normal-depends on girsync. If a second program family consumes
   it, stop (§A6, §F5).
2. Move the directory to `tools/sync/test-support/`; rename package and lib target to
   `girsync-test-support` / `girsync_test_support`; update its `description`.
3. Switch to the tools-only lint table (§F7). Update its path deps for the new location:
   `girsync` becomes a sibling (`..`), and shared crates (`tempdir`, `log`) become
   `../../crates/<crate>`. Do not change versions or features.
4. Repoint both dev-dependency edges (girsync, sled2redb) to the new name/path; keep the
   `test-support → girsync` normal dependency as a test-only inversion (§F6).
5. Update the two `tests/common/mod.rs` shims: the `pub use` crate name and their doc
   comments, including the explicit re-export of the `#[macro_export]` helpers
   (`skip_if_case_insensitive`, `skip_if_no_link`). Call sites keep their `mod common;`
   and `common::helper(..)` spellings.
6. Update all remaining §F8 seams: workspace membership, `Cargo.lock`, READMEs, nextest
   comments and the `crates/tempdir` references.
7. Deliver in several commits if useful: move + manifest, consumer/shim rewrites, lockfile,
   docs.

## Constraints / validation
- Preserve the in-process runner and option/cache/planner fixture surface unchanged; this
  is a rename/move/lint change, not a redesign (§F6).
- Keep env pinning (`GIRSYNC_TEST_*`) and nextest process isolation intact — do not move
  tests between binaries or change their filters (§F8).
- Do not absorb the cache (T01) or any girsync leaves (T02) into this crate.
- Shared crates and gircap's crates stay untouched (§F5, §F9).

## Done when
`girsync-test-support` lives at `tools/sync/test-support/` with tools-only lints, both
dev-dep edges and the `→ girsync` inversion survive, both shims compile unchanged at call
sites, and all §F8 seams plus `Cargo.lock` are updated. If a premise is stale, **stop**
and record `skipped` with a one-line reason.
