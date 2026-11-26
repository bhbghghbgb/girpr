# T05 — CLI help snapshots → trycmd + strip-ansi

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §R3 (lines 172–184) — ~150 lines of
  hand-written clap `--help` layout parsing should become `trycmd` snapshots (or clap
  introspection for flag presence), not assertions on rendered text.
- §LEAF-2 (lines 432–439) — the hand-written ANSI stripper in test support becomes
  `strip-ansi-escapes::strip_str`.
- §X3 (lines 110–125) — the cross-cutting "test helpers parse other libraries' output"
  theme.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28), §A12 (48–50).
  Keep items: `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tests/cli_contract.rs` — `strip_ansi`, `HELP_*`, `option_body`, `help_entry` (`:66–205`),
  helpers at `:482`, `:514`, `:531`, and the help-layout tests at `:224–592`, `:751`,
  `:790`, `:852`, `:1013`.
- `src/cli.rs:579–593`; `crates/tempdir/src/test_support.rs:39–62`.
- New `.trycmd` / `.stdout` snapshot files; root and `crates/tempdir` manifests.
- Locate current occurrences before editing; the list above is a hint.

## Implementation plan
1. Validate that the help-layout tests assert flag presence/descriptions rather than
   product behavior. Decide between whole-output `.trycmd` snapshots and clap
   introspection (`Command::get_arguments()`, `Arg::get_long()`); use the latter where a
   snapshot would be brittle.
2. Remove the indentation/token parser and CRLF handling; add the chosen snapshot/introspection
   cases and regenerate the snapshots from the current CLI.
3. Replace the ANSI stripper body with `strip_ansi_escapes::strip_str(s)` — no signature
   change; downstream console-output assertions stay.
4. Keep the runtime tests (exit codes, JSON plane, log-file echo) as ordinary Rust tests.

## Constraints / validation
- Pinning whole help output is an acceptable change in what is asserted; CLI output
  itself need not change (§R3 lines 183–184).
- Downstream only needs plain text; removing more escape sequences than before is fine
  (§LEAF-2 lines 438–439). Do not touch the async process seam (T06).
- Adapt, do not delete, tests of intended flag behavior (§A5).

## Done when
No hand-written help-layout or ANSI-stripping parser remains, snapshots/introspection
cover the same flags, and the CLI and tempdir tests pass. If a premise is stale or a
keeper would change, **stop** and record `skipped`.
