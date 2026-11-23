# T16 — Move scope-typo refusal earlier and surface purge diagnostics

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §6 **COR-06** (lines 474–485) — `build_and_assert_plan`
  walks the whole install to fold `PurgeKeys` before `check_scope_selection`; when no
  purge flag is set, the "--include matched nothing" error is decidable immediately
  after `build_plan`. Evaluate the early-exit case right after `build_plan` when
  `!purge_wanted`; keep the full check after the walk only for purge-enabled runs
  where a purge candidate can rescue the selection.
- §6 **COR-07** (lines 486–495) — unreadable purge candidates are counted as 0 bytes
  and never surfaced; surface `purge_errors=N` (and any hidden-byte count) in
  `SUMMARY` whenever non-zero, independent of ceilings; consider strict refusal.
- §6 **COR-09** (lines 505–512) — the legacy `girpr.lock` keep-rule is root-only but
  undocumented; add a test pinning root-only matching and say so in `keep_set()` docs,
  or widen the rule if subdirectory locks are possible.
- Keep items: best-effort selection semantics, `SUMMARY` machine-readable shape,
  §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/mod.rs` (~385–398, 473–506), `app/repair-plan/src/lib.rs`
  (`check_scope_selection` ~100–124).
- `src/repair/purge.rs` (unreadable candidates ~144–155, 205–210; `keep_set()`
  ~348–360, 415–444), `src/repair/scope/limits.rs` (~121–133, 150–156).
- Locate current code before editing; the list above is a hint.

## Implementation plan
1. Confirm the walk-before-check ordering and the purge_error bookkeeping in the
   current tree. If a purge candidate can genuinely rescue a selection even without a
   purge flag, do not move the check early.
2. Add the early scope refusal immediately after `build_plan` for `!purge_wanted`
   runs, retaining the full post-walk check for purge-enabled runs (COR-06).
3. Surface `purge_errors=N` (and hidden bytes, if tracked) in `SUMMARY` whenever
   non-zero, without changing the machine-readable shape; consider strict refusal as
   a follow-up rather than a silent change (COR-07).
4. Pin the root-only lock keep-rule with a test and document it in `keep_set()` (or
   widen only if subdirectory locks are real) (COR-09).
5. Deliver in several commits if useful: early exit, diagnostics, keep-rule test.

## Constraints / validation
- A purge candidate must still be able to rescue a selection on purge-enabled runs.
- `SUMMARY` is a data-plane record: add the field without breaking its shape or other
  records (see `guidance/DO-NOT-TOUCH.md`, lines 527–530). This is the task's sole
  exception to "data plane unchanged".
- Do not change any §12 non-finding.

## Done when
Scope typo refusal is early when safe, purge errors are visible in `SUMMARY`, the
lock keep-rule is pinned/documented, and tests pass. If the early-exit premise is
false or the `SUMMARY` shape cannot be preserved, **stop** and record `skipped` with a
one-line reason.
