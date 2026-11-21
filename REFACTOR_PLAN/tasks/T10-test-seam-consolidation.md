# T10 — Consolidate shared test seams and includes

## Goal / pointers
- `docs/09-crate-split-turnaround-research.md` §F29 (lines 349–363) —
  `tests/artifact_bridge/server.rs` (990) is included by both
  `artifact_bridge.rs:113` and `artifact_server.rs:25` via `#[path]`;
  `common/run_lock.rs` and `common/digest.rs` are included in several places;
  `tools/capture/tests/common/mod.rs` (2,021) is inlined by 8–10 gircap binaries.
- Related: §F32 (385–392) on the six girsync subdirectory harnesses and the
  link-count / recompile-per-edit trade-off; §F9 (195–199) on test-target count.
- Assumptions to validate: §A7 (63–70), §A5 (48–53), §F31 (377–383).

## Scope
- `tests/artifact_bridge/server.rs`, `tests/artifact_bridge.rs`,
  `tests/artifact_server.rs`, `tests/run_lock.rs`, `tests/common/{run_lock,digest}.rs`.
- `tools/capture/tests/common/mod.rs`, `tools/sync/tests/common/mod.rs`.
- girsync's six subdirectory `main.rs` harnesses (`why`, `sync_plan`,
  `stat_trust`, `rw_threads`, `lazy`, `hash_mode`) — only if validation shows a
  net turnaround win (§F32 warns of the trade-off).
- Locate all `#[path]`/`mod common;` sites in the current tree before editing.

## Implementation plan
1. Map the include graph and identify the pieces that are truly identical
   across binaries. Promote shared helpers to a public library surface with
   thin per-binary shims (§A7).
2. Do **not** share between the product and gircap: capture's common deliberately
   refuses to borrow from the product mock (§F29, §F40). Consolidation is within
   one program's test suite only.
3. Evaluate the girsync harness merge only against the turnaround goal: it
   reduces link count but increases recompiled test code per edit (§F32). Skip
   it with a reason if the trade-off is unfavourable.
4. Update `.config/nextest.toml` filters for renamed binaries/paths (§F31) and
   any §A5/§F39 doc tables if a leaf is added.

## Constraints / validation
- Cross-tool independence is a hard boundary (§A5, §F40) — never merge product
  and capture fixtures.
- Keep the four fake HTTP servers distinct (§F40).
- This is test infrastructure only — no production byte/behaviour change (§A6).
- Do not merge harnesses merely to lower link count if it slows the common
  edit-then-test loop (§F32); the goal is turnaround, not a smaller target count.

## Done when
The validated shared test seams are compiled once behind thin shims,
cross-program independence holds, and all affected test suites pass with
correct nextest filters.
