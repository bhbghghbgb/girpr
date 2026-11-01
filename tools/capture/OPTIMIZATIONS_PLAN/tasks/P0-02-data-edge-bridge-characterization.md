# P0-02 — Data-edge and bridge characterization tests

Report: `../../OPTIMIZATIONS_REPORT.md` lines 833-850 (Phase 0 list), plus the
finding sections named below. This task changes tests only.

Dependencies: none.

## Goal

Pin today's behavior for the data-shape and bridge cases later tasks will
change:

1. a folder entry (`is_folder`) reaching the diff/selection pool (A3, lines
   97-113);
2. a non-empty `chunk_download.url_suffix` (A5, lines 133-148);
3. repeated response headers with the same name (A12, lines 264-276);
4. the bridge's `manifest_index` when old and new builds declare the same
   manifest id (C8, lines 590-619, especially the `manifest_index` paragraph);
5. (optional if cheap) the bridge's manifest cache lock being held during disk
   IO, or the `error_shape(extra)` parameter being discarded. Only pin these if
   the existing synthetic mini-bundle makes it trivial; otherwise leave them to
   task P3-02.

Characterization only: assert current behavior, including the lossy/overwrite
case. Do not fix it here.

## Where to start

- Folder case: `src/manifest/diff.rs` (`diff_field`), `src/select/mod.rs`
  (`candidates`), and `src/manifest/tests.rs` / `src/select/tests.rs`. Build a
  manifest pair with one non-folder change and one folder entry; assert whether
  the folder becomes a candidate today.
- URL suffix: `src/manifest/mod.rs` (`CapturedManifest::chunk_url`) and
  `src/blobs/mod.rs` (`Sources::from_manifests`, `Sources::url`). A unit test
  with a hand-built `CapturedManifest` is enough; assert the current
  suffix-less URL.
- Headers: `src/http/record.rs` (`header_map`) and its tests. Build a
  `HeaderMap` with two values for one name and assert the current single-value
  result.
- Bridge id collision: `tests/artifact_bridge/server.rs` (`Served::load`,
  `manifest_index`, `manifest_resolution`) and `tests/artifact_server.rs`. Add a
  mini-bundle case where both tags declare the same manifest id and assert what
  the current index resolves to.

## Steps

1. Prefer in-module unit tests; the bridge case may need the synthetic
   `MiniBundle`.
2. Assert the field or route under test, not a whole document.
3. For the folder case, record both the diff row and whether selection admits
   it, because the later policy decision may keep it in the diff but exclude it
   from selection.
4. For the URL case, include an empty-suffix control so the test shows the
   difference is only the suffix.
5. Keep the tests independent of `artifacts/` and the network.

## Tests and acceptance

- Each case has a deterministic test that fails if today's behavior changes.
- `cargo test -p gircap` passes; `artifact_server` tests still run without the
  real bundle.
- No production code changes and no changes to `STATE.md`.

## Commit plan

1. `test(manifest): characterize folder entries and chunk URL suffixes`
2. `test(bridge): characterize repeated headers and shared manifest ids`

## Notes

- Do not reference the report or plan in test names/comments.
- If the shared-id behavior cannot be reached because the mini-bundle builder
  always uses distinct ids, extend the builder locally in the test rather than
  changing production code.
