# T17 — gircap: small production leaves

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F25 (lines 310–320) — production
  leaves: `encoding.rs` 54, `paths.rs` 507, `pathmap.rs` 156, `rejections.rs`
  331, `hash/*` ~1,146, `exit/mod.rs` 144, `logging/mod.rs` 249, `report/*`
  ~1,300, `tasks.rs` 223.
- Assumption: §F37 (424–430) warns that several shared leaves are already
  tiny/zero-dep and not worth further splitting — validate before adding new
  crates.
- Assumptions to validate: §A2 (27–33), §A5 (48–53), §A4 (40–46), §A6 (55–61).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/src/{encoding,paths,pathmap,rejections,tasks}.rs`,
  `tools/capture/src/hash/**`, `exit/mod.rs`, `logging/mod.rs`, `report/**`.
- Existing shared leaves `girpr-exit`, `girpr-log`, `girpr-report-core`,
  `girpr-digest` (§A3) — extend/reuse rather than duplicate (§F37).
- Locate each file and its `use crate::` edges in the current tree before editing.

## Implementation plan
1. Validate which candidates are truly acyclic leaves and which are already
   covered by an existing shared crate (§F37). A file that is only re-exported
   by the facade is not a win (§A2) — skip it with a reason.
2. Group the validated leaves into logic-free crates (or extend existing ones)
   and make gircap depend on them directly.
3. Keep exit tables, report formats and replay/status helpers local where §A13
   deems them deliberate keepers; move mechanics, not policy.
4. Honor §A5 and update §A5/§F39 doc tables in the same work; update nextest
   filters (§F31) if tests move.

## Constraints / validation
- §A2 payoff must be real; §F37 leaves where splitting won't help are excluded.
- No byte/behaviour change — exit codes, log format, report bytes stay identical
   (§A6).
- No girpr dependency or binary invocation (§A5).

## Done when
The validated leaves are logic-free crates consumed directly by gircap (or
folded into existing shared leaves), deliberate keepers stay local, and all
associated tests pass with unchanged bytes.
