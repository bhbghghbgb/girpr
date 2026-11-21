# T05 — Replace wiremock with a std-only loopback HTTP mock

## Goal / pointers
- `docs/10-test-optimization-research.md` §F2 (lines 145–160) — wiremock is used in four
  places (root `src/test_http.rs`, `crates/fetch/src/test_http.rs`,
  `tools/capture/src/http/tests.rs`, `tools/capture/http/src/tests.rs`) and is the
  exclusive source of `regex`, `futures`, `deadpool`, `assert-json-diff` and
  `base64 0.22`. Fix: a tiny dev-only, dependency-free leaf crate exposing scripted
  status/body/headers, request journal and hit counts; migrate the four sites and drop
  wiremock from root, `girpr-fetch`, `gircap`, `gircap-http`. Keep at most one wiremock
  smoke if wire compatibility is contractual.
- Fold-ins: §F11 (243–249, two duplicated hand-rolled servers in root `src/`),
  §F25 (346–350, `girpr-fetch`'s six canned-reply tests), §A9 (101–106).
- Assumptions to validate: §A1 (46–52), §A3 (61–65), §A10 (108–110), §A11 (112–116),
  §6 (503–510). Guidance: `DO-NOT-TOUCH.md` (cross-program seam boundary).

## Scope
- New leaf crate (proposed `crates/…`) with no product dependencies; four migration
  sites and their call sites (`repair/mod.rs`, `fetch/hyp.rs`,
  `fetch/sophon/stream.rs`, both capture http test files, root repair
  `manifest.rs`/`metadata.rs` servers).
- Manifests of root, `girpr-fetch`, `gircap`, `gircap-http`, plus `Cargo.lock` and
  `cargo tree -d` verification. Locate all four copies and the existing raw-socket mock
  patterns (`test-fixtures`, capture) before editing.

## Implementation plan
1. Validate §F2/§A9: confirm wiremock's exclusive dependency trees with `cargo tree -i`
   and `-d`, and enumerate every call site (including the `received_requests()` API
   that has no std equivalent).
2. Create the dependency-free leaf with scripted replies, a request journal and hit
   counts; the existing helpers already count hits with `AtomicUsize`, so only the
   journal API needs replacing.
3. Migrate the four sites one at a time, folding in the two duplicated root `src/`
   servers (F11) and the fetch canned-reply helper (F25). Give the metadata server a
   `Drop`-abort driver so listeners do not leak until runtime teardown.
4. Remove wiremock from the four manifests (and any optional feature), verify
   `cargo tree -d` no longer shows its exclusive duplicates, and decide whether to keep
   one wiremock smoke for compatibility.
5. Deliver in several commits: leaf crate, migrations per program, manifest cleanup.
   Update gircap doc tables/manifest comments if the new leaf becomes its dependency.

## Constraints / validation
- The new leaf must have **no product dependencies**, so `girpr-fetch` and
  `girpr-test-fixtures` can use it without a cycle (§A9, `DO-NOT-TOUCH.md`).
- Do not merge the distinct fake-server contracts; the capture mock explicitly refuses
  to borrow from the product's. Keep the four servers' behaviors intact.
- No byte, status, header or URL contract may change; assert tests still pin the same
  replies (§6). Do not add sleeps (§A6).
- Under nextest one process runs per test (§A1) — a shared server cannot be shared
  across tests, so keep per-test server instances.
- Isolated payoff (§A11) is measured with `cargo nextest run -p girpr-fetch` and
  `cargo tree -e dev -p <crate>`; a workspace run alone will not show the win.

## Done when
All four sites use the std-only mock, wiremock (and its exclusive trees) is gone from
the four manifests except possibly one documented smoke, every test still pins its
original reply/status/header contract, and the dependency tree is verified.
