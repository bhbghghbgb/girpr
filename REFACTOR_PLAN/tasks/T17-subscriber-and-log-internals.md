# T17 — Tracing subscriber assembly and `girpr-log` internals

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Share the subscriber stack assembled by all three binaries and tidy the
duplicated internals of `girpr-log`, without taking ownership of each program's
guard or flush behavior.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:88-100` — X2, subscriber stack.
- `docs/09-shared-code-refactor-findings.md:659-663` — SYNC-04, caveats.
- `docs/09-shared-code-refactor-findings.md:796-799` — CAP-10, capture caveat.
- `docs/09-shared-code-refactor-findings.md:615-619` — LF-13, duplicated writers.
- `docs/09-shared-code-refactor-findings.md:621-626` — LF-14, parse-and-warn.

## Boundary / approach

Add `subscriber_for(plan, console_writer, format, span_fields)` in
`girpr-log::install`, leaving guard ownership per program. Internally, share the
`Arc<Mutex<Vec<u8>>>` writer/test helper and add a private `parse_or_warn`
helper.

## Guardrails

- Girpr's process-global slot and `flush_log` on every exit path stay local.
- The tools' `LogGuard` drop/finish stays local.
- `PROJECT`, warning prefixes, targets and file format are caller parameters.
- Capture's `AppendFile` serializes writes; girpr writes text rather than JSON.
- `crates/log/src/install.rs:1-14` documents the current split as intentional;
  only the duplicated remainder moves.
- The two warning strings in `filter.rs` are asserted and must stay byte-identical
  unless tests are deliberately updated.
- Keep `CaptureBuffer` `pub(crate)` unless the feature surface is intentionally widened.

## Commit breakdown

1. Add `subscriber_for` with tests for layer/filter choice and degradation path.
2. Adopt in girsync logging.
3. Adopt in capture logging, preserving `MakeWriter` and flush order.
4. Adopt in girpr logging, preserving the static guard and `flush_log`.
5. Consolidate internal writer helpers and `parse_or_warn`.

## Validation

- Every log-capture assertion in the workspace; flush-on-exit tests.
- `cargo nextest` for girpr-log and the three binaries.

## Done when

Subscriber construction has one implementation, guards remain per program, and
log bytes are unchanged.
