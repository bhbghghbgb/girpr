# T18 — Report-core records and emit sink

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Move the generic ordered record mechanic and the "render → mirror to log →
write one line" sink into `girpr-report-core`, and adopt it in the three report
planes. `OutputFormat` is supplied by T16; do not duplicate it here.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:102-114` — X3, three planes on one core.
- `docs/09-shared-code-refactor-findings.md:652-657` — SYNC-03, `Record` planes.
- `docs/09-shared-code-refactor-findings.md:698-703` — SYNC-10, emit implementations.
- `docs/09-shared-code-refactor-findings.md:810-815` — CAP-12, emit part.

## Boundary / approach

Add a generic field-list/`Record` mechanic and a
`write_record_line(writer, line, log_hook)` sink in `girpr-report-core`, with the
broken-pipe policy parameterized. Keep each program's `Val` vocabulary and
layout local.

## Guardrails

- `Val` vocabularies are deliberately incompatible: girpr forbids booleans and
  renders flags as `1`/`0`; capture renders `true`; girsync carries
  `serde_json::Value` and positional layouts.
- `Order::Alphabetical` is girsync's contract.
- Stdout locking and broken-pipe policies differ; do not silently unify
  girsync's `println!` with capture's locked stdout.
- Event-name derivation and field order stay at call sites.
- Do not enable `serde_json/preserve_order` (A10).

## Commit breakdown

1. Add generic ordered field-list/record mechanic with tests.
2. Add the line sink with parameterized broken-pipe policy.
3. Adopt in `tools/sync` report plane.
4. Adopt in `app/report` plane.
5. Adopt in `tools/capture` plane.

## Validation

- Stdout byte-contract tests for all three binaries.
- `cargo nextest` for report-core, app/report and both tools.

## Done when

Record rendering and line emission have one implementation each, and all three
byte contracts are unchanged.
