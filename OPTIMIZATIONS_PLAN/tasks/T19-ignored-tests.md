# T19 — Tidy the ignored tests (bundle reuse, peak_heap, stale case)

Priority P2 · crate girpr/girsync · checkpoint CP5

## Pointers (give these to the implementing session)

* Report finding: `docs/08-test-runtime-optimization.md:L408–L411` (§6.3 5) and
  `L445–L462` (§7).
* Report priority row: `docs/08-test-runtime-optimization.md:L478` (§8, P2).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T19-ignored-tests.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".

## Goal

These tests are `#[ignore]`d and cost zero in the normal gate, so this is
lowest priority and must not distract from P0/P1. If they are ever enabled, the
bridge suite alone would add ~4 minutes. Three concrete tidies: (1) structure the
artifact-bridge suite to verify the immutable bundle once and share the
manifest/bridge instead of re-verifying/re-binding four listeners per case;
(2) shrink `peak_heap`'s payload (~2.16 MB → ~256 KiB — the ratio is
scale-independent); (3) fix or delete the currently failing ignored test
`girsync::sync::rows_for_a_directory_created_and_removed_in_one_run_are_dropped`
(`sync.rs:1431`: "a row survived for Sub").

## Implementation plan

Code anchors from the report (validate before editing):

* `tests/artifact_bridge.rs` — 6 cases, ~38–48 s each; each re-runs `gircap
  verify` and binds 4 listeners.
* `tests/peak_heap.rs:268` — payload size.
* `tools/sync/tests/sync.rs:1431` — the failing ignored test.

Possible commits:

1. Share the bundle verification / bridge setup across the six bridge cases.
2. Shrink the `peak_heap` payload while keeping the ratio assertion meaningful.
3. Investigate the failing ignored test: fix it if the failure is a real stale
   expectation, otherwise delete it. Record which and why.

## Acceptance criteria

* Bridge cases share verification if they are ever enabled; no per-case
  re-verification of the same immutable bundle.
* `peak_heap` still asserts its scale-independent ratio with a smaller payload.
* The failing ignored test is fixed (passes) or removed; no known-failing test
  remains.

## Guardrails

* Validate against current code first; if these tests changed, stop and mark
  `skipped` with a note.
* These are ignored tests — do not add them to the normal gate. Do not change
  product behavior to make the stale test pass unless clearly correct; prefer
  removing a stale test.
* Run ignored tests with `cargo nextest run --workspace --run-ignored only` to
  verify the affected ones. Report follow-up ideas without implementing them.
