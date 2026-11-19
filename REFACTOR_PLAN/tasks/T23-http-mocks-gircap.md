# T23 — gircap HTTP mock servers

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §A5 (lines 172–206), §F2
  (lines 648–652).
- Assumption to validate: §I.11 (lines 847–850).

## Scope
- Package: `tools/capture`, `src/http/tests.rs` (`local_server`),
  `tests/common/mod.rs` (`Mock`, `serve`, `MemoryTransport`).

## Implementation plan
1. Replace canned `serve_bodies`/`serve_status` servers with `wiremock` 0.6 or
   `mockito` 1.7 using `expect(n)` / `received_requests()`.
2. Keep `Served`, `Hit`, `HitKind`, `Bundle`, `Response` and URL-prefix rewriting
   as shared domain state.
3. If the real mock server is fast enough for the 113 610-chunk tests, the
   `HttpTransport`/`MemoryTransport` injection seam (`src/http/transport.rs`
   lines 208–369) can be deleted entirely (§A5 restructure).
4. Assert only headers the product reads (`etag`, `content-md5`, `content-type`);
   incidental `Date`/`Server`/`transfer-encoding` change (§A5 caveat).

## Constraints / validation
- Keep one small raw-socket helper for the truncated/lying `Content-Length`
   tests (`src/http/tests.rs` lines 716–732) — a well-behaved mock server cannot
   lie about length (§I.11).
- Domain servers are not delegable; only the canned-reply servers here.
- Tests named in §A5.

## Done when
Canned servers use a mock library, domain state is preserved, and malformed-
response tests still work.
