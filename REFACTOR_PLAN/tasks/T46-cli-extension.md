# T46 — Narrow CLI leaf extension (`OutputFormat` + `LogArgs`)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F51 (lines 664–672) — a narrow CLI leaf
  extension is plausible; a unified flag bundle is not. Candidate boundary is to add
  feature-gated `OutputFormat` and a `LogArgs` builder parameterized by project name and
  usage code to `girpr-cli`. Confidence medium for the narrow extension, low for a bundle.
- Assumptions to validate: §A7 (76–80), §A5 (57–67), §A10 (98–105), §A14 (129–135), §A11 (106–113).

## Scope
- Extend `crates/cli` (currently one clap-error mapper function). Sources: `OutputFormat`
  declared in all three program report planes, log flags declared three times; prerequisites
  `girpr-exit` and feature-gated `girpr-model/clap` already exist. Locate before editing.

## Implementation plan
1. Validate §F51: confirm the mechanical prerequisites and that `--version` presence differs.
2. Add feature-gated `OutputFormat` and a `LogArgs` builder parameterized by project name and
   usage code; keep exit codes and flag help text local (do not build a single bundle).
3. Migrate callers directly (§A2); move through §A14 seams.
4. Deliver in commits: leaf extension, migrations, seams.

## Constraints / validation
- High-risk (§F51): `--version` presence differs, flag help text is byte-identical and
   pinned, exit codes stay separate (§A7), and `.config/nextest.toml` pins the CLI-contract
   binary/paths (§A10). Do not change help text or codes; update filters if tests move.
- Feature unification (§A11) — keep the extension feature-gated.
- Do not build a unified flag bundle (§F51).

## Done when
The narrow extension exists with pinned help/exit bytes unchanged and all three programs
still have their own flag sets and exit codes. If a bundle is required, **stop** and record
`skipped`.
