# T20 — Shared blob→bytes→digest verify walker

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F25 (lines 403–410) — offline
  verification walkers duplicate a "re-derive from recorded bytes" discipline; candidate
  boundary is a shared memoized blob→bytes→digest walker with a recorded-versus-re-derived
  comparison shape. Confidence medium.
- Assumptions to validate: §A6 (68–75), §A13 (122–128), §A5 (57–67), §A2 (29–36), §A14 (129–135).

## Scope
- New leaf (proposed `girpr-verify-walker`). Sources: capture `tools/capture/src/verify/checks.rs`
  and `verify/materialize.rs`; product check phase plus `src/repair/scan.rs` and
  `phases/limits.rs`. Locate before editing.

## Implementation plan
1. Validate §F25: confirm both sides rebuild inputs and that findings are split between
   re-derived and unrecorded (published).
2. Define a memoized walker over blob→bytes→digest with a comparison shape that keeps
   "recorded" vs "re-derived" distinct; leave structural-property rules to callers.
3. Migrate callers directly (§A2); move through §A14 seams if gircap calls it.
4. Commit walker, callers, seams.

## Constraints / validation
- High-risk (§F25): §A6 structural properties are re-derived against `chunks/index.json`,
  not manifests; verify reports coverage as findings rather than failures (§A13). Do not
  turn findings into failures or reuse an observation buffer.
- Recorded-vs-unrecorded split is published (§A5); wording and counts must stay identical.
- Do not reopen N15 (cache codec/schema/publisher closed).

## Done when
The walker is shared with the recorded/re-derived distinction intact, all verify output
bytes are unchanged, and doc seams are updated. If verification would become tautological
(§A13), **stop** and record `skipped`.
