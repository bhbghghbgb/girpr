# T15 — ScriptedServer → wiremock

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §LEAF-1 (lines 415–430) —
  `girpr-loopback-mock`'s `ScriptedServer` owns a listener, driver thread, parser,
  header map, status table and lifecycle; `wiremock` (or `mockito`/`httpmock`) supplies
  those mechanics while `Reply`/`RecordedRequest`/`ScriptedServer` stay as a thin wrapper.
- §X1 (lines 56–89) — cross-cutting HTTP-server finding.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28), §A10 (39–40).
  Keep items: `guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/loopback-mock/src/lib.rs` (whole, `:1–317`; notably `:26–91`, `:93–226`,
  `:228–317`) and its `Cargo.toml` (currently dependency-free).
- `drop_does_not_hang_on_an_unfinished_request_line` (`:328–364`).
- Downstream users in fetch/capture/root tests that assert on `base()`, `hits()`,
  `requests()` and header values. Locate them before editing.

## Implementation plan
1. Validate the wrapper's public contract: ordered replies with "last reply repeats",
   `start_expecting(n)` hit-count assertion, recorded request headers/bodies, and
   `base()` shape.
2. Map replies to `ResponseTemplate`, register ordered mocks with `up_to_n_times(1)`
   (last repeats), implement `start_expecting` with `expect(n)`, and translate
   `received_requests()` into `RecordedRequest`/headers.
3. Delete the driver thread, `serve_one`, `reason()` and the custom `Headers` type;
   add `wiremock` as a dev-dependency of the leaf (acceptable per §LEAF-1 line 429).
4. Remove the now-obsolete lifecycle test; keep downstream assertions on base/hits/
   requests/headers, regenerating header spelling where needed.

## Constraints / validation
- "Last reply repeats" and "assert hit count on drop" must remain reproducible
  (§LEAF-1 line 430). If a library cannot reproduce them, stop rather than weakening the
  contract.
- The leaf is shared by several products; its public API shape must stay compatible or
  all callers must be updated in the same task.
- Test-only blast radius (§A10); no production path changes.

## Done when
The listener/parser/writer/lifecycle code is gone, `ScriptedServer` is a thin wiremock
wrapper, downstream tests pass, and the obsolete lifecycle test is removed. If a premise
is stale or a keeper would change, **stop** and record `skipped`.
