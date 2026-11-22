# T43 — `girpr-primitives` consolidation assessment (rebuild-domain trap, likely skip)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F48 (lines 634–642) — several
  dependency-free leaves are fragments whose merge is a rebuild-domain trap; candidate
  boundary is a `girpr-primitives` consolidation. **High-risk**: the value of these leaves is
  the rebuild domain (§A2), not code volume. Confidence medium.
- Assumptions to validate: §A2 (29–36), §A14 (129–135), §N10 (734–737), §N11 (739–744).

## Scope
- `girpr-cli` (one function), `girpr-exit` (one macro), `girpr-util-counter` (one function),
  `girpr-audio-language` (no deps), `girpr-loopback-mock` (must never gain deps). Locate before editing.

## Implementation plan
1. Validate §F48: confirm the leaves and the rebuild-domain rationale; confirm
   `girpr-loopback-mock` declares it must stay dependency-free.
2. Assess only: merging loopback-mock with cli pulls clap into every consumer; merging
   exit/audio-language couples unrelated artifact-byte contracts. If net-negative, record a
   validated skip with the one-line reason.
3. Implement only on hard evidence of a rebuild-domain win (fewer compiled variants after
   an edit), not just fewer crates. Move through §A14 seams if done.

## Constraints / validation
- The value is the rebuild domain (§A2), not code volume; do not merge leaves if it widens
   rebuild fan-out.
- Do not correct gircap deviations (N10) or re-propose extractions with no remaining
   duplication (N11).

## Done when
A validated decision is recorded — expected `skipped` citing the rebuild-domain rationale.
