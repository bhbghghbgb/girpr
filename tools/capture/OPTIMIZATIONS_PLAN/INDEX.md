# Optimization plan index

This directory is a temporary execution plan for `OPTIMIZATIONS_REPORT.md`.
It is deleted, together with the report, when the work is finished.

- `INDEX.md` - this file: task map, dependencies, review checkpoints.
- `HOW-TO.md` - rules shared by implementers, reviewers, and the orchestrator.
- `ORCHESTRATION.md` - the orchestrator protocol and session prompt templates.
- `STATE.md` - the committed checklist/state file. The orchestrator owns it.
- `tasks/*.md` - one implementation task per file.

## Report ordering

Tasks follow the report's recommended ordering (`OPTIMIZATIONS_REPORT.md`
lines 826-924): Phase 0 (pin the contract), Phase 1 (correctness and
verification), Phase 2 (selection/accounting), Phase 3 (runtime/resources),
Phase 4 (clean code). Within a phase the order is the report's order.

## Task map

| Task | Report | Depends on | Review batch |
|---|---|---|---|
| P0-01 Capture/close-out characterization tests | 833-850 | - | RC-1 |
| P0-02 Data-edge and bridge characterization tests | 833-850 | - | RC-1 |
| P0-03 Verify-surface characterization tests | 833-850 | - | RC-1 |
| P1-01 Refuse identical resolved tags (A1) | 56-74, 446-457 | P0-01 | RC-2 |
| P1-02 Carry encoding, not a `.gz` suffix (A2) | 76-95 | P0-01 | RC-2 |
| P1-03 Keep chunk URL suffixes (A5) | 133-148 | P0-02 | RC-2 |
| P1-04 A14 scans `game/` on disk (A6) | 150-166 | P0-01 | RC-2 |
| P1-05 A11 checks the declared side size (A7) | 168-187 | P0-01 | RC-2 |
| P1-06 Independently check published checksums (A8) | 189-205, 481-503 | P0-01 | RC-2 |
| P1-07 Schema and path validation (A9) | 207-227 | P0-03 | RC-3 |
| P1-08 Cross-check patchmap plan and index (A10) | 229-246 | P0-03 | RC-3 |
| P1-09 `list` descends past non-bundles (A11) | 248-262 | P0-03 | RC-3 |
| P1-10 Explicit folder policy (A3) | 97-113 | P0-02 | RC-3 |
| P1-11 Duplicate-path TSV truth (A4) | 115-131 | P0-01 | RC-3 |
| P1-12 `decode_counts` empty-chunk non-folders (B3) | 431-444 | P0-02 | RC-3 |
| P1-13 Record repeated response headers (A12) | 264-276 | P0-02 | RC-4 |
| P1-14 Reserve error-body budget before reading (A13) | 278-294 | P0-02 | RC-4 |
| P1-15 Deterministic rejection cap (A14) | 296-312 | P0-02 | RC-4 |
| P1-16 Blob diagnostic fixes (A15, A16) | 314-340 | - | RC-4 |
| P1-17 Numeric and tag-length honesty (A17, D4) | 342-354, 723-734 | - | RC-4 |
| P1-18 Summary log and escaped keys (A18, A19) | 356-381, 744-750 | - | RC-4 |
| P2-01 Separate scoring rank from admission rank (B1) | 387-410 | P1-10, P1-12 | RC-5 |
| P2-02 Reconcile `budget_cost` semantics (B2) | 412-429 | P0-02, P2-01 | RC-5 |
| P2-03 Reuse or remove `is_a_real_upgrade` (B4) | 446-457 | P1-01, P2-01 | RC-5 |
| P2-04 Capture-id uniqueness invariant (B5) | 459-475 | P2-02 | RC-5 |
| P3-01 Verify once, share the bridge fixture (C1, C8, E1-E3) | 481-503, 590-619, 754-784 | P2-04 | RC-6 |
| P3-02 Bridge server correctness and locking (C8, E4-E6) | 590-619, 754-784 | P0-02 | RC-6 |
| P3-04 Reuse retention per pair (C2) | 505-517 | - | RC-7 |
| P3-05 Avoid assembly copy-through (C6) | 558-574 | - | RC-7 |
| P3-06 Build `ChunksByFile` once and use `HashMap` (C5) | 544-556 | - | RC-7 |
| P3-07 Early A16 exit and larger digest buffer (C3, C4) | 519-542 | P1-06 | RC-7 |
| P3-08 Symlink containment and termination (C9) | 621-635 | P1-09, P3-01 | RC-7 |
| P3-09 One definition of A12 (C7) | 576-588 | - | RC-7 |
| P4-01 One acceptable-state table (D1) | 682-693 | P1-10 | RC-8 |
| P4-02 Remove remaining dead fields (D2) | 695-708 | P2-03, P3-02, P4-01 | RC-8 |
| P4-03 Parse `getBuild` once (D3) | 710-721 | - | RC-8 |
| P4-04 Allocation cleanups (C10, C11, C12) | 637-676, 744-750 | - | RC-8 |

`D5` (736-742) and section `I` (952-967) are notes, not tasks. Do not merge
the two TSV writers and do not touch the deliberate designs listed in section I.

## Review checkpoints

A checkpoint is reached when every task in the batch is `pending_review`.

| Batch | Tasks | Why this size |
|---|---|---|
| RC-1 | P0-01..P0-03 | Tests only; one contract-pinning review. |
| RC-2 | P1-01..P1-06 | The six highest-value correctness fixes; one coherent selfcheck/verify review. |
| RC-3 | P1-07..P1-12 | Verify-surface hardening plus data edge cases. |
| RC-4 | P1-13..P1-18 | Diagnostic/fidelity fixes, each small but in different modules. |
| RC-5 | P2-01..P2-04 | Selection semantics; must be reviewed together because the rank/budget decisions interact. |
| RC-6 | P3-01..P3-02 | Bridge restructure and bridge-server fixes; both are structural and need careful review. |
| RC-7 | P3-04..P3-09 | Independent performance/resource changes. |
| RC-8 | P4-01..P4-04 | Polish and dead-code removal. |

After RC-8, the follow-up phase starts (see `ORCHESTRATION.md`).
