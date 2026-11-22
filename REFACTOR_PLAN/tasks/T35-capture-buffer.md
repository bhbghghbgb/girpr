# T35 — Shared capture-buffer primitive

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F40 (lines 557–565) — the log-capture
  buffer is implemented twice under different contracts; candidate boundary is a shared
  capture-buffer primitive, scoping/subscriber local. Confidence high.
- Assumptions to validate: §A4 (48–56), §A5 (57–67), §A10 (98–105), §A14 (129–135), §N12 (746–748).

## Scope
- `crates/log/src/test_support.rs` (`Captured` = `Arc<Mutex<Vec<u8>>>` + `Write` + strict
  UTF-8 `.text()` + debug-level scoped subscriber) and
  `crates/sync-test-support/src/lib.rs` (`SharedBuffer`/`BufferWriter` with
  `from_utf8_lossy` + `MakeWriter` + girsync's full subscriber stack). Locate before editing.

## Implementation plan
1. Validate §F40: confirm the two shapes and their strict-vs-lossy and `MakeWriter`
   differences; confirm `logs_of` itself is not duplicated (N12).
2. Define a shared buffer primitive (storage + `Write` + drain); keep scoping/subscriber
   installation local to each program.
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. If the sync stack would pull a program crate into the log leaf (§A4), scope down to the
   storage primitive only or stop.

## Constraints / validation
- High-risk (§F40): captures are asserted byte-for-byte; strict-vs-lossy UTF-8 or
   `MakeWriter` changes can silently alter lines (§A5).
- Do not pull a program crate into `girpr-log` (§A4). Do not absorb `logs_of` (N12).
- nextest one-process-per-test (§A10) — no cross-test sharing.

## Done when
A shared buffer primitive exists with both scoping stacks intact and captured bytes
unchanged, or a validated skip with the one-line reason is recorded.
