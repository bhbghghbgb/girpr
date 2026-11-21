# T21 — Use `shared_bundle` for read-only capture tests; drop `fs2`

## Goal / pointers
- `docs/10-test-optimization-research.md` §F36 (lines 445–454) — full bundle captures
  are rebuilt per test process via `capture_ok` / `captured`, while `shared_bundle()`
  is the existing read-only alternative. Fix: switch read-only callers to
  `shared_bundle`; do not change spawn-based or flag-sensitive cases.
- `docs/10-test-optimization-research.md` §F38 (lines 464–469) — `fs2` is an unused
  dev-dep of `gircap`; actual use is in `gircap-test-support`. Fix: delete the manifest
  line; no dependency-tree change.
- Assumptions to validate: §A1 (46–52), §A5 (75–80), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (F37 is rejected; do not narrow
  the shared-bundle cache key).

## Scope
- `tools/capture/test-support/src/lib.rs` (`shared_bundle`, `OnceLock` cache,
  `capture_ok`, `captured`), the read-only callers named in §F36 (e.g.
  `tools/capture/tests/capture_layout.rs`), and `tools/capture/Cargo.toml` (`fs2`).
  Locate current callers before editing.

## Implementation plan
1. Validate §F36: classify each `capture_ok`/`captured` caller as read-only or
   spawn/flag-sensitive. Switch only read-only callers to `shared_bundle`.
2. Verify no test depends on a freshly generated bundle where a shared one would change
   an assertion; keep the pinned 32 KB blob and byte contracts unchanged (§A5, §6).
3. Validate §F38: confirm `fs2`'s only real use is in gircap-test-support, then remove
   the dev-dep line from `gircap` and verify the dependency tree is unchanged.
4. Update nextest filters if a named test moves (§A10). Deliver in two commits
   (bundle switch, dep removal).

## Constraints / validation
- Do not narrow the shared-bundle cache key (F37 is a considered and rejected change).
- Do not switch spawn-based or flag-sensitive cases to the shared bundle; one process
  per test under nextest (§A1) means a shared cache cannot replace per-case isolation
  where flags matter.
- No byte contract or pinned blob change (§6, §A5); no sleeps (§A6).

## Done when
Read-only callers use `shared_bundle`, sensitive callers are untouched, `fs2` is removed
from gircap with no dependency-tree change, and the capture suite passes with fewer
full bundle captures.
