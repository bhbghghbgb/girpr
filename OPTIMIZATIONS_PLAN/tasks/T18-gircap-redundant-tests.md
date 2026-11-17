# T18 — Merge redundant gircap read/verify test groups

Priority P1 · crate gircap · checkpoint CP4

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L344–L349` (§6.2 3) and
  `L359–L364` (§6.2 5).
* Report priority row: `docs/08-test-runtime-optimization.md:L477` (§8, P1).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T18-gircap-redundant-tests.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

Two related cleanups: (§6.2 5) the three
`every_command_that_reads_a_bundle_agrees_*` cases (`capture_layout.rs:414,470`
and the rejection twin at `498`) are subsumed by their `a_bundle_without_*_does_not_verify`
partners plus the materialize variant — merge them; `cli_contract.rs::the_binary_exists_where_this_test_expects_it`
(`:102–104`) is subsumed by every other spawning test. (§6.2 3) `verify::reverify`
re-hashes/re-decodes every file; verify once per shared bundle rather than once
per test where the shared bundle from T06 is available.

## Implementation plan

Code anchors from the report (validate before editing):

* `tools/capture/tests/capture_layout.rs:414`, `:470`, `:498` — redundant
  "agree" cases; their `a_bundle_without_*_does_not_verify` partners.
* `tools/capture/tests/cli_contract.rs:102–104` — binary-exists case.
* `src/verify/report.rs:144–199` — `verify::reverify` (product code, do not
  change); call sites in `capture_layout.rs` (6), `matrix.rs`, `offline_bundle.rs`
  (~11).

Possible commits:

1. Remove/merge the subsumed "agree" cases, keeping the partners plus
   materialize variant; drop the trivial binary-exists case.
2. Where T06's shared read-only bundle exists, verify it once and have the
   cases read that verified state instead of each calling `reverify`.

## Acceptance criteria

* Fewer verify calls and fewer redundant tests; all distinct behaviors remain
  covered by the kept tests.
* No product change to `verify::reverify` — only its call sites are reduced.
* The shared-bundle verification is race-free under nextest.

## Guardrails

* Validate against current code first (and confirm T06's shared bundle landed);
  if the prerequisites are absent, stop and mark `skipped` with a note, or do
  only the independent redundancy removals.
* Do not change `verify::reverify` product behavior. Be conservative merging —
  if an "agree" case asserts something its partner does not, keep it and note it.
* Run the gircap suites to verify; report follow-up ideas without implementing them.
