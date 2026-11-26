# T28 — Ordered JSON fixtures → serde_json Serialize

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §CAP-2 (lines 674–687) — the bespoke
  ordered-JSON tree and string escaper become a local `Serialize` implementation over
  `Vec<(String, J)>` rendered with `serde_json::to_string`.
- §A7 (lines 33–34) — **do not** enable `serde_json/preserve_order`; it would change key
  ordering for unrelated consumers such as `app/report`.
- Validate assumptions first: §A1 (15–16), §A5 (26–28), §A12 (48–50). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/test-support/src/lib.rs` — `J` (`:1511–1523`), `render`/`write`
  (`:1536–1572`), `write_string` (`:1580–1594`), `envelope`/`game_ref` (`:1596–1608`),
  `compact` (`:1613–1616`), body construction (`:1619–1849`).
- `tests/schema.rs:558`; `tests/recorded_json_golden.rs`; capture-layout pins on stored
  `api/*` bytes. `tools/capture/test-support/Cargo.toml`.

## Implementation plan
1. Validate that the JSON tree exists only to preserve insertion order. Implement a
   local `Serialize` over `Vec<(String, J)>` that calls `serialize_map` in order, then
   render with `serde_json::to_string`.
2. Delete the hand-written `write_string` escaper; keep `compact()`'s validity checking
   (or drop it deliberately if `to_string` already guarantees validity).
3. Regenerate the byte-exact body/golden pins; keep semantic assertions about the
   envelope fields and body content (§A5).
4. Confirm `preserve_order` is not enabled anywhere in the workspace as a result of
   this change (§A7 lines 33–34).

## Constraints / validation
- No `serde_json/preserve_order` — this is a cross-cutting constraint, not a suggestion.
- Fixture strings are assumed to use only escapes serde_json emits identically
  (§CAP-2 line 687); verify against the actual stored bodies before accepting.
- Test-only blast radius; no shipped JSON output changes.

## Done when
The hand-rolled tree/escaper is gone, insertion order is preserved by a local `Serialize`
impl, `preserve_order` is off, and the schema/golden/layout tests pass with regenerated
bytes. If a premise is stale or a keeper would change, **stop** and record `skipped`.
