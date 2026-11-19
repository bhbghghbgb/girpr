# T13 — girpr JSON renderer → `Serialize`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A1 (lines 24–68), especially
  the candidate (46–49), restructure (51–56), girpr tests (58–62) and caveat
  (64–68).

## Scope
- Package: root `girpr`, `src/report/record.rs` (`Record::json`, `bare`,
  `push_lowercase`, `push_json_string`, `push_json_event`, `push_json_value`).

## Implementation plan
1. Implement `serde::Serialize` for the local `Record`/`Val` types using
   `serialize_map(Some(n))` + `serialize_entry(k, v)` yielding entries in the
   current declaration order (`Str | UInt | Int`).
2. Make `json()`/`to_json_line()` become `serde_json::to_string(self)`.
3. Do not enable `preserve_order` — `serialize_map` emits in the order the impl
   yields; `serde_json` is already a dependency.

## Constraints / validation
- Escaping of control characters stays byte-identical (§A1 caveat).
- girpr uses declaration order; do not adopt another tool's ordering.
- Tests named in §A1 (`src/report/tests.rs`) pin exact bytes. NDJSON consumers
  stay valid.
- This is one of three sibling tasks (T13–T15). The "one decision per workspace"
  note (§A1 header) should be recorded in `STATE.md` persistent context once
  settled, so T14/T15 can reuse the decision.

## Done when
The hand-rolled renderer in this crate is replaced by `serde` and the byte-level
tests pass (or any intentional byte change is recorded).
