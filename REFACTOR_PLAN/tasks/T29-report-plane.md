# T29 — Generalized report plane

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F34 (lines 501–508) — sled2redb's reuse
  of girsync's report plane constrains generalization; candidate boundary is one generalized
  report plane parameterized by order, field vocabulary and renderer. **High-risk** because
  generalization would pull clap into a shared crate and any default-order change flips
  pinned NDJSON bytes. Confidence high.
- Assumptions to validate: §A5 (57–67), §A6 (68–75), §A7 (76–80), §A2 (29–36), §A14 (129–135).

## Scope
- `tools/sync/src/report/mod.rs` (`Record`, `Report`, `OutputFormat` clap `ValueEnum`,
  default-order NDJSON rendering), `tools/sled2redb/src/lib.rs` (builds its record with
  girsync's report types), `crates/sync-test-support/src/lib.rs` (rendered-text assertions).
  Locate before editing.

## Implementation plan
1. Validate §F34: confirm sled2redb reuses girsync's report plane and that `OutputFormat`
   is a clap `ValueEnum` in girsync.
2. If attempted, factor a report-plane core parameterized by order/field vocabulary/renderer
   while keeping clap's `ValueEnum` in each program (do not pull clap into the shared crate).
3. Migrate callers directly (§A2); move through §A14 seams.
4. If default-order change is unavoidable, stop — this breaks pinned bytes (§A5).

## Constraints / validation
- Default-order change flips pinned NDJSON bytes compared as rendered text (§F34, §A5);
   key order, casing and `because=` (not `why=`) must stay identical.
- Do not pull clap into a shared crate (§F34). Exit/summary parsing must stay valid (§A7).
- Report planes cannot be verbatim merged (§A6); only mechanics shared.

## Done when
The parameterized core exists with clap local and NDJSON bytes byte-identical, or a
validated skip with the one-line reason is recorded.
