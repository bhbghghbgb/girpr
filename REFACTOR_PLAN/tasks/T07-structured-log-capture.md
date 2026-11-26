# T07 — Structured JSON log capture

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §R8 (lines 244–257) — tests parse
  `tracing`'s human log format by hand; use `tracing-subscriber`'s `json` feature plus
  `serde_json` and read `level`/`message` fields directly.
- §LEAF-10 (lines 530–534) — stale `girpr-log` comments still describe the old joined
  filter spec; update the comments (no crate applies).
- §X3 (lines 110–125) — cross-cutting "parsing other libraries' output" theme.
- Validate assumptions first: §A1 (15–16), §A5 (26–28), §A12 (48–50). Keep items:
  `guidance/DO-NOT-TOUCH.md` (shared crate — unrelated consumers must keep working).

## Scope
- `src/repair/phases/check.rs` — `logged_level`, `log_records`, `check_warnings`,
  `check_summary` (`:587–652`) and uses at `:723–747`, `:857`, `:938`, `:1064–1075`.
- `tests/repair_machinery.rs:318–342`; `crates/log/src/filter.rs:131–137`,
  `crates/log/src/plan.rs:57–88` (comments only).
- `girpr-log` test-support surface; root and `crates/log` manifests.

## Implementation plan
1. Validate that no shipped log format is expected by these tests; add a JSON-formatted
   capturing subscriber to `girpr-log`'s test-support surface (feature-gated as today)
   rather than parsing rendered text.
2. Rewrite `logged_level`/`log_records` and the test helpers to read structured
   `level`/`message` fields; remove the "numeric token is not a level" workaround
   (`:1064–1075`) and the `"2" parses as Level` special case.
3. Keep the warning-set and summary assertions, expressed over structured fields.
   Update the stale filter/plan comments in `girpr-log` to describe `Directive` appends.
4. Run the check-phase and repair-machinery tests plus any other `girpr-log` consumers.

## Constraints / validation
- `girpr-log` is shared: the new capture surface must not change shipped behavior or
  break consumers that use the existing test-support features (§A8 lines 35–36).
- No shipped log format changes (§R8 line 256). Tests must assert documented fields,
  not formatting quirks (§A12).

## Done when
No hand-written log-format parser or numeric-token workaround remains, the JSON capture
surface works for all consumers, comments match the code, and the tests pass. If a
premise is stale or a keeper would change, **stop** and record `skipped`.
