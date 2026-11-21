# T16 — gircap: blobs + manifest crates

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F25 (lines 310–320) — `blobs/*`
  ~4,851 lines (3,359 production; `zstd`), `manifest/*` ~4,307 lines (3,778
  production; `prost`, `zstd`; `diff.rs` 1546, near-leaf `proto.rs` 376).
- §F27 (327–333) — heavy deps group by subsystem but the lib facade defeats
  isolation (§A2).
- Assumptions to validate: §A2 (27–33), §A5 (48–53), §A4 (40–46), §A6 (55–61).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (`zstd_measure` §A13).

## Scope
- `tools/capture/src/blobs/**`, `tools/capture/src/manifest/**`.
- The `zstd`/`prost` deps; `girpr-proto` already exists (§A3) — reuse it rather
   than duplicating wire types.
- The `zstd_measure` / `zstd_measure_from` pair in `tools/capture/src/hash/mod.rs:518-523`
   stays duplicated (§A13, §F40).
- Locate all edges in the current tree before editing.

## Implementation plan
1. Validate that blobs and manifest are acyclic or find their SCC, and that each
   has a direct high-level consumer (§A2). Decide crate grouping by edges.
2. Move the subsystems behind crate boundaries owning `zstd`/`prost`, and make
   gircap depend on them directly so a change elsewhere leaves them cached.
3. Reuse `girpr-proto` for wire types; keep protobuf field spelling and artifact
   bytes identical (§A6). Keep the `zstd_measure` pair duplicated (§A13).
4. Promote test-only seams via §A7; honor §A5 and update §A5/§F39 doc tables in
   the same work; update nextest filters (§F31).

## Constraints / validation
- §A2 payoff must be real (no facade-only split).
- No byte/behaviour change in blobs/manifest artifacts (§A6).
- Do not merge the `zstd_measure` pair or pull other `DO-NOT-TOUCH` items in
   (§A13, §F40).
- No girpr dependency or binary invocation (§A5).

## Done when
Blobs and manifest are logic-free crates consumed directly by gircap, zstd/prost
stay cached, proto/artifact bytes are unchanged, and all blobs/manifest tests
pass.
