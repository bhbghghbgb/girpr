# T31 — Capture cosmetics and optional differ

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §CAP-8 (lines 752–761) — the generated
  README markdown table may use `tabled` with `Style::markdown()` (or `comfy-table`).
- §CAP-9 (763–772) — `GamePackages::staleness` may use `semver` on strict `x.y.z`
  fixture tags.
- §CAP-10 (774–781) — `selfcheck_slug` may use a local `as_str()` (or strum's
  `IntoStaticStr`) instead of a serde string round-trip.
- §CAP-11 (783–791) — the Python bundle-differ migration is **optional** and is not a
  dependency swap.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A5 (26–28), §A12 (48–50).
  Keep items: `guidance/DO-NOT-TOUCH.md`, especially the permissive versions at lines
  822–823 and the bundle-diff pins.

## Scope
- `tools/capture/src/emit/index.rs:2058–2157` (`Manifest::readme`), especially `:2094–2118`;
  `tests/capture_layout.rs:996–1030`; `tests/bundle_diff.rs`.
- `tools/capture/src/hyp/types.rs:416–457`; `src/hyp/mod.rs:897–898`, `types.rs:1106–1136`.
- `tools/capture/src/run/mod.rs:957–963`; summary pins in `tests/matrix.rs`,
  `tests/cli_contract.rs`.
- `tools/capture/tools/bundle_differ.py:28–53`, `:75–149`, `:157–229`; `tests/bundle_diff.rs`.

## Implementation plan
1. Treat each item as independently skippable. Validate the fixture tags before using
   `semver`: report lines 771–772 say refusing `7` or prerelease suffixes is acceptable
   **only if** the fixtures are strict `x.y.z` — verify, otherwise skip CAP-9. This task
   does not touch dispatcher's permissive version spellings (non-finding, lines 822–823).
2. If CAP-8 is chosen, build the matrix table with explicit labels and keep values
   quoted from `self`/`selfcheck`; regenerate the README skeleton pins on padding changes.
3. If CAP-10 is chosen, use a local `const fn as_str()` matching `State`/`Kind`/`Class`/
   `Encoding`, or strum's `IntoStaticStr`; keep the `result=ok` summary pins unchanged.
4. CAP-11 is a deliberate Python→Rust rewrite, not a library swap. Implement it only if
   the batching review endorses it; keep `DERIVED` exemptions and `--strict-exemptions`,
   and preserve `tests/bundle_diff.rs` behavior.
5. Record each item's done/skipped outcome separately in the task note.

## Constraints / validation
- All items are cosmetic or optional; do not expand them into broader formatting or
  toolchain changes. Skipping low-payoff items is a legitimate outcome (§A1).
- Semantic assertions stay; only spelling/padding pins may be regenerated (§A5).

## Done when
Each chosen sub-item uses its proposed mechanism with pins regenerated and tests passing;
skipped sub-items are recorded with a one-line reason. If any keeper would change,
**stop** for that sub-item and leave it untouched.
