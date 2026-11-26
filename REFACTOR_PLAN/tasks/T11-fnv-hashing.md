# T11 — FNV-1a hashing → fnv

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §APP-7 (lines 347–357) — the hand-rolled
  FNV-1a key hash in the run lock becomes `fnv::FnvHasher`.
- §CAP-5 (lines 712–723) — the capture test-support `fnv1a` becomes the same hasher;
  its recursive source/bundle discovery becomes `walkdir`.
- §X4 (lines 127–134) — cross-cutting small bit/hash/map mechanics.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md`, especially `File::lock` (§A9 lines 40–41).

## Scope
- `app/lock/src/lib.rs` — constants (`:40–43`), `key_for`/`fnv_step`/`fnv_mix`
  (`:145–190`); case-folding tests at `:797`, `:824`, `:841`, `:853`.
- `tools/capture/test-support/src/lib.rs` — `fnv1a` (`:207–214`), `collect_rs_files`
  (`:216–229`), `capture_source_key` (`:231–258`), `bundle_source_key` (`:271–287`),
  `find_bundle` (`:401–419`).
- `app/lock/Cargo.toml`, `tools/capture/test-support/Cargo.toml`.

## Implementation plan
1. Validate byte order, offset and prime of the local FNV-1a against `fnv`'s constants;
   if they differ and lock filenames matter, decide explicitly (old lock files simply
   become unrelated, §APP-7 line 357).
2. In the lock, use `FnvHasher`: write the lossy path string bytes, a `0xff` separator,
   then raw OS bytes / folded UTF-16 units. Keep case folding in the caller; do not
   introduce a file-lock dependency (§A9).
3. In capture test support, use `FnvHasher` over path/file bytes and replace the
   recursive `read_dir` walker with `WalkDir` filtered to `.rs`/`MANIFEST.json`, with
   explicit `sort_by_file_name` for stable fingerprints.
4. Run the lock case-folding/distinct-key tests (adjust only if byte order intentionally
   changed) and the capture layout/offline-bundle/schema tests; bump `FORMAT` once if
   the shared bundle cache key changes.

## Constraints / validation
- Preserving the exact hash byte order keeps existing lock filenames; otherwise note
  the intentional invalidation in the task note.
- `fnv` and `walkdir` are already locked; re-check versions. Sorting is still required
  for stable fingerprints.
- No test pins the numeric capture key; do not add one (§CAP-5 lines 720–723).

## Done when
Both hand-rolled FNV loops and the recursive walker are gone, lock tests pass and
capture fingerprints remain stable. If a premise is stale or a keeper would change,
**stop** and record `skipped`.
