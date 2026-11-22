# T19 — `girpr-value-ranges` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F24 (lines 395–401) — range-like CLI
  arguments are validated three ways with three error semantics; candidate boundary is a
  leaf of pure predicates returning facts, with each program mapping a fact to its own
  error type and exit class. Confidence high on duplication, medium that it is worth coupling.
- Assumptions to validate: §A5 (57–67), §A7 (76–80), §A2 (29–36), §A14 (129–135), §A9 (88–97).

## Scope
- New leaf (proposed `girpr-value-ranges`). Sources: `tools/capture/src/config/mod.rs`
  (transport values → `UsageError` exit 1), `src/cli.rs` (`MAX_JOBS`, `bail!`),
  `tools/sync/src/config/common.rs` (`nz` closure → exit 3). Locate before editing.

## Implementation plan
1. Validate §F24: confirm the three validation sites and their three error/exit semantics.
2. Define pure predicates returning facts (in-range / out-of-range with the offending
   value); keep each program's error type, message and exit class local.
3. Migrate all three callers directly (§A2); move through §A14 seams where relevant.
4. Commit leaf, migrations, seams.

## Constraints / validation
- High-risk (§F24): error strings and exit codes are published contracts (§A5, §A7) —
  the product's `bail!`, capture's exit 1 and girsync's deliberate exit 3 must not change.
- Divergent error semantics are keepers (§A9); if a single error type seems desirable,
  stop and raise a follow-up.
- Do not absorb exit-code values themselves (§A7, N8) — only the pure predicates.

## Done when
Range predicates are shared as pure facts, all three programs keep their own error
strings and exit classes, and no published message/code changed.
