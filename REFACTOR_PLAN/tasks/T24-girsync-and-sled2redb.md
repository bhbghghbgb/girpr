# T24 — Cut girsync and sled2redb I/O and transaction cost

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §9 **TOOL-02** (lines 654–659) — girsync's live walk
  calls `std::fs::metadata(ent.path())` even though `walkdir::DirEntry` caches metadata
  on Windows; use `ent.metadata()?` and keep `path_is_symlink()` for the link flag.
- §9 **TOOL-16** (756–762) — a fresh 512 KiB buffer is allocated per girsync hashed
  file; add `hash_file_with_buf(path, algos, &mut [u8])` and give each rayon worker a
  reusable buffer via `for_each_init` (the copy path already takes a caller-owned
  buffer).
- §9 **TOOL-18** (770–777) — sled2redb stages the whole migration in one giant redb
  transaction; use `cache.begin_write()` and its standard batching (`COMMIT_BATCH` /
  `COMMIT_BYTES` / `COMMIT_INTERVAL`), keeping a final commit and atomic publish.
- Keep items: girsync's pinned `postcard` digest vocabulary and file record shapes,
  atomic redb publish, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/sync/src/scan.rs` (~200–202), `tools/sync/cache/src/hash/stream.rs`
  (~74–91), `tools/sync/src/effective/phase_c.rs` (~139–140),
  `tools/sync/src/commands/sync/apply/copy.rs` (~33–54),
  `tools/sled2redb/src/convert.rs` (~239–252).
- Locate current code before editing; the list above is a hint.

## Implementation plan
1. Confirm the redundant stat, per-file buffer allocation and single transaction in
   the current tree. If the dependency version does not cache `DirEntry` metadata,
   make TOOL-02 a no-op rather than guessing.
2. Use `ent.metadata()?` while retaining the symlink flag detection (TOOL-02).
3. Add `hash_file_with_buf` and give each rayon worker a reusable buffer; reuse the
   existing copy-path buffer style (TOOL-16).
4. Batch the redb migration through `cache.begin_write()` with periodic commits and a
   final commit, keeping the temporary-file publish atomic (TOOL-18).
5. Deliver in several commits if useful: stat, buffers, batched transaction.

## Constraints / validation
- The cache's `postcard` digest vocabulary and file record shapes stay byte-for-byte;
  IO reductions must not change stored bytes.
- Intermediate redb commits must remain safe (destination is a temporary file
  published only at the end).
- Do not change any §12 non-finding.

## Done when
Redundant stats and per-file allocations are removed, the migration uses bounded
batches with an atomic publish, stored bytes are unchanged, and tests pass. If a
premise is stale or the wire vocabulary would change, **stop** and record `skipped`
with a one-line reason.
