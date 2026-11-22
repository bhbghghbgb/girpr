# T01 — Move and rename `girpr-sync-cache` → `girsync-cache`

## Goal / pointers
- `docs/shared-crate-ownership-findings.md` §F3 (lines 91–102) — this crate is consumed
  only by girsync and its converter sled2redb, so it is girsync-owned; the consistent
  identity is `girsync-cache` (`girsync_cache`) at `tools/sync/cache/`.
- §F1 `sync-cache` row (line 81) and §F2 (lines 85–89) — confirm the single-family
  classification before editing.
- §F7 (lines 141–148) — crates moved under `tools/sync/` switch from
  `[lints] workspace = true` to the tools-only `[lints.rust]` / `[lints.clippy]` table.
- §F6 (lines 132–134) — sled2redb consumes this crate **directly**, not through girsync's
  lib facade, and the crate pins the digest vocabulary in its `postcard` wire format.
- §A7 (lines 36–39) and §F8 (lines 150–162) — a move also changes workspace `members`,
  dependent path deps, `Cargo.lock` package entries and path text in docs.
- Assumptions to validate: §A2 (14–18), §A3 (19–23), §A4 (24–29), §A6 (33–35).
  Keep items: §F5 (116–123), §F9 (164–177), `guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/sync-cache/` in full (manifest, source, tests) → `tools/sync/cache/`.
- Direct consumer manifests: `tools/sync/Cargo.toml`, `tools/sled2redb/Cargo.toml`.
- Workspace membership and `Cargo.lock` package entry/dependency lists.
- Path text: root `README.md`, `tools/sync/README.md`, `tools/sync/DESIGN-NOTES.md`,
  `tools/sled2redb/README.md`, `.config/nextest.toml` comments and source doc comments.
  Locate every occurrence in the current tree before editing; the list above is a hint.

## Implementation plan
1. Validate §F3/§F1 against current manifests: the only consumers are girsync and
   sled2redb. If a second program family has appeared, stop — this is a shared crate and
   must not move (§A6, §F5, §F9).
2. Move the crate directory to `tools/sync/cache/`; rename the package to `girsync-cache`
   and the lib target to `girsync_cache`; update its `description` to girsync ownership.
3. Switch its lint table to the tools-only subset (§F7). Convert its intra-workspace path
   deps from `../<crate>` to `../../crates/<crate>` (`time`, `fs-identity`, `digest`,
   `tempdir` dev-dep) without changing any version or feature.
4. Update the two direct consumers to the new name and path, keeping sled2redb's **direct**
   edge (§F6) and girsync's direct dependency for its own tests.
5. Update all §F8 seams: root workspace `members`, `Cargo.lock` renames and dependent
   dependency lists, and every path/name string in the READMEs, design notes, nextest
   comments and doc comments that names the old crate.
6. Deliver in several commits if useful: directory move + manifest, consumer rewrites,
   lockfile, doc seams.

## Constraints / validation
- Preserve the `postcard` row codec, `FileRec`/`Meta` serde shapes and the pinned digest
  vocabulary byte-for-byte (§F6, lines 132–134). No schema, key order or wording change.
- Do not route sled2redb through girsync's lib facade (§F6).
- Do not touch the 19 shared crates or gircap's crates (§F5, §F9).
- The crate must end under `tools/sync/` with the tools-only lint table (§F7, §A4).
- Verify the workspace still builds and tests resolve with the renamed package.

## Done when
`girsync-cache` lives at `tools/sync/cache/`, tools-only lints, both consumers depend on
it directly by its new name, all §F8 seams and `Cargo.lock` are updated, and the wire
format and tests are unchanged. If the ownership premise is stale or a keeper would
change, **stop** and record `skipped` with a one-line reason.
