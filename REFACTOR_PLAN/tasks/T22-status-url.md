# T22 — Status error + URL join → `reqwest`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §B2 (lines 374–392).

## Scope
- Package: root `girpr`, `src/sophon/stream.rs` (`status_failure`, `verdict`,
  `retryable`, `join_url`) and `tests/repair_offline.rs`.

## Implementation plan
1. Resolve `url_prefix` into a `reqwest::Url` at the manifest boundary, use
   `.join(id)?` per blob instead of the hand-rolled `format!("{}/{}", …)`.
2. Use `reqwest::Response::error_for_status_ref()` for the canonical status
   error instead of the hand-built one.
3. Keep the narrow `retryable(u16)` policy (Starward/gircap parity) but express
   it with `StatusCode::is_server_error()` plus `408 | 429`.

## Constraints / validation
- `Url::join` percent-encodes ids and resolves `.`/`..`; ids are opaque hashes so
   normal runs are unaffected. A prefix without a trailing `/` replaces its last
   path segment — validate production prefixes (§B2 caveat).
- Tests named in §B2.

## Done when
The hand-built status error and URL join are gone and the named tests pass.
