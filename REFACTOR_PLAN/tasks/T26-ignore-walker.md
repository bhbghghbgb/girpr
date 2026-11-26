# T26 — Walk + include/exclude → ignore

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §SYNC-9 (lines 636–649) — the bespoke
  per-directory `ExcludeMemo` over a raw `walkdir` iterator may become `ignore`'s
  `WalkBuilder` + `OverrideBuilder`.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A4 (22–23). This is a
  **medium/low-confidence** finding (§SYNC-9 line 648); keep items:
  `guidance/DO-NOT-TOUCH.md` (the filter/scan non-findings and T20's globset work).

## Scope
- `tools/sync/src/scan.rs:115–251`; `tools/sync/leaves/src/filter.rs:46–188` (`Filter`,
  `ExcludeMemo`); `Cargo.toml:51`, `:65`.
- `leaves/src/filter.rs:190–429`, `scan.rs:301–399` and filter-driven integration suites.
- `girpr_filter::any_matches_or_ancestor` usage from T20; coordinate if T20 changed it.

## Implementation plan
1. **Validation gate:** build a behavior-diff harness over the pinned filter tables
   before changing the consumer. If `ignore`'s globset semantics cannot reproduce the
   include-on-path-only and "excluded directory covers subtree" rules, **stop** — the
   report says adopt only after agreement (§SYNC-9 lines 648–649).
2. If the gate passes, configure `WalkBuilder` with hidden/gitignore/parent handling
   disabled and translate include/exclude into overrides. Keep cache pruning,
   `link::try_is_link` classification and heartbeat as post-filters.
3. Remove `ExcludeMemo` and the `any_matches_or_ancestor` edge from this consumer if no
   other caller remains; otherwise leave the shared leaf intact for T20.
4. Re-baseline only the table rows the verified diff shows changed. `ignore 0.4.35` is
   not currently locked; verify a resolved build.

## Constraints / validation
- `ignore` uses globset semantics, not `glob::Pattern` (§SYNC-9 line 647); the gate must
   prove agreement with the pinned tables before any consumer change.
- Do not change cache pruning, link classification or heartbeat behavior.
- A validated skip/revert is the expected outcome if the diff cannot be made clean.

## Done when
The behavior-diff harness demonstrates agreement, the walker uses `ignore`, and the
filter/scan suites pass with justified re-baselining. If the gate fails, record `skipped`
with a one-line reason.
