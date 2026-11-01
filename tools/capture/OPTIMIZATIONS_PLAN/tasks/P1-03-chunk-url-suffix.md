# P1-03 — Keep chunk URL suffixes

Report: `../../OPTIMIZATIONS_REPORT.md` lines 133-148 (A5).

Dependencies: P0-02.

## Goal

Make `blobs::Sources` build the same `{prefix}/{id}{suffix}` URL as
`CapturedManifest::chunk_url`. Today `Sources::from_manifests` stores only the
prefix and `Sources::url` drops a non-empty `chunk_download.url_suffix`.

## Where to start

- `src/blobs/mod.rs`: `Sources` (around line 207), `from_manifests`,
  `from_pairs`, `url`, `tags`; call sites of `Sources::url` in
  `src/assemble/assembly.rs` and tests.
- `src/manifest/mod.rs`: `CapturedManifest::chunk_url_prefix` (around line
  351), `chunk_url` (around line 356), and the declared
  `chunk_download.url_suffix` field.
- `src/hyp/types.rs` around lines 595-613 documents the measured join order.

## Steps

1. Add a `chunk_url_suffix()` accessor next to `chunk_url_prefix()` on
   `CapturedManifest`, returning the declared suffix. Keep `chunk_url`
   consistent by using the same accessors.
2. Change `Sources` to store one value per tag that contains both base and
   suffix. A small private `SourceBase { prefix, suffix }` is clearer than two
   parallel `BTreeMap`s. Keep `bases` private.
3. `from_manifests` inserts both fields. `from_pairs` remains the test seam and
   inserts an empty suffix.
4. `Sources::url` returns `format!("{prefix}/{chunk_id}{suffix}")`.
5. Update `tags` and any iteration to the new value type.
6. Add a test with a non-empty suffix (for example `.zst`) and an empty-suffix
   control. If the P0-02 characterization test exists, replace its expectation
   with the suffixed URL.

## Tests

- Unit: `Sources::from_manifests` with a manifest whose `url_suffix` is
  `.zst` yields `{prefix}/{id}.zst`.
- Unit: empty suffix yields exactly the old URL.
- Unit: unknown tag still returns the existing typed error.
- If a URL is recorded in an artifact (patchmap `plan[].url`), assert the
  assembled plan uses the suffix as well.

## Acceptance

- A non-empty suffix can no longer be silently dropped.
- Empty-suffix behavior and all existing URLs are unchanged.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(blobs): retain the chunk URL suffix per source`
2. `test(blobs): cover suffixed and unsuffixed chunk URLs`

## Notes

- No network test is needed; this is a pure URL construction change.
- Do not reference the report or plan in code comments.
