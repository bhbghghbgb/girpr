# T21 — Atomic publish helpers → library

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §B4 (lines 428–454).

## Scope
- Package: root `girpr`, `src/util/atomic.rs`, `temp_sibling` in
  `src/util/path.rs`, the pure `copy_publish_with` path in
  `src/repair/phases/post.rs`, and `src/repair/file/temp.rs` (for the exclusion
  below).

## Implementation plan
1. Adopt `atomic-write-file`, `atomicwrites` or
   `tempfile::NamedTempFile::persist()` for `write_atomic` and the pure
   `copy_publish_with` path.
2. Keep the lexical guards (`ends_in_a_file_name`, Windows trailing-dot/space
   rules), `create_dir_all` and the `durable` decision.
3. **Do not** delegate `promote_tmp` or the audio cache move — the `_tmp` files
   are durable resume artifacts (length-based resume, rollback to chunk offset,
   keep-source-on-failure) that a generic temp-file crate would discard (§B4).

## Constraints / validation
- Temp names change (no `.girpr_tmp` sibling); `GIRPR_TEST_NO_FSYNC` may lose its
  seam; sync/durability granularity may change.
- Verify the crate's Windows overwrite semantics and whether it creates parents.
- Tests named in §B4 (atomic.rs, path.rs `temp_sibling`, post.rs, temp.rs).

## Done when
The clean publish paths are delegated, resume artifacts remain hand-rolled, and
tests pass.
