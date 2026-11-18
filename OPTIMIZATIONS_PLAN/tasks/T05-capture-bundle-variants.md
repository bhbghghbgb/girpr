# T05 — Capture: reduce full captures 3→1 (Ceiling + GzipThreshold variants)

Priority P0 · `tools/capture` (gircap) · checkpoint CP2 · report findings B1, B5

## Pointers (give these to the implementing session)

* Report finding B1: `docs/09-test-scale-reduction-research.md:L211–L234`.
* Report finding B5: `docs/09-test-scale-reduction-research.md:L302–L316`.
* Report sequencing note: `docs/09-test-scale-reduction-research.md:L821–L822`.
* Report assumptions (B5 likely to need a smoke fallback):
  `docs/09-test-scale-reduction-research.md:L843–L851`.
* This plan: `OPTIMIZATIONS_PLAN/tasks/T05-capture-bundle-variants.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

The dominant scale cost in `tools/capture` is the shared bundle: each
`BundleVariant` means a separate mock + spawned `gircap capture` process + full
on-disk bundle tree. Drop two of the three variants, leaving one capture for the
whole suite.

1. **B1 — delete the `Ceiling` variant.** `BundleVariant::Ceiling` (common/mod.rs
   :64–65, key `"ceiling123456"`, flag `--max-decompressed-bytes 123456`) has
   exactly one consumer, `the_ceiling_variant_records_the_ceiling_it_used`
   (`tests/schema.rs:298–303`), asserting only
   `MANIFEST.json["max_decompressed_bytes"] == 123456`. Delete the variant and
   test; add a unit test in `src/emit/index.rs` beside
   `the_manifest_is_whole_when_build_manifest_returns_it` (~:1463) using
   `Fixture::input()` (:1428): set `max_decompressed_bytes = 123456`, assert
   `build_manifest(...)` carries it (`build_manifest` :72 copies the field at
   :158; flag→plan is pinned at `src/config/tests.rs:153–154`).
2. **B5 — downshift `the_gzip_variant_records_its_encoding_decisions`**
   (`tests/schema.rs:450–~510`). `hash::should_gzip` is already exhaustively
   tested at `schema.rs:424–442`. Replace the full-capture test with a small disk
   test: two files (one below, one above 1000 B) through
   `emit::index::digest_file(path, encoding)` (:226) plus
   `should_gzip(len, threshold)`, asserting one `gzip` and one `identity`. Delete
   `BundleVariant::GzipThreshold` (common/mod.rs :64, :72, :81).

## Implementation plan

Code anchors (validate before editing): `tests/schema.rs:298–303`, `:424–442`,
`:450–~510`; `tests/common/mod.rs:58–85,111–169` (and the variant arms at
:64–65/:72/:81); `src/emit/index.rs:72, :226, :1428, :158, ~:1463`;
`src/config/tests.rs:153–154`.

Possible commits: one or two commits, but **do B1 and B5 in the same change** —
`bundle_source_key()` hashes all of `src/**`, so splitting them forces repeated
one-time recaptures.

If `digest_file` is not the path `emit::tree::write` uses (report's stated
uncertainty), keep a thin e2e smoke for B5 rather than dropping the property.
Alternatively, if "a non-default value survives an actual process" must stay,
fold the field check from B1 into `the_gzip_variant_records_its_encoding_decisions`
by giving that variant the flag too.

## Acceptance criteria

* Only one `BundleVariant` and one full capture remain for the whole suite.
* The `max_decompressed_bytes` field-copy and `should_gzip` boundary decision
  remain pinned by unit tests; the document-level fact on the default bundle
  remains.
* The remaining bundle/schema/offline-bundle suites (report §B non-findings,
  L332–L340) are untouched, including the 3 ignored `bundle_shapes` cases.
* Affected suites pass.

## Guardrails

* Verify the unit tests really reach the production path before deleting the
  e2e assertions; fall back to a thin smoke if not. B5 is the least certain of
  this group.
* Test-only change (plus unit tests). Report follow-up ideas without implementing
  them.
