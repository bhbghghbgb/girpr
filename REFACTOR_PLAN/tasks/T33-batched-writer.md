# T33 — Generic batched durable writer

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F38 (lines 537–543) — durability/commit
  policy is already parameterized inside one crate; candidate boundary is a generic batched
  durable writer. **High-risk** because it would expose test knobs to consumers and change
  crash-safety semantics. Confidence medium-low.
- Assumptions to validate: §A11 (106–113), §A13 (122–128), §A2 (29–36), §A14 (129–135).

## Scope
- `crates/sync-cache/src/write.rs` (`CommitLimits` batching by ops/bytes/time, resolved
  `CommitPolicy`, test-knob thresholds), `sync-cache/src/lib.rs` (`write_durability()`),
  `tools/sync/src/commands/sync/mod.rs` (pre-drop commit ordering). Locate before editing.

## Implementation plan
1. Validate §F38: confirm the batching/durability is already parameterized within
   `girpr-sync-cache` and pre-drop commit ordering is a crash-safety invariant.
2. Assess only: a generic writer would expose test knobs to consumers and could change
   crash-safety semantics — if so, record a validated skip with the one-line reason.
3. Implement only on hard evidence that knobs stay test-local and ordering is preserved.
   Move through §A14 seams if done.

## Constraints / validation
- High-risk (§F38): pre-drop commit ordering is a crash-safety invariant (§A13); feature
   unification can add compiled variants (§A11). Do not expose test knobs to consumers.
- Do not reopen closed cache decisions (§N15).

## Done when
A validated decision is recorded — expected `skipped` with the crash-safety/knob reason, or
a safe generic writer preserving ordering.
