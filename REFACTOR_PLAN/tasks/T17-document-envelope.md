# T17 — Deepen `girpr-document-envelope` from `girpr-report-core`

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F22 (lines 377–384) — schema-versioned
  ordered-JSON envelopes are repeated in five capture documents; candidate boundary is to
  deepen `girpr-report-core` into an envelope owning `{schema_version, tool, ordering,
  newline policy}` with per-program field lists. Confidence medium.
- Assumptions to validate: §A4 (48–56), §A5 (57–67), §A6 (68–75), §A14 (129–135), §N9 (729–732).

## Scope
- Deepen `crates/report-core` (the existing ordered-JSON writer). Sources: capture
  `tools/capture/src/lib.rs` (bundle manifest version gate), `emit/index.rs`,
  `emit/selfcheck.rs`, `pathmap.rs`, `rejections.rs`. Locate before editing.

## Implementation plan
1. Validate §F22: confirm the five envelope sites and that report-core is borrowed only as
   a writer today.
2. Extend the leaf with an envelope primitive owning version/tool/ordering/newline policy
   and per-program field lists; do not absorb published wording tables.
3. Migrate capture sites directly (§A2); move through §A14 seams (report-core already exists,
   so update its consumers/docs rather than adding a new crate where possible).
4. Commit leaf change, site migrations, seams.

## Constraints / validation
- High-risk (§F22): §A4 reserves report format and §A5 pins key sets; the published wording
  table in `emit/selfcheck.rs` must **not** be absorbed (N9). Selfcheck key sets must stay
  byte-identical.
- Do not enable `preserve_order` or merge report planes (§A6).
- Do not change schema version gate behavior (v3 additions, v2 refusal — §A5).

## Done when
Envelope mechanics are shared, all five documents keep the exact same version/tool/key
bytes, and the selfcheck wording table stays local. If format would be absorbed, **stop**
and record `skipped`.
