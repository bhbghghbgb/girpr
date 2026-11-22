# T08 — `girpr-chunk-verify` leaf

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F11 (lines 262–268) — a `file ↔ scan`
  cycle blocks extracting chunk verification; candidate boundary is a leaf owning the
  verdict/remap types plus `verify_in_place`, after `Mode`/`resolve_mode` move first.
- Bottleneck context: §F16 (317–323) — large files (`inplace.rs`, `temp.rs`, `post.rs`,
  `repair/mod.rs`) hold the decision/execution graph; follow F11/F12 boundaries.
- Assumptions to validate: §A10 (98–105), §A12 (114–121), §A13 (122–128), §A5 (57–67).

## Scope
- New leaf (proposed `girpr-chunk-verify`). Sources: `src/repair/scan.rs` (verdicts,
  remap, `source_ok_by_chunk`/`source_ok_by_range`) and `src/repair/file/{mod,inplace}.rs`
  (`verify_in_place`, `ScanReport`, `Verdict`). Depends on T07's location for `Mode`.
  Locate before editing.

## Implementation plan
1. Validate §F11/§F16: confirm the `file ↔ scan` import cycle and that T07 has landed;
   confirm `Verdict::modified` + coarse-MTIME guard is a correctness proof (§A13).
2. Move `Mode`/`resolve_mode` to the neutral leaf first (T07), then define
   `girpr-chunk-verify` owning verdict/remap types and `verify_in_place`.
3. Break the cycle by having both `file` and `scan` depend on the leaf; do not introduce a
   new edge back. Migrate callers directly (§A2).
4. Deliver in commits: leaf + cycle break, caller migrations, doc seams.

## Constraints / validation
- `Verdict::modified` plus the coarse-MTIME guard is a correctness argument — changing it
  can make the writer skip a file whose bytes changed (§A13, §N2).
- The `#[cfg(test)]` halves of these files are much of the suite (§A10); moving tests
  changes fully-qualified nextest filters — update `.config/nextest.toml` (§A14).
- No published byte/key/flag change (§A5). Do not absorb `_tmp` resume (N2).

## Done when
Chunk verification is a leaf, the `file ↔ scan` cycle is gone, the correctness proof is
unchanged, and all moved-test filters are updated.
