# T03 — Run the three independent plan-phase network fetches concurrently

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §2 **S-04** (lines 158–172) — `load_manifests_and_reuse`
  awaits `manifest::fetch_latest` then `manifest::local_reuse_map`, and
  `fetch_deprecated` is issued later; none depends on another's output. Wall-clock is
  the sum of three network phases instead of roughly their max.
- Change suggested in the report: start all three with `tokio::join!`/`try_join!` in
  `plan_phase`, sharing one client and one combined concurrency budget (total 4–8 in
  flight) rather than two independent caps of 4.
- **Assumption** at report lines 170–171 — `deprecated_files` is safe to fetch before
  the local-reuse map is built (currently best-effort and independent).
- Keep items: best-effort deprecated fetch, plan output shape, §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/mod.rs` (`load_manifests_and_reuse` ~209–212, ~305–332, ~400,
  `build_and_assert_plan` ~1172–1173).
- `src/repair/phases/manifest.rs` (`fetch_latest`, `local_reuse_map`,
  `MAX_MANIFEST_CONCURRENCY = 4` ~42).
- `app/fetch` client construction as needed. Locate current call sites before editing.

## Implementation plan
1. Validate the assumption in the current tree: `deprecated_files` must be
   independent of the reuse map and safe to fetch early. If not, stop or narrow the
   overlap.
2. Start the three fetches together with `try_join!`/`join!`, sharing one client and a
   combined concurrency budget (4–8 total in flight).
3. Preserve each phase's error handling and best-effort semantics; do not turn a
   best-effort failure into a fatal one.
4. Deliver in several commits if useful: budget plumbing, join, error handling tests.

## Constraints / validation
- Plan/report/progress output must be unchanged (lines 527–530).
- Keep `deprecated_files` best-effort (lines 170–171, 630–636).
- Do not change tuned profiles or any §12 non-finding.

## Done when
The three network phases overlap under a shared client/budget, error and best-effort
semantics are preserved, and plan tests pass. If the early-deprecated assumption is
false, **stop** and record `skipped` with a one-line reason.
