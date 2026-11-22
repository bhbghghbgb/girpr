# T39 — Umbrella `test-support` facade assessment (generalization trap, likely skip)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F44 (lines 599–605) — test-support
  feature flags are a generalization trap; candidate boundary is one umbrella `test-support`
  facade re-exporting every surface. **High-risk**: feature unification can make tokio
  timers/runtime and clap compile into consumers that currently avoid them. Confidence high.
- Assumptions to validate: §A11 (106–113), §A2 (29–36), §A14 (129–135).

## Scope
- Test surfaces in `crates/log/Cargo.toml`, `util-hash`, `util-path`, `fetch`,
  `tools/capture/hash/Cargo.toml`, `crates/report/Cargo.toml` (tokio `test-util`). Locate before editing.

## Implementation plan
1. Validate §F44: confirm the six test surfaces and their feature gates.
2. Assess only: an umbrella facade unifies features across the graph (§A11) and makes
   async/clap compile into consumers that avoid them. If so, record a validated skip with
   the one-line reason.
3. Implement only on hard evidence that no consumer gains tokio/clap features; measure
   compiled variants, not just source duplication. Move through §A14 seams if done.

## Constraints / validation
- Feature unification (§A11) can increase compiled variants; do not assume a consolidated
   surface is cheaper (§A2). Keep `log`'s `test-support`/`test-support-async` split intact.
- Do not absorb the other test-support tasks (T34–T38, T40).

## Done when
A validated decision is recorded — expected `skipped` with the §A11 reason.
