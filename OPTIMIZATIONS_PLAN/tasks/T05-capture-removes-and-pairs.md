# T05 — Remove capture no-op test; merge same-function pairs

Priority P0 · crate gircap · checkpoint CP3

## Pointers (give these to the implementing session)

* Report findings: `docs/08-test-suite-reduction-audit.md:L177–L180` (§E.1),
  `L188–L191` (§E.4).
* Report "kept" note: `docs/08-test-suite-reduction-audit.md:L200–L204` (§E.6).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T05-capture-removes-and-pairs.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Two safe reductions in gircap:

1. **REMOVE no-op:** `src/hash/tests.rs:578`
   `the_measured_archive_digest_is_reproducible` only re-asserts
   `md5_hex(b"").len() == 32` (also at 546/472) and the zstd magic (also at
   623/634); its doc comment claims to pin a 9,007,187-byte digest that never
   appears. **1 saved, low risk.**
2. **MERGE five same-function pairs:** `src/report/tests.rs` 321/331, 142/152,
   589/605; `src/config/tests.rs` 411/483; `src/manifest/proto.rs` 225/243. For
   the last, the "no allocation" claim is not measured — drop the misleading
   doc, keep the equality assertion. **5 saved, low–medium risk.**

Expected ~6 tests removed.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/capture/src/hash/tests.rs:578` (and the duplicate anchors 546/472,
  623/634 to confirm the assertions really are repeated).
* `tools/capture/src/report/tests.rs:321/331, 142/152, 589/605`.
* `tools/capture/src/config/tests.rs:411/483`.
* `tools/capture/src/manifest/proto.rs:225/243`.

Possible commits:

1. Remove the no-op test after confirming its assertions live elsewhere.
2. Merge the five pairs into single tests preserving both cases' assertions.

## Acceptance criteria

* No assertion lost; the equality assertion in `manifest/proto.rs` kept, only its
  misleading doc removed.
* Affected gircap unit suites pass; no product code touched.

## Guardrails

* Do not touch `tests/measured_fixture.rs` (§E.5 optional, follow-up F04) or the
  mutation-driven HTTP/verify/emit suites (§E.6).
* Do not touch capture `tests/cli_contract.rs` log-file process tests — those
  belong to T13. The `cli_contract.rs:214` test belongs to T12.
* Validate each anchor; skip pairs that differ in what they assert.
* Report follow-up ideas without implementing them.
