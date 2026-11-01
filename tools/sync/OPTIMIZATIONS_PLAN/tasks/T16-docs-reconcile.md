---
id: T16
title: Reconcile documentation with the code
batch: F
checkpoint: F
report: [L513-L531]
design_notes: "Stale must-be-true statements teach the next reader the wrong invariant"
depends_on: [T15]
size: M
risk: low
---

# T16 — Reconcile documentation with the code

## Goal

Make `DESIGN-NOTES.md` and `README.md` describe the code as it is after
T01–T15. Resolve every contradiction rather than adding a new note beside an
old one. In particular:

- the by-name digest rule after T01 widened `get_algo` to see known names in
  the foreign arm;
- the backup policy, dst `old-*`-only snapshot, and failure story from T09;
- the cache memory knob and one-time test-cache decision from T10;
- the `Filter` bundle and pre-built side labels from T12 (only where the docs
  named the old triple);
- the phase names from T15;
- source-layout paths that have moved (`apply/` directory, `cache/db/` files,
  `planner/`, `report/`, etc.).

## Why

Both documents are treated as the contract. A stale “must be true” statement is
worse than no statement, because it teaches the next reader the wrong
invariant. The report's §3.6 says the by-name note is the safest rule for 1.1,
but the code has now moved, so the note must move with it.

## Files to read

- `tools/sync/DESIGN-NOTES.md` — all of it, especially “A digest is looked up
  by name, not by slot”, the link/copy invariants, and “Known limits”
- `tools/sync/README.md` — source layout, commands, backups, sync steps,
  cache semantics, known limits
- The diffs/commits of T01–T15, not just their plan files
- `src/hash/digestset.rs` — the final `get_algo`/`get_named` behaviour
- `src/config/common.rs`, `src/filter.rs`, `src/cache/db/backend.rs`,
  `src/commands/sync/mod.rs` — the final shapes

## Constraints

- **One statement per rule.** Delete or rewrite the stale sentence; do not
  leave the old one and add a “now also” paragraph.
- Keep the documents' voice and structure: bold invariant, explanation,
  “Enforced by” tests. Do not turn them into changelogs.
- README is user-facing: document observable behaviour (flags, files left
  behind, defaults), not internal function names.
- DESIGN-NOTES is implementation-facing: name the types/functions that carry
  the invariant.
- Do not document a plan or a future task. Describe only what is in the code at
  this checkpoint.
- Do not add references to `OPTIMIZATIONS_REPORT.md` or this plan directory;
  those are deleted at the end.

## Implementation

1. **By-name digest rule.** Read the final `DigestSet` implementation. Rewrite
   the DESIGN-NOTES section so it states the actual contract: a known algorithm
   is found in the foreign arm by name/spelling, an unknown name is exact, and
   `get_algo`/`get_named` agree. Update the “Enforced by” tests to the names
   that exist after T01. If T01 kept a slot-only `get`, say who may call it and
   why.
2. **Backups.** Replace the README sync step and every `backup-*` / `old-*`
   mention with the T09 policy: defaults, `--no-backup`, `--backup-retain N`,
   dst gets only `old-*`, and what a failed `--ignore-cache` removal leaves.
   Update the concurrency note that counted two `fs::copy` snapshots.
3. **Cache size / test cache.** Add or update the README cache-memory note
   (`GIRSYNC_CACHE_SIZE_MIB`, default 64, per handle) and the test-cache
   precedence. If DESIGN-NOTES mentions the hardcoded size, update it.
4. **Filter bundle.** Where README/DESIGN-NOTES describe the filter triple,
   describe the `Filter` value and the cached match options. Do not rename
   user flags.
5. **Phase names.** Make the README sync steps and the T15 module-doc table use
   the same names. Remove any old step numbering that disagrees.
6. **Source layout.** Update the README table for files that are now
   directories or moved: `commands/sync/apply/` (`mod.rs`, `copy.rs`,
   `phases.rs`), `cache/db/` (`mod.rs`, `cachedb.rs`, `backend.rs`,
   `handle.rs`), `planner/`, `report/`, `effective/`, and anything else the
   table still names as a flat file that no longer exists. Verify with
   `Get-ChildItem src -Recurse -File`.
7. **Known limits.** Re-read “Known limits, deliberately not fixed” and the
   README equivalents. Remove any limit a task fixed (for example a cache-scan
   note that T07 changed) and keep the ones that remain. Do not add new limits
   the tasks did not create.
8. Run a consistency grep for the old names/phrases (`unique_sibling` if it
   moved, `CACHE_SIZE`, `includes`/`excludes` as fields, `backup_db(dst)`,
   `apply.rs`) and fix or remove each hit in the docs.

## Commit plan

1. `docs(sync): reconcile the digest lookup rule with the code`
2. `docs(sync): document backups, cache size, and the filter bundle`
3. `docs(sync): update the source layout and phase names`
4. `docs(sync): re-check known limits against the implemented tasks`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync
```

Docs-only; the test run is to prove no code change slipped in.

## Acceptance

- No sentence in either document contradicts the shipped code.
- The by-name section describes the final lookup behaviour and names existing
  tests.
- Backup, cache-size, and test-cache docs match the flags and defaults.
- The source-layout table lists real paths.
- No reference to the optimization report or plan directory remains.
- No stale “known limit” that a task removed.

## Reviewer focus

- Is any old rule still present beside its replacement?
- Does the by-name note overclaim (for example folding unknown names)?
- Are the README backup steps and the actual `cmd_sync` order the same?
- Do the source-layout paths exist?
- Did a “known limit” survive a task that fixed it?
