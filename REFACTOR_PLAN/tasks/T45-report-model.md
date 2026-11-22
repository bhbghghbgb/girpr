# T45 — `girpr-report-model` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F50 (lines 654–662) — the three report
  field models now have a much smaller common target: the `Field`/`Value` vocabulary and
  event placement. Candidate boundary is a leaf with `Field`, `Value`, `EventPlacement` and
  per-tool schema tables, leaving `report-core` as writer. Confidence medium-low.
- Assumptions to validate: §A5 (57–67), §A6 (68–75), §A2 (29–36), §A12 (114–121), §A14 (129–135).

## Scope
- New leaf (proposed `girpr-report-model`). Sources: `girpr-report` (counters + 1/0 flags),
  gircap `tools/capture/src/report/mod.rs` (`Val::{Str,Int,Bool}`, event-first order),
  girsync alphabetical NDJSON; `girpr-report-core` stays the renderer.
  Locate before editing.

## Implementation plan
1. Validate §F50: confirm the smaller common target and that report-core already owns the
   renderer.
2. Define the leaf with `Field`/`Value`/`EventPlacement` and per-tool schema tables; keep
   the recursive `Summary`/`Meter`/`StoppedBy` group (`girpr-report`) local (§A12).
3. Migrate callers directly (§A2); move through §A14 seams if gircap calls it.
4. Given medium-low confidence, a validated skip is legitimate if the vocabulary cannot be
   shared without touching pinned keys.

## Constraints / validation
- High-risk (§F50): booleans vs `1`/`0`, percent-encoding and pinned key orders are
   contracts (§A5, §A6). Do not merge report planes verbatim.
- Do not absorb `Summary`/`Meter`/`StoppedBy` (§A12) or the renderer (report-core).

## Done when
The vocabulary is shared with per-tool schema tables and byte-identical output, or a
validated skip with the one-line reason is recorded.
