# T27 — Capture mock server → axum

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §CAP-1 (lines 655–672) — the ~270-line
  raw listener in capture test support becomes an `axum` `Router` + `axum::serve` with
  `State<Table>`; wiremock/httpmock are viable only if canned replies suffice.
- §X1 (lines 56–89) — cross-cutting HTTP-server finding.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28), §A10 (39–40).
  Keep items: `guidance/DO-NOT-TOUCH.md`, especially the malformed raw helper (line 84).

## Scope
- `tools/capture/test-support/src/lib.rs` — rationale (`:11–18`), `Mock::start`
  (`:1235–1266`), `serve` (`:1888–1988`), `is_blob_route` (`:1992–1994`), the flaky-failure
  table (`:1996–2020`), `respond` (`:2040–2095`), `write_response` (`:2132–2153`).
- Request-log rows in `tests/capture_layout.rs` (~1127, ~1194–1201), plus
  `tests/measured_fixture.rs`, `tests/offline_bundle.rs`, `tests/matrix.rs`,
  `tests/recorded_json_golden.rs`, `tests/schema.rs:558`.
- `tools/capture/test-support/Cargo.toml`. Locate consumers before editing.

## Implementation plan
1. Validate the four HYP endpoints, `getBuild`, the two CDN blob families, the armed
   503 "fail once" table and the `NoSuchKey` XML body. Keep `Table`, `build_table`,
   `Recorded`, `Mock::in_memory`/`transport` unchanged.
2. Replace `start`/`serve`/`write_response` with axum routes and a state lock for
   `flaky`; turn path guards into extractors. Add axum as a dev-dependency of test support.
3. Preserve the in-memory `HttpTransport` seam (§CAP-1 line 671). The deliberately
   malformed raw-socket helper in `http/src/tests.rs:114+` stays raw and is **not** part
   of this task.
4. Regenerate recorded `resp_headers` and request-log/golden files — expect header
   spelling changes — while keeping every semantic assertion.

## Constraints / validation
- Test-only blast radius (§A10); no production path changes.
- Recorded headers in `api/requests.ndjson` will change spelling (§CAP-1 lines 666–667);
  regenerate rather than delete assertions.
- FNV/walkdir source-key work belongs to T11; URL/query parsing to T09. Do not duplicate
  those changes here — coordinate through `STATE.md`.

## Done when
The raw listener is gone, axum serves all routes with the flaky table intact, goldens are
regenerated, and the capture test suites pass. If a premise is stale or a keeper would
change, **stop** and record `skipped`.
