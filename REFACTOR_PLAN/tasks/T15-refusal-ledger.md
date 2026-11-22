# T15 — `girpr-refusal-ledger` assessment (A4-forbidden, likely skip)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F20 (lines 356–365) — a bounded,
  deterministic refusal ledger exists only in gircap; candidate boundary is a leaf owning
  bounded order-independent retention and the JSON envelope. **High-risk** because it
  creates a new shared *observation format* across programs, which §A4 forbids, and the
  sampling/`omitted` count is byte-visible (§A5). Confidence medium.
- Assumptions to validate: §A4 (48–56), §A5 (57–67), §A14 (129–135).

## Scope
- gircap `tools/capture/src/rejections.rs` (`RejectionsDocument`, caps, smallest-rows
  retention, duplicate/omission counts) and rule IDs in `tools/capture/src/paths.rs`.
  No analogue in girpr/girsync. Locate before editing.

## Implementation plan
1. Validate §F20: confirm the ledger is capture-specific and that no girpr/girsync
   validator collects such a sidecar.
2. Assess only: would a shared leaf create a shared observation format across programs?
   If yes (as §A4 forbids) or if the `omitted` count/sampling is byte-visible (§A5), record
   a validated skip with the one-line reason.
3. Implement only on hard evidence of a safe, logic-free boundary that does not introduce
   a cross-program format. If done, move through §A14 seams.

## Constraints / validation
- §A4 forbids a shared report/observation format; §A5 pins the `omitted` count and JSON
  envelope. These are the expected blockers.
- Do not absorb the path-shape rule IDs (T14) into a shared format.
- Do not reopen N9 (capture artifact writer/wording stays local).

## Done when
A validated decision is recorded: either a safe logic-free leaf with byte-identical
output, or (expected) `skipped` with the §A4/§A5 reason.
