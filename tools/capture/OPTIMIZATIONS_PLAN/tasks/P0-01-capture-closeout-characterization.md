# P0-01 — Capture/close-out characterization tests

Report: `../../OPTIMIZATIONS_REPORT.md` lines 833-850 (Phase 0 list), plus the
finding sections named below for the exact case. This task changes tests only.

Dependencies: none.

## Goal

Pin today's observable behavior for six cases that later tasks will change:

1. a captured file whose name ends in `.gz` but is not gzip (A2, lines 76-95);
2. a resolved pair where `main.tag == diff_tags[0]` (A1, lines 56-74);
3. a stray pre-existing file under `game/` (A6, lines 150-166);
4. a patchmap side `size` that disagrees with the chunk sum (A7, lines 168-187);
5. a second-pass checksum change after A16 (A8, lines 189-205);
6. duplicate manifest paths in the `paths.tsv` sidecar (A4, lines 115-131).

The tests are characterization tests: assert what the code does **today**,
including a failure or exit code where today's behavior is the bug. Do not fix
the behavior here. Name them clearly (e.g. `currently_...` or
`characterizes_...`) so the later task can replace the expectation deliberately.

## Where to start

- `tests/capture_layout.rs`, `tests/schema.rs`, `tests/common/mod.rs` for
  end-to-end capture/close-out fixtures.
- `src/config/tests.rs` for `VersionPair::select` using a hand-built
  `BranchParams` with `tag == diff_tags[0]`.
- `src/manifest/dump.rs` unit tests for `paths_tsv` with duplicate entries.
- `src/emit/closeout.rs` and `src/emit/index.rs` tests for the close-out and
  A16 cases; use the smallest existing fixture rather than a real bundle.
- `src/verify/materialize.rs` / `src/verify/checks.rs` tests for a side-size
  disagreement, if that is cheaper than a full bundle.

Prefer unit tests where the seam already exists. Use an integration test only
when the claim is about the bundle on disk or an exit code.

## Steps

1. For each case, find or build the smallest fixture that reaches it. Record
   the fixture shape in a short test comment.
2. Assert the current output: exit code, the specific failing assertion id, the
   digest/row that is written, or the exact error text fragment. Avoid
   asserting whole documents; assert the field under test.
3. For the `.gz` case, cover both a non-gzip `*.gz` file and (if cheap) a real
   gzip stream stored under a captured path, so the later encoding fix has both
   ends pinned.
4. For the second-pass case, assert the ordering that makes the hole possible
   (the published `checksums.json` is produced by the reuse path after A16) or
   inject a controlled rewrite if the existing seams allow it. If no seam
   exists, add a focused unit test around `write_checksums_reusing` rather than
   inventing a production hook.
5. Make every test deterministic and independent of the real `artifacts/`
   directory.

## Tests and acceptance

- All six cases have a test that fails if the current behavior changes.
- `cargo test -p gircap` (or the repository's standard crate-test command)
  passes; no test depends on network access or a pre-built bundle.
- No production code changes and no changes to `STATE.md`.

## Commit plan

1. `test(capture): characterize .gz detection, same resolved tag, and stray game files`
2. `test(capture): characterize side-size, second-pass checksum, and duplicate-path sidecar`

## Notes

- Do not encode the finding numbers in test names or comments in a way that
  will be meaningless after the report is deleted. Describe the behavior.
- If a case cannot be reached without production changes, say so in the return
  instead of adding a test hook.
