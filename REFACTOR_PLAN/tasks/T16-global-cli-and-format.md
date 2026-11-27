# T16 — Global CLI args and shared `OutputFormat`

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Share the five global CLI flags, the `OutputFormat` value set and the
clap-error/exit shim across the three binaries. This task owns the shared
`OutputFormat`; the report planes that consume it are T18.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:72-86` — X1, three flag/format declarations.
- `docs/09-shared-code-refactor-findings.md:640-644` — SYNC-01, global args.
- `docs/09-shared-code-refactor-findings.md:646-650` — SYNC-02, `OutputFormat`.
- `docs/09-shared-code-refactor-findings.md:801-808` — CAP-11, parse/error shim.
- `docs/09-shared-code-refactor-findings.md:810-815` — CAP-12, format part only.

## Boundary / approach

Add a flattenable clap args struct in `girpr-cli` parameterized by project name,
`global` and the caller's format type. Move `OutputFormat` (value set, `as_str`,
`Display`, `FromStr`) into `girpr-report-core` behind an optional `clap` feature.
Add generic `parse_or_report_error` and `log_file_announcement` in `girpr-cli`.

## Guardrails

- The product's flags are not `global`; each program's `value_name`/default
  spelling differs.
- Capture parses the format by hand; its error wording stays local.
- `--help` bytes and exit classification are asserted in all three binaries.
- Capture flushes stdout before `process::exit`; girpr returns `ExitCode` so
  destructors run. Process boundaries stay local.
- Mechanism in `girpr-cli`/`girpr-exit` is already shared; only the shims move.

## Commit breakdown

1. Add `OutputFormat` to `girpr-report-core` behind optional `clap` + tests.
2. Add flattenable global args struct to `girpr-cli`.
3. Adopt in girsync.
4. Adopt in capture (keep hand-parsed error wording).
5. Adopt in girpr and replace the three `main` shims.

## Validation

- `--help` golden tests and exit-code classification tests for all three binaries.
- `cargo nextest` for cli, report-core and the three binaries.

## Done when

The flags and format have one declaration each, and help/exit bytes are
unchanged.
