# T04 — Root test origin server → axum

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §R2 (lines 157–170) — the hand-rolled
  HTTP/1.1 test server (`Reply`/`InFlight`/`serve`/`handle_connection`) should become
  an `axum` `Router` + `axum::serve`; `wiremock` fits the canned cases but not the
  held-open/gated ones.
- §X1 (lines 56–89) — the cross-cutting server finding and its test-impact guidance.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28), §A10 (39–40).
  Keep items: `guidance/DO-NOT-TOUCH.md`, especially the raw malformed helper (line 84).

## Scope
- `src/test_origin.rs` (whole file): `Reply` (`:17–50`), `InFlight` (`:60–133`),
  `serve` (`:177–221`), `handle_connection` (`:224–275`).
- `src/repair/phases/manifest.rs:554–865` test setup; root `Cargo.toml` dev-dependencies.
- Locate the downstream users before editing; the list above is a hint.

## Implementation plan
1. Validate the server's scripting policy (`NeverAfterHit`, `StatusAfterHit`,
   `GatedBody`) and in-flight counting in the current tests. Keep `Reply`/`Route` as
   handler state and keep `InFlight` with its `Notify`/watch gate.
2. Replace the `TcpListener`/`JoinSet` accept loop, request-target extraction and status
   line with axum routes and `axum::serve`. Hold a request open with
   `std::future::pending()`. Add axum as a dev-dependency.
3. Rewrite the setup in `manifest.rs` around the new server handle; keep `TestServer::base()`
   returning a string if callers expect that.
4. Regenerate recorded request headers/bodies or golden files whose spelling changes;
   keep every semantic assertion (attempt counts, cancellation, bounded concurrency,
   timeout).

## Constraints / validation
- Test-only blast radius: no production path changes (§A10, lines 39–40).
- The deliberately malformed raw-socket helper is **not** part of this task and stays raw
  (§X1 line 84; §CAP-1 line 672).
- Lifecycle tests that pin the hand-written accept loop/shutdown become obsolete and may
  be removed (§X1 lines 87–88); tests of the intended behavior must be adapted, not
  deleted (§A5).

## Done when
The origin server is axum-based, the gating/in-flight policy survives, all manifest
tests pass with regenerated fixtures, and no raw parser remains in `test_origin.rs`. If
a premise is stale or a keeper would change, **stop** and record `skipped`.
