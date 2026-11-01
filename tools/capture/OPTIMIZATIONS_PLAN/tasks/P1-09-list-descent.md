# P1-09 — `list` descends past non-bundles

Report: `../../OPTIMIZATIONS_REPORT.md` lines 248-262 (A11).

Dependencies: P0-03.

## Goal

When a directory has a `MANIFEST.json` that is not a bundle this build can read
(`ManifestProbe::NotABundle`), `verify::list` must keep descending. Today it
`continue`s and hides a nested bundle.

## Where to start

- `src/verify/list.rs`: `list_with` (around line 93), `child_dirs` (around line
  151), `ManifestProbe` usage; `src/verify/bundle.rs` (`probe_manifest`,
  `ManifestProbe::{Missing, NotABundle, Version}`).
- Tests in `src/verify/list.rs` and `src/verify/tests.rs`.

## Steps

1. In `list_with`, separate the two facts:
   - `Missing` -> descend (unchanged).
   - `NotABundle` -> descend; do not add an `Unreadable` row (the doc comment
     already distinguishes "not a bundle" from "a bundle this build
     refuses").
   - `Version` that `Bundle::open` rejects -> add `Unreadable` and stop
     descending (a real bundle boundary).
   - `Version` that opens -> add `Listed` and stop descending.
2. Keep the traversal stack/sort order and the final sort of `bundles` and
   `unreadable`.
3. Do not add a visited set here; symlink handling is P3-08. But do not
   accidentally turn a cycle into a deeper recursion while changing this.
4. Update the doc comment if it currently says any `MANIFEST.json` is a
   boundary.

## Tests

- Outer directory with an unparseable `MANIFEST.json`, child directory with a
  valid bundle: `list_with` reports the child.
- Outer directory with a `MANIFEST.json` missing `schema_version`: same.
- A valid bundle boundary still stops descent (nested valid bundle inside a
  valid bundle is not listed separately).
- A `Version` document that fails to open still produces `Unreadable` and does
  not descend.
- Flip the P0-03 characterization test.

## Acceptance

- Mixed/half-written roots produce a complete inventory.
- `list` may report more bundles; that is intended.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(verify): descend through directories whose manifest is not a bundle`
2. `test(verify): cover a nested bundle under a non-bundle directory`

## Notes

- Keep the injected reader seam; do not require a real filesystem for the
  control-flow tests if the existing harness allows otherwise.
- Do not reference the report or plan in code comments.
