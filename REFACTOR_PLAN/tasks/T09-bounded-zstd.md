# T09 — Bounded zstd decode primitive

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F28 (lines 373–377) — bounded
  zstd decode + digest of decompressed bytes in two shapes.
- Assumptions to validate: §A3 (29–35), §A7 (62–67), §A2 (21–27).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§N10 — the paired
  `zstd_measure*` loops stay duplicated).

## Scope
- Candidate home: `girpr-hash` (T08) or a dedicated bounded-decode leaf.
- Report names: `src/sophon/stream.rs` (streaming into a hashing sink without
  holding the blob), `tools/capture/src/hash/mod.rs` (buffered with a ceiling,
  emits hex). Locate before editing.

## Implementation plan
1. Validate §F28's medium-confidence claim: the only plausible shared piece is
   the low-level **safe bounded decode**; the async streaming shape and the
   owned buffered shape are genuine and should stay separate.
2. If a genuinely shared bounded-decode primitive exists, extract it into a leaf
   with a caller-supplied ceiling and a caller-supplied sink (e.g. the hash sink
   from T08). Keep the result handling local to each tool.
3. Keep the `zstd_measure` / `zstd_measure_from` pair duplicated for their
   distinct error messages (§N10); do not "unify" them.
4. If the two shapes share nothing beyond the `zstd` API itself, stop and mark
   `skipped` with the §F28 reason — a forced shared wrapper would add no value.

## Constraints / validation
- Decompressed-byte digests and ceilings must stay identical (§A3).
- Do not hold a whole blob in the streaming product path (§F28).
- A new gircap leaf dep requires §A2 doc tables to be updated.

## Done when
A genuinely shared bounded-decode primitive is extracted (or the task is
justifiably skipped), and all zstd/digest tests pass with unchanged bytes.
