# T24 — cache-json NDJSON stream + direct hex

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §SYNC-6 (lines 601–612) — the hand-rolled
  `lines()`/`line_no`/`parse_line` NDJSON import becomes
  `Deserializer::from_reader(...).into_iter::<Line>()`; the report explicitly notes the
  initially suggested `json-line` crate does not exist (§A11, lines 44–47).
- §SYNC-7 (lines 614–622) — the three `girpr_digest::encode_hex`/`decode_hex` wrapper
  calls become direct `hex` calls and the dependency is dropped from this crate.
- Validate assumptions first: §A1 (15–16), §A2 (17–18), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/sync/cache-json/src/convert.rs:89–188`; `cache-json/src/ndjson.rs:94`, `:97`,
  `:121`, `:134–148`; `cache-json/Cargo.toml:37–38`.
- `cache-json/tests/roundtrip.rs:143–217` (pins `"line 2"`, `"line 3"`, `"blank line"`,
  `"second meta line"`, `"repeats path"`, bad-hex error, temp cleanup).

## Implementation plan
1. Validate the meta-first and duplicate-path rules and how blank/malformed lines are
   classified today. Replace the line loop with a `serde_json` `StreamDeserializer`
   fold that keeps both rules.
2. Decide how to report errors: the deserializer exposes `byte_offset()`, not a line
   number. Either drop the "line N" wording or count `\n` bytes; keep the error wording
   that tests actually assert where practical (§SYNC-6 lines 607–608).
3. Replace the three hex wrapper calls with `hex::encode`/`hex::decode` and remove
   `girpr-digest` from this crate's manifest (keep it where else used). `write_line` stays.
4. Update the roundtrip tests for the chosen error wording; keep every semantic assertion.

## Constraints / validation
- Accepting whitespace-separated JSON objects instead of strict one-per-line is OK
  (§SYNC-6 line 612).
- `hex 0.4.3` is already locked; `serde_json` is already used. No dependency on the
  non-existent `json-line` crate (§A11).
- `FromHexError` is the same underlying type, so bad-hex error content should survive
  (§SYNC-7 lines 620–622).

## Done when
The line-number parser and hex wrappers are gone, meta-first/duplicate rules still hold,
and the roundtrip tests pass with adjusted wording. If a premise is stale or a keeper
would change, **stop** and record `skipped`.
