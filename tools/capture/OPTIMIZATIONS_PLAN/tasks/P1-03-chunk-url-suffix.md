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
  357), `chunk_url` (around line 366), and the declared
  `chunk_download.url_suffix` field (on `Download`).
- `src/hyp/types.rs`: `BuildManifest::archive_url` / `BuildManifest::chunk_url`
  (around lines 601-608) and the shared `pub(crate) fn join(&Download, &str)`
  (around line 621). `join` appends a non-empty `url_suffix` and is now the one
  definition of the `{prefix}/{id}{suffix}` join.

## Steps

1. Expose the declared chunk `Download` on `CapturedManifest` (for example
   `chunk_download() -> &Download`, or add `chunk_url_suffix()` next to
   `chunk_url_prefix()`). `chunk_url` already calls `join`; keep it that way.
2. Change `Sources` to store one value per tag that carries both prefix and
   suffix (a private `SourceBase { prefix, suffix }`, or a stored `Download`).
   A small struct is clearer than two parallel `BTreeMap`s. Keep `bases` private.
3. `from_manifests` inserts both fields from the declared `chunk_download`.
   `from_pairs` remains the test seam and inserts an empty suffix.
4. `Sources::url` builds the URL through `crate::hyp::types::join(&Download,
   chunk_id)` rather than a new `format!` spelling, so there is still exactly
   one definition of the join. Pass the stored prefix/suffix as a `Download`
   (or extend the helper to take prefix and suffix directly if that is smaller).
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
- The merge that followed this plan extracted the shared `join` helper described
  above and clarified its comment: appending a non-empty suffix is a deliberate
  conservative choice (the reference Sophon clients ignore the field), not a
  measured join order. That does not change the finding — `Sources::url` still
  drops a suffix that every other URL builder appends — but it means the fix must
  reuse `join` rather than introduce a second spelling.
- Do not reference the report or plan in code comments.
