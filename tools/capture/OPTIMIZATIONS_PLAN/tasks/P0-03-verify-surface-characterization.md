# P0-03 — Verify-surface characterization tests

Report: `../../OPTIMIZATIONS_REPORT.md` lines 833-850 (Phase 0 list), plus the
finding sections named below. This task changes tests only.

Dependencies: none.

## Goal

Pin today's behavior for the verify-surface cases later tasks will change:

1. a patchmap `plan[]` that is self-consistent but disagrees with
   `chunks/index.json` (A10, lines 229-246);
2. a non-bundle directory containing a nested bundle (A11, lines 248-262);
3. (optional, if the existing fixtures make it cheap) a bundle with a current
   `MANIFEST.json` and an old-schema or malformed secondary document, and
   `patchmap.expectations.result_path` pointing outside the bundle (A9, lines
   207-227).

Characterization only. For case 1, assert that `check_patchmap_arithmetic`
currently passes a plan that the index cannot produce. For case 2, assert that
`list_with` currently hides the nested bundle. Do not fix either.

## Where to start

- `src/verify/tests.rs` (`check_patchmap_arithmetic`, `arithmetic_map`),
  `src/verify/checks.rs`, and `src/verify/materialize.rs` (`rebuild`,
  `rebuild_all`). Build the smallest `PatchMap` + `ChunkIndex` pair.
- `src/verify/list.rs` (`list_with`) tests; the manifest reader is injected, so
  the nested case can be built with an in-memory tree or a temp directory.
- `src/verify/bundle.rs` (`Bundle::open`, `validate_document_paths`,
  `schema_version_of`) if testing case 3.

## Steps

1. For the plan/index case, make the plan's totals, expectations, and
   `must_not_fetch` all internally consistent, then change a chunk id, target
   range, source range, or URL so the index's references describe a different
   patch. Assert the rows that currently pass.
2. For the nested case, make the outer directory contain a `MANIFEST.json` that
   `probe_manifest` reports as `NotABundle` (unparseable or no version) and a
   child directory that is a valid bundle. Assert what `list_with` returns
   today.
3. For case 3, cover one secondary document version and one `result_path`
   escape; assert the current accept/refuse result. Skip this sub-case if it
   requires a large fixture.
4. Keep fixtures synthetic and deterministic; do not read `artifacts/`.

## Tests and acceptance

- Cases 1 and 2 have tests that fail if today's accept/hide behavior changes.
- `cargo test -p gircap` passes.
- No production code changes and no changes to `STATE.md`.

## Commit plan

1. `test(verify): characterize plan/index disagreement`
2. `test(verify): characterize list descent and secondary document validation`

## Notes

- The later tasks will invert these expectations; keep the assertions narrow so
  the inversion is a small diff.
- Do not reference the report or plan in test names/comments.
