# T14 — girsync JSON renderer → `Serialize`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A1 (lines 24–68), §E4
  (lines 588–592).

## Scope
- Package: `tools/sync`, `src/report/record.rs` (`json`, `write_json`,
  `push_bare`, `push_json_member`, `push_json_value`, `push_json_string`,
  `push_json_char`).

## Implementation plan
1. Implement `serde::Serialize` for the record type using
   `serialize_map(Some(n))` + `serialize_entry(k, v)`.
2. Make `json()` become `serde_json::to_string(self)`, removing the ~70 LOC of
   alphabetical key sorting and the manual JSON escaper (§E4).
3. Decide explicitly whether to keep girsync's current alphabetically sorted
   keys or move to declaration order to match the siblings; record the decision
   in `STATE.md` persistent context (§A1 "one decision per workspace").

## Constraints / validation
- girsync's key order may change if declaration order is adopted (§A1 caveat).
- Tests named in §A1 (lines 58–62) pin exact bytes including sorted keys, plus
  `tools/sync/tests/output.rs` (§E4). These are the byte-level oracle.

## Done when
The hand-rolled renderer is replaced and the ordered/sorted-key decision is
validated against the byte-level oracle.
