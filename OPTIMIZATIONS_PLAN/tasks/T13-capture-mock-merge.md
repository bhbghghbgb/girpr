# T13 — Capture: merge mock / no-network full captures

Priority P1 · `tools/capture` (gircap) · checkpoint CP5 · report finding B4

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L281–L300` (§B4).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L805` (pattern 5).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T13-capture-mock-merge.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Fold four full-capture tests that each prove "the run only talked to the loopback
mock" into two, removing two full captures and two mock servers.

`tests/capture_layout.rs`:
* :205 `every_request_the_capture_made_went_to_the_mock` — path prefixes
  (:205–216) and `getGameBranches < getBuild` ordering.
* :1092 `the_request_log_rows_are_what_the_spec_says_they_are` — request-log row
  shape.
* :1339 `every_request_url_in_the_bundle_is_a_loopback_address` — recorded URLs
  (:1339–1361); redundant with the other two — delete.
* :1368 `the_chunk_cdn_base_comes_from_the_mock_not_from_the_region` — stored
  CDN base (:1368–1401); the product property worth pinning (the tool uses the
  bases it was given). Loopback is guaranteed by construction (`Mock::start`
  binds `127.0.0.1:0`, `common/mod.rs:1110–1112`).

## Implementation plan

Code anchors (validate before editing): `tests/capture_layout.rs:205–216`,
`:1092`, `:1339–1361`, `:1368–1401`, and the shared-capture mechanism in
`tests/common/mod.rs`.

Steps (one or two commits):

1. Keep :1368.
2. Fold the distinct assertions from :205 (prefixes + `getGameBranches < getBuild`
   ordering) and :1092 (request-log row shape) onto its one bundle.
3. Delete :1339.

## Acceptance criteria

* All distinct assertions (prefixes, ordering, row shape, CDN base) survive on
  the single shared bundle; 4 tests → 2, −2 full captures, −2 mock servers.
* Exact row/byte pins still hold on the shared capture — the report's stated
  risk (same world ⇒ same requests; verify, don't assume).
* Affected suite passes. No production behavior change.

## Guardrails

* If the row/byte pins do not hold when moved to the shared bundle, keep the
  tests separate and report `skipped` for that fold rather than weakening an
  assertion.
* Do not touch the byte-exact HTTP/verify/emit suites (report §B non-findings).
  Report follow-up ideas without implementing them.
