# T11 — girsync: extract the cache crate

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F24 (lines 300–308) — the girsync
  cache subtree is ~6,710 lines (`cache/open.rs` 2947, `cache/db/cachedb.rs`
  1507, `cache/backup/publish.rs` 877, `cache/write.rs` 568, `wire.rs` 241)
  and owns `redb`, `postcard`, `tempfile`.
- §F27 (327–333) — heavy deps group by subsystem but the single lib facade
  defeats isolation (§A2).
- Assumptions to validate: §A2 (27–33), §A10 (86–94, `util ↔ cache` cycle),
  §A4 (40–46), §A6 (55–61).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/sync/src/cache/**` (open, db/cachedb, backup/publish, write, wire,
  schema/lock/db), the `redb = "=4.6.0"` dep (`tools/sync/Cargo.toml:95`),
  postcard and tempfile.
- Cycle partner: `tools/sync/src/util.rs:34` (`util ↔ cache`).
- Test-only seams: `tools/sync/src/cache/mod.rs:106-115`,
  `cache/open.rs:150-155`.
- Locate all edges in the current tree before editing.

## Implementation plan
1. Validate the cache subtree size/dep concentration and map the `util ↔ cache`
   cycle. The boundary must sit around the whole strongly-connected group or at
   an acyclic leaf with shared types moved down (§A10).
2. Create the cache crate owning `redb`/`postcard`/`tempfile` and make girsync
   depend on it directly (§A2) so a change in planner/commands/other subsystems
   leaves the cache cached — and so sled2redb (which depends on the girsync lib,
   §F26) does not rebuild the cache on an unrelated girsync edit.
3. Promote test-only seams via the §A7 pattern (public lib + thin shim) without
   making one program's tests depend on another's (§A5).
4. Update `.config/nextest.toml` filters (§F31) and any doc seams (§F39) if a new
   shared crate is introduced.

## Constraints / validation
- §A2 payoff: the cache must be reachable directly by girsync (and ideally
   sled2redb) without the whole lib facade staying an ancestor of every target.
- Do not change cache/dump byte contracts (§A6) — postcard bytes and schema.
- Do not cut the `util ↔ cache` cycle illegally (§A10). If it cannot be broken,
   stop and raise a follow-up.
- Keep `redb` pinned as today (`=4.6.0`) (§F4).

## Done when
The cache subtree is a crate consumed directly by girsync, heavy native deps
stay cached across unrelated edits, cache byte contracts are unchanged, and all
girsync cache tests pass.
