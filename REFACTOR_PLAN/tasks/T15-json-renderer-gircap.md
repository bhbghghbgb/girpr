# T15 — gircap JSON renderer → `Serialize`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A1 (lines 24–68), §F1
  (lines 642–646).

## Scope
- Package: `tools/capture`, `src/report/mod.rs` (`to_json_line`, `push_json_key`,
  `is_plain_ascii_json`, `json_string`, `push_json_value`).

## Implementation plan
1. Implement `serde::Serialize` for the `Val` enum/record using
   `serialize_map(Some(n))` + `serialize_entry(k, v)` in event-first order
   (`Str | Int | Bool`, per §A1 restructure).
2. Make `to_json_line()` become `serde_json::to_string(self)`, removing the
   `is_plain_ascii_json` fast path (§F1, lines 644–645).

## Constraints / validation
- Backslash escaping: `is_plain_ascii_json` currently leaves backslashes
   unescaped while `serde_json` emits `\\`, so Windows-path lines change (§A1
   caveat). Retain a thin escape helper **or** explicitly accept the change and
   record it.
- Escaping of control characters stays byte-identical.
- Tests named in §F1 (`tools/capture/src/report/tests.rs`) pin exact bytes.

## Done when
The hand-rolled renderer is gone and the backslash-escaping decision is recorded
and tested.
