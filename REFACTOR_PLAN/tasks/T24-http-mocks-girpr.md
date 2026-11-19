# T24 — girpr canned HTTP reply server

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A5 (lines 172–181, 192–200),
  §H (line 799).

## Scope
- Package: root `girpr`, `src/test_http.rs` (lines 17–57, raw-socket canned
  replies).

## Implementation plan
1. Replace the raw-socket canned replies with the same mock library chosen in
   T23 where the server only feeds `reqwest` controlled responses.

## Constraints / validation
- **Out of scope:** the bridge server in `tests/artifact_bridge/server.rs`. Its
   four origins, route families, build-id invariants and recorded failure shapes
   are domain logic a mock library cannot express; keep it (§A5 lines 194–196,
   §H line 799).
- Tests named in §A5 (lines 198–200); none pin the toy server implementation,
   only its responses and request log.

## Done when
The canned reply server is delegated and no domain-server behavior was changed.
