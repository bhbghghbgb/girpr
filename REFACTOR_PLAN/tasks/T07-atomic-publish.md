# T07 — Atomic temp-then-publish leaf

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F10 (lines 195–205) — atomic
  temp-then-publish implemented in four shapes.
- Assumptions to validate: §A3 (29–35), §A2 (21–27), §A1 (13–19).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§N6 `_tmp` resume artifacts,
  §N8 capture's deliberately non-atomic documents).

## Scope
- Candidate shared crate `crates/fs-publish` (`girpr-fs-publish`).
- Report names: `src/util/atomic.rs` (`write_atomic_with`),
  `src/repair/phases/post.rs` (`check_publish_target`, `copy_publish_with`),
  `tools/sled2redb/src/convert.rs` (`Builder::tempfile_in`, `persist` /
  `persist_noclobber`), `tools/sync/src/cache/backup/publish.rs` (working name +
  hard-link claim). Locate before editing.

## Implementation plan
1. Validate the four shapes in §F10 and identify the common primitive: create a
   `NamedTempFile` in the target directory, write, optionally sync, then
   `persist` / `persist_noclobber`.
2. Expose it from a leaf with explicit parameters for durability (`sync_all`),
   parent-directory creation, replace-vs-noclobber, and error type. Do not hide
   these in the crate.
3. Migrate the named call sites that share the primitive. Keep the policies
   local: girpr's injected copy closure and lexical guards, sled2redb's
   overwrite-vs-fresh choice, and girsync's claim protocol.
4. Explicitly exclude capture's named `.slot.tmp` blobs and non-atomic small
   documents (§F10, §N8) and girpr's `_tmp` resume artifacts (§N6).

## Constraints / validation
- No change to atomicity/durability guarantees or to the claim protocol
  (§F10, §N6).
- Byte-stable artifact writes must remain byte-stable (§A3).
- A new gircap leaf dep requires §A2 doc tables to be updated.

## Done when
The common temp-then-publish mechanics come from the leaf, each caller keeps its
durability/replace policy, excluded artifact writers are untouched, and all
publish/cache tests pass.
