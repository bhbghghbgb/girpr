# T02 — Move and rename `girpr-sync-leaves` → `girsync-leaves`

## Goal / pointers
- `docs/shared-crate-ownership-findings.md` §F3 (lines 91–102) — consumed only by girsync,
  so girsync-owned; the consistent identity is `girsync-leaves` (`girsync_leaves`) at
  `tools/sync/leaves/`.
- §F1 `sync-leaves` row (line 82) and §F2 (lines 85–89) — validate the single-family
  classification against current manifests.
- §F7 (lines 141–148) — switch to the tools-only lint table after the move.
- §A7 (lines 36–39) and §F8 (lines 150–162) — workspace `members`, dependent path deps,
  path text in docs and any lockfile package entry for the rename.
- Assumptions to validate: §A2 (14–18), §A3 (19–23), §A4 (24–29), §A6 (33–35).
  Keep items: §F5 (116–123), §F9 (164–177).

## Scope
- `crates/sync-leaves/` in full → `tools/sync/leaves/`.
- Consumer manifest: `tools/sync/Cargo.toml` (the crate is also consumed directly by
  girsync's own integration tests — keep that).
- Workspace `members`, `Cargo.lock` entry, and path text in root `README.md`,
  `tools/sync/README.md`, `tools/sync/DESIGN-NOTES.md`, `.config/nextest.toml` comments
  and source doc comments. Locate occurrences in the current tree before editing.

## Implementation plan
1. Validate §F3/§F1: confirm no consumer outside the girsync family (girsync + sled2redb).
   If a second family has appeared, stop — do not move a shared crate (§A6, §F5).
2. Move the directory to `tools/sync/leaves/`; rename package and lib target to
   `girsync-leaves` / `girsync_leaves`; update its `description` to girsync ownership.
3. Switch to the tools-only lint table (§F7). Convert intra-workspace path deps from
   `../<crate>` to `../../crates/<crate>` (`filter`, `fs-identity`, `tempdir` dev-dep)
   without changing versions or features.
4. Update `tools/sync/Cargo.toml` to the new name/path, preserving direct consumption by
   girsync and its tests (no re-exporting facade).
5. Update every §F8 seam: workspace membership, `Cargo.lock`, README/design-note path
   text and nextest/doc comments.
6. Deliver in several commits if useful: move + manifest, consumer rewrite, lockfile, docs.

## Constraints / validation
- Keep the four production leaves (rw limiter/gate, filter policy, link classification,
  hardlink facts) and their dependencies unchanged; this is a location/name/lint change
  only (§F4 of the report describes the girpr group; §F3 covers this one).
- Do not touch shared leaves (`girpr-filter`, `girpr-fs-identity`, `girpr-tempdir`) beyond
  repointing this crate's path dependencies (§F6, lines 137–139).
- gircap's crates and the 19 shared crates stay untouched (§F5, §F9).

## Done when
`girsync-leaves` lives at `tools/sync/leaves/` with tools-only lints, girsync consumes it
directly by its new name, and all §F8 seams and `Cargo.lock` are updated with tests still
passing. If the classification premise is stale, **stop** and record `skipped` with a
one-line reason.
