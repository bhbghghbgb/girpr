# T46 - T4: pin the output contract's ordering

Report: `../../OPTIMIZATIONS_REPORT.md` lines 1135-1150 (T4), and the note on
tests at lines 1218-1226.

Dependencies: T44, T45.

## Goal

`tests/cli_contract.rs` and `tests/artifact_bridge.rs` assert field
presence/values; the order and the one-record-per-event rule are implicit. The
README calls the data plane an automation contract. Reordering
`REPORT`/`PLAN`/`SUMMARY`, or emitting a second object for the same event, is a
breaking change the current tests may not catch.

Add a compact golden sequence for a scripted run: `--print-plan`, a tiny
repair, and a check-only. Pin event names, field keys, and ordering, not
free-form log lines.

## Where to start

- `tests/cli_contract.rs`: existing data-plane assertions.
- `tests/artifact_bridge.rs`: fixture-driven runs.
- `tests/repair_offline.rs`: the small repair fixtures (T47 may split it).
- `src/report/record.rs`: `text`, `json`, `render`, field order.
- `README.md`: the data-plane section (the contract wording).

## Steps

1. Choose a fixture that is small and deterministic: a two-file plan, one
   damaged file, one clean file, no network. Use the existing fixture server /
   temp-dir patterns.
2. For each scenario, capture the stdout lines and assert:
   - the **event order**: `REPORT`, then `PLAN` (only when requested), then
     zero or more `PROGRESS`, then exactly one `SUMMARY` last;
   - exactly one `PLAN` object per `--print-plan` (no duplicate line);
   - exactly one `SUMMARY` per run;
   - the top-level field **keys and order** for each event. For JSON, compare
     the raw key sequence or a parsed `serde_json::Value` with an ordered
     schema; do not compare the whole object's values.
   - `PROGRESS` may be absent in a short run; do not require it.
3. Store the golden as a compact list, not a full transcript:
   ```text
   report: local_version latest_version biz exe ...
   plan: files ...
   summary: total skipped repaired failed download_bytes deleted_extra_bytes exit
   ```
   Keep it in the test file or a small `tests/golden/` fixture. Do not include
   timestamps, durations, paths, or free-form log text.
4. Scenarios:
   - `--print-plan` (no writes): `REPORT`, `PLAN`, `SUMMARY`, exit `0`;
   - a tiny repair: `REPORT`, `SUMMARY`, exit `0`;
   - `--check-only`: `REPORT`, `SUMMARY`, exit `0`/`4` as the fixture says;
   - if a `PROGRESS` appears, it is between the fixed events.
5. Do not assert the exact values of version/build fields that the fixture
   controls; the fixture sets them, so they are stable, but the contract is the
   key/order.
6. Update the README's data-plane wording only if the test reveals an
   undocumented order. Do not change the implementation to satisfy a test that
   guessed wrong; the contract is what the code emits today, and T21/T20 may
   have changed the dry-run lines.

## Tests

- The new golden tests plus the full `cargo test -p girpr`.
- The golden must fail if `SUMMARY` moves before `PLAN`, if a second `SUMMARY`
  is emitted, or if a field key is renamed. Do not assert wording that a
  harmless change could alter.

## Acceptance

- One golden sequence per scenario, pinning event names, field keys, and order.
- The one-record-per-event rule is explicit.
- No free-form log lines in the golden.
- `cargo test -p girpr` passes.

## Commit plan

1. `test(output): pin the event order and one-record rule`
2. `test(output): add print-plan, repair, and check-only goldens`

## Notes

- Golden output is brittle to harmless wording changes; restrict it to
  event names/field keys and ordering, exactly as the report says.
- T47 may split the test file after this; keep the golden in the area that
  owns the output contract.
