# T17 — Leaf-crate test cleanups

## Goal / pointers
- `docs/10-test-optimization-research.md`:
  - §F27 (lines 362–366) — `crates/report/src/ledger.rs` concurrency tests spawn 40
    threads for exact sums; only one case uses a `Barrier`. Reduce the two
    barrier-free sum tests to 4×25 or one thread; keep barrier tests at 8.
  - §F28 (lines 368–374) — redundant tag-6 byte assertion in `crates/fetch/tests/fetch_direct.rs`
    and a duplicate round-trip in `crates/fetch/src/sophon/proto.rs` (value 0, so it
    does not pin tag 6). Keep the integration binary for its "consumable without the
    facade" compile check and the proto crate's authoritative test; drop the duplicate.
- Assumptions to validate: §A1 (46–52), §A10 (108–110), §6 (503–510).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md`.

## Scope
- `crates/report/src/ledger.rs`, `crates/fetch/tests/fetch_direct.rs`,
  `crates/fetch/src/sophon/proto.rs`, `crates/proto/src/lib.rs` (authoritative tag-6
  test). Locate current sites before editing.

## Implementation plan
1. Validate §F27: confirm which concurrency tests have a `Barrier` and which do not;
   reduce only the barrier-free sum tests, keeping the genuine race test at 8 threads.
2. Validate §F28: confirm the authoritative tag-6 test in the proto crate still pins
   bytes; remove the duplicate assertion from the integration test while keeping the
   integration binary and its compile check.
3. Update nextest filters if a named case moves (§A10); deliver in separate commits.

## Constraints / validation
- Keep the barrier-based race test's thread count; the reduced tests must still assert
   exact sums (§6).
- Do not remove the `fetch_direct.rs` binary — its "consumable without the facade"
   compile check is the point (F28, §5).
- Tag-6 bytes remain pinned by the authoritative test; no byte contract may be lost.

## Done when
Barrier-free ledger tests use fewer threads with exact-sum assertions intact, the
duplicate tag-6 assertion is gone while the authoritative byte test and the direct
fetch binary remain, and both crate suites pass.
