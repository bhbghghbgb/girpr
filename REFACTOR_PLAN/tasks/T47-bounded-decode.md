# T47 — `bounded_decode` in `girpr-digest` assessment (likely re-confirm skip)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F52 (lines 674–682) — the
  bounded-decode/skip verdict has a new boundary but the same risk; candidate boundary is a
  caller-supplied-ceiling, caller-supplied-sink `bounded_decode` in `girpr-digest`.
  **Confidence low; likely re-confirms the prior skip.**
- Negative: §N7 (722–723) — the `zstd_measure`/`zstd_measure_from` pair stays duplicated for
  distinct error text. Assumptions: §A2 (29–36), §A5 (57–67), §A14 (129–135).

## Scope
- `crates/fetch/src/sophon/stream.rs` and `tools/capture/hash/src/` (the two clean-leaf
  shapes), plus `girpr-digest` as the proposed host. Locate before editing.

## Implementation plan
1. Validate §F52/§N7: confirm the two shapes are in clean leaves but that distinct error
   text is the documented reason for duplication.
2. Assess only: the streaming invariant is an always-on test that fails if a buffered decode
   writes the whole body at once; native `zstd-sys` rebuild propagation dominates cost
   (§A2). If a shared wrapper adds no value, record a validated skip with the one-line reason.
3. Implement only on hard evidence that the streaming invariant and distinct error text are
   preserved. Move through §A14 seams if done.

## Constraints / validation
- The always-on streaming test must still fail on a whole-body buffered decode (§F52).
- Distinct error text is a keeper (N7, §A9); do not unify messages.
- Do not correct gircap deviations (N10). No byte contract drift (§A5).

## Done when
A validated decision is recorded — expected `skipped` re-confirming N7, or a safe
`bounded_decode` with the streaming invariant intact.
