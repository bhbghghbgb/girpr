# T12 — CLI leaf (`OutputFormat`, `LogArgs`, clap-error mapper)

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F20 (lines 292–303) — five log
  flags, `--output`, `OutputFormat`, and clap-error → exit-code mapping declared
  three times.
- Assumptions to validate: §A9 (78–81), §A2 (21–27), §A1 (13–19).
- Related: T11 (`girpr-log` already owns the flag policy).

## Scope
- Candidate shared crate `crates/cli` (`girpr-cli`).
- Report names: `src/cli.rs`, `tools/sync/src/cli.rs`,
  `tools/capture/src/cli/mod.rs` (LogArgs, flag declarations),
  `src/report/record.rs`, `tools/sync/src/report/mod.rs`,
  `tools/capture/src/report/mod.rs`, `tools/sled2redb/src/lib.rs` (`OutputFormat`),
  `src/main.rs`, `tools/capture/src/cli/mod.rs`, `tools/sled2redb/src/main.rs`
  (clap-error mappers). Locate before editing.

## Implementation plan
1. Validate §F20: the log flags share help text, `value_name`, defaults and
   `girpr_log::parse_level`, differing only in global/flattened placement and
   project name; `--version` presence differs (capture only).
2. Create `girpr-cli` with: a shared `OutputFormat`; a clap-error → exit-code
   mapper parameterised by usage code (no shared exit-code table — §A9); and a
   `LogArgs` macro or builder that injects the project name and projects the
   five log flags into each tool's clap structure.
3. Migrate the three tools and the `sled2redb` `OutputFormat` user. Keep each
   tool's flag placement and `--version` policy local — do not impose one
   "standard flags" bundle that capture cannot adopt (§F20).
4. If gircap gains a new leaf dep, update §A2 doc tables.

## Constraints / validation
- Exit codes remain separate contracts (§A9); the mapper is parameterised by
  the tool's own usage code.
- Help text, defaults and flag names must stay byte-identical (§A3).
- Do not pull tool-specific flags or exit tables into the leaf.

## Done when
`OutputFormat`, the clap-error mapper and the log-flag declarations are sourced
from the shared leaf, each tool keeps its placement/`--version` policy, and all
CLI tests pass.
