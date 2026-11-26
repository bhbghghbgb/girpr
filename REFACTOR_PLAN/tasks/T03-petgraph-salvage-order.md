# T03 — Salvage order → petgraph

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §R6 (lines 212–227) — the conservative
  two-direction salvage-order heuristic defers the real optimum: a topological order
  over "chunk j's write clobbers chunk i's source", breaking cycles by downloading.
  The proposed library is `petgraph` with `DiGraph` + `toposort` + explicit cycles.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28). This is the
  report's **lower-confidence, higher-risk** finding (§R6 lines 226–227); keep items:
  `guidance/DO-NOT-TOUCH.md` (non-findings at lines 824–826).

## Scope
- `src/repair/reuse.rs` — the heuristic (`:50–56`, `:83–172`) and
  `salvage_order_source_boundary_cases` (`:228`).
- Consumers: `src/repair/scan.rs:260`, `src/repair/file/inplace.rs:519–520`,
  `src/repair/file/temp.rs`; the `est_download_bytes`/`chunks_reused` pins in those
  files and in `tests/repair_scan.rs`. Root `Cargo.toml`.

## Implementation plan
1. **Validation gate first.** Re-derive the current rule's safety argument from the code.
   If it cannot be re-established in the new formulation, **stop** — do not change
   writer call sites. A validated skip is the expected outcome if confidence stays low.
2. If the gate passes, introduce a narrow leaf crate owning the graph decision and
   cover it with property tests before touching any writer. Build one node per bad
   chunk, an edge when `W_j ∩ R_i` (j's write clobbers i's source), topologically sort,
   break cycles by downloading, and keep the existing rule as tie-break/fallback.
3. Rewire `salvage_order_refs` and the writer/scan consumers to the leaf; re-baseline
   the estimate/reuse-count pins in `scan.rs`, `file/temp.rs`, `file/inplace.rs` and
   `tests/repair_scan.rs`.
4. Deliver in multiple commits if useful: leaf with property tests, consumer rewire,
   re-baselined pins.

## Constraints / validation
- Correctness is byte-level: reuse counts changing is acceptable, **writing bad bytes
  is not** (§R6 lines 226–227). Never weaken a safety check to make the graph fit.
- Do not change the surrounding `src/repair/file/**` byte movement beyond the order
  source (report non-findings, lines 824–826).
- Property tests must cover cycles and multiple overlapping clobber directions before
  any writer consumes the new order.

## Done when
The graph decision is proven safe by property tests, consumers use it, and all
estimate/reuse pins are re-baselined with the full repair tests passing. If the safety
argument cannot be re-established, **stop** and record `skipped` with a one-line reason.
