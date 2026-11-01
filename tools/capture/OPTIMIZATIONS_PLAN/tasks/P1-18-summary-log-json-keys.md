# P1-18 — Summary log line and escaped JSON keys

Report: `../../OPTIMIZATIONS_REPORT.md` lines 356-368 (A18), 370-381 (A19),
and 744-750 (D6 context).

Dependencies: none.

## Goal

1. The terminal `SUMMARY` record must reach the JSON-lines log file.
2. `report::push_json_key` must produce valid JSON for a key that needs
   escaping.

## Where to start

- `src/main.rs`: the `log_guard.finish()` call (around line 63) and the
  `summary.emit(...)` / `exit::exit_with(code)` sequence.
- `src/report/mod.rs`: `push_json_key` (around line 345), `is_plain_ascii_json`,
  `json_string`, `emit`.
- `src/logging/mod.rs`: `LogGuard::finish` (around line 65).
- `src/report/tests.rs` and any logging test that runs a command with
  `--log-file`.

## Steps

1. Move `log_guard.finish()` to after `summary.emit(common.output)` (and after
   any `pre.emit` in the error path that should be logged). It must still run
   before `exit::exit_with(code)`, because `exit_with` terminates the process.
2. Confirm `summary.emit` emits the `tracing::debug!` record while the guard is
   still open, and that `finish` flushes it. Do not duplicate the SUMMARY line
   by logging it twice.
3. Fix `push_json_key`: for a plain ASCII key, keep the fast path (quote +
   push + quote). For a non-plain key, do **not** add extra quotes around
   `json_string(k)`, because `json_string` already returns a quoted literal.
   Either return the quoted form directly or restructure so there is one
   quoting owner.
4. Add a unit test for `push_json_key` with a key containing `"`, `\`, and a
   control character; parse the result with `serde_json` or compare to
   `serde_json::to_string`.
5. Add a test (if the logging harness allows it) that a run with `--log-file`
   writes a final `SUMMARY` record. If a full run is too heavy, test
   `report::emit` plus the guard ordering at the smallest seam available.

## Tests

- `push_json_key` plain key: output unchanged.
- `push_json_key` escaped key: output is valid JSON and the key round-trips.
- A report line built with an escaped dynamic key parses.
- Log test: the last JSON-lines record is the `SUMMARY` event, or at least a
  `SUMMARY` record is present.
- Existing report/logging tests pass.

## Acceptance

- The log file is self-contained: it includes the run's final verdict.
- No extra or duplicate SUMMARY line.
- Dynamic keys cannot corrupt a JSON line.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(logging): flush the log after the summary is emitted`
2. `fix(report): quote an escaped JSON key exactly once`
3. `test(report,logging): cover escaped keys and the terminal summary`

## Notes

- Check `exit_with`'s implementation before moving `finish`; it must not need
  the guard already closed.
- Do not change the text SUMMARY format.
- Do not reference the report or plan in code comments.
