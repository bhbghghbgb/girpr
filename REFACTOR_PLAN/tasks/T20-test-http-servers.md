# T20 — Test HTTP/loopback server scaffolding

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Share socket/stop/gate mechanics for the canned and replay HTTP servers, add a
raw-scripted reply mode for the malformed tests, and consolidate argv/binary
invocation and junction-fallback helpers.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:217-236` — X12, server scaffolds.
- `docs/09-shared-code-refactor-findings.md:357-365` — R-12, route-based origin.
- `docs/09-shared-code-refactor-findings.md:367-373` — R-13, binary invocation helpers.
- `docs/09-shared-code-refactor-findings.md:665-670` — SYNC-05, junction fallback.
- `docs/09-shared-code-refactor-findings.md:692-696` — SYNC-09, argv prefix.
- `docs/09-shared-code-refactor-findings.md:829-842` — CAP-15/CAP-16, capture servers.
- `docs/09-shared-code-refactor-findings.md:481-486` — APP-14, optional helper only.

## Boundary / approach

Extract socket/stop/gate mechanics in `girpr-loopback-mock`, add a route-driven
`RouteServer` and an opt-in raw-scripted reply mode, add a test-only
`girpr-test-http` leaf with an injected handler, and add `argv_with_program` to
`girpr-cli` plus one binary-runner helper in `tests/common`.

## Guardrails

- `docs/00-spec.md` §2 forbids sharing gircap's *fixtures* with repair; only
  server mechanics may move.
- The canned wiremock server, the axum replay server and the axum world server
  serve deliberately different contracts; do not merge the contracts or the
  per-test reply policies.
- The malformed tests require a server that can lie about `Content-Length`; a
  wiremock-only replacement is impossible.
- Capture records full argv in `MANIFEST.json → tool_argv`; the sentinel spelling
  is a byte contract.
- If the binary helper moves into a library, the exe path must be an explicit
  parameter.
- Both symlink and junction mechanisms must remain available, including the
  `"{first}; junction fallback failed: {e}"` wording where asserted.
- APP-14's wrappers are deliberately local; only an optional `script_bodies`
  helper is in scope.

## Commit breakdown

1. Share socket/stop/gate mechanics in `girpr-loopback-mock`.
2. Add route-driven `RouteServer`; adopt `src/test_origin.rs`.
3. Add raw-scripted reply mode; adopt capture's malformed-server tests.
4. Add `girpr-test-http` leaf; adopt capture/test-fixture axum scaffolds.
5. Add `argv_with_program`, binary runner and junction fallback helper.

## Validation

- Raw-socket malformed tests, journal assertions and `MANIFEST.json` goldens.
- `cargo nextest` for loopback-mock, test fixtures, capture test-support and root tests.

## Done when

Scaffolding is shared but the three server contracts and all reply policies
remain distinct.
