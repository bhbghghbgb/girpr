# T12 — `girpr-decision-record` transport-free primitive (low confidence)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F17 (lines 325–332) — both programs
  keep a parallel machine-readable decision trace with incompatible transports; candidate
  boundary is a transport-free primitive for "record a typed decision, drain it", each
  program owning its fields. **Confidence low.**
- Assumptions to validate: §A5 (57–67), §A10 (98–105), §A14 (129–135), §N14 (754–756).

## Scope
- New leaf (proposed `girpr-decision-record`). Sources: product `src/repair/diagnostics.rs`
  (thread-local `RepairDiagnostics`, written never read) and gircap
  `tools/capture/src/emit/selfcheck.rs` (one `Assertion` row per instance into bundle bytes).
  Locate before editing.

## Implementation plan
1. Validate §F17: confirm one transport is thread-local + current-thread runtime and the
   other is a **published artifact** (§A5); confirm the field sets are intentionally local
   (N14).
2. Only if a genuinely transport-free core exists, define a minimal "record a typed
   decision, drain it" primitive with program-owned field types.
3. Migrate callers directly (§A2); move through §A14 seams if gircap calls it.
4. Given low confidence and that one transport is byte-published, the expected outcome is a
   validated skip — do not force a shared surface that risks the artifact bytes.

## Constraints / validation
- One transport is thread-local and depends on nextest one-process-per-test semantics
  (§A10); the other is a published artifact (§A5). Do not merge transports.
- Product field set and capture matrix/budget/require-class semantics stay local (N14).
- No byte or key-order drift in selfcheck output (§A5).

## Done when
A safe transport-free core is shared without touching published bytes — or a validated
skip is recorded with the one-line reason (expected default).
