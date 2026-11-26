# T23 — Link-target normalization → path-clean

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §SYNC-5 (lines 589–599) — the push/pop
  `Component` walker that cleans junction/symlink target text becomes `path-clean`
  (`Path::clean()`), keeping lexical (not filesystem) normalization.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A4 (22–23). Keep items:
  `guidance/DO-NOT-TOUCH.md` — the "only redirecting reparse points" policy at report
  line 840–841 and the fail-closed prefix rule at 818–819.

## Scope
- `tools/sync/src/commands/sync/plan/link.rs:143–165` (`absolute_alias`),
  `:173–188` (`alias_target`); `leaves/src/link.rs:157–172`;
  `src/commands/sync/plan/safety.rs:728–756`.
- Tests via `tests/sync_plan/table.rs`, `tests/sync_plan/plan_print.rs`,
  `tests/why/table.rs`; add direct `..`-escape cases. Affected manifests.

## Implementation plan
1. Validate that the current walker is lexical only and that `try_is_link` /
  `on_disk_name` decisions are independent of it.
2. Apply `Path::clean()` after stripping the canonical prefix / building the candidate
   at each named site; keep the refusal of paths escaping the root and every downstream
   link/safety decision.
3. Add direct `..`-escape cases to the plan/safety tables; re-baseline only the affected
   table rows if normalization spelling changes.
4. Verify the sync plan suites pass on both platforms. `path-clean 1.0.1` is not
   currently locked; verify a resolved build.

## Constraints / validation
- Lexical normalization must stay — do not canonicalize against the filesystem
  (§SYNC-5 line 599).
- The safety property (refusing escaped paths) must survive unchanged; if `clean()`
   cannot preserve it, stop.

## Done when
All three hand-written component walkers use `Path::clean()`, escape cases are covered,
and the plan/why suites pass. If a premise is stale or a keeper would change, **stop**
and record `skipped`.
