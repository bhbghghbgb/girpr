# T41 — Shared line-sink primitive

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F46 (lines 617–623) — "write one
  rendered line + newline" is implemented in two shared leaves; candidate boundary is a tiny
  shared line-sink primitive. Confidence medium.
- Assumptions to validate: §A6 (68–75), §A5 (57–67), §A2 (29–36), §A14 (129–135).

## Scope
- `crates/report-core/src/lib.rs` (`write_line` writes a string + exactly one `\n`),
  `crates/log/src/install.rs` (hand-written warnings via `writeln!` with a caller-chosen
  target/prefix). Locate before editing.

## Implementation plan
1. Validate §F46: confirm the two implementations and that log's call sites need a
   caller-chosen target/prefix that `write_line` does not model.
2. If attempted, define a line-sink primitive that supports the target/prefix parameter;
   keep report-core as the sole ordered-JSON writer.
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. If making log depend on report-core pulls serde into every log consumer, scope down to a
   neutral primitive or stop.

## Constraints / validation
- High-risk (§F46): making log depend on report-core pulls `serde` into every log consumer
   and blurs "no report format is shared" (§A6). A new neutral primitive must be serde-free.
- Exactly one newline per line (§A5); emitted warning bytes must not drift.

## Done when
A serde-free line-sink is shared with caller-chosen target/prefix and unchanged bytes, or a
validated skip with the one-line reason is recorded.
