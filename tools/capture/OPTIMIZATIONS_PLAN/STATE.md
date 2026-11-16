# Checklist state

Committed state for the temporary optimization work. The orchestrator owns this
file; implementers and reviewers must not edit it. Every change to this file is
its own commit (`chore(plan): ...`), separate from code commits.

Status vocabulary: `todo`, `in_progress`, `pending_review`, `completed`,
`reverted`, `blocked`. Review batches: `pending`, `in_progress`, `done`.
See `ORCHESTRATION.md` for the transition rules.

## Tasks

| ID | Title | Status | Commits | Notes |
|---|---|---|---|---|
| P0-01 | Capture/close-out characterization tests | completed | d1261aa6 310f28f9 | |
| P0-02 | Data-edge and bridge characterization tests | completed | b28655dc 00300863 | |
| P0-03 | Verify-surface characterization tests | completed | 811ddc9f 5a00c3fb | |
| P1-01 | Refuse identical resolved tags (A1) | completed | 82ca495f 0af07de5 | |
| P1-02 | Carry encoding, not a `.gz` suffix (A2) | completed | 0360ca2a 82d03428 | |
| P1-03 | Keep chunk URL suffixes (A5) | completed | 4cbafd88 25f7da7b | |
| P1-04 | A14 scans `game/` on disk (A6) | completed | c7dcbff9 35a94be2 | |
| P1-05 | A11 checks the declared side size (A7) | completed | 12ad7888 d898d57e | |
| P1-06 | Independently check published checksums (A8) | completed | 4dea01a6 bb9a7423 | |
| P1-07 | Schema and path validation (A9) | in_progress | | |
| P1-08 | Cross-check patchmap plan and index (A10) | todo | | |
| P1-09 | `list` descends past non-bundles (A11) | todo | | |
| P1-10 | Explicit folder policy (A3) | todo | | |
| P1-11 | Duplicate-path TSV truth (A4) | todo | | |
| P1-12 | `decode_counts` empty-chunk non-folders (B3) | todo | | |
| P1-13 | Record repeated response headers (A12) | todo | | |
| P1-14 | Reserve error-body budget before reading (A13) | todo | | |
| P1-15 | Deterministic rejection cap (A14) | todo | | |
| P1-16 | Blob diagnostic fixes (A15, A16) | todo | | |
| P1-17 | Numeric and tag-length honesty (A17, D4) | todo | | |
| P1-18 | Summary log and escaped keys (A18, A19) | todo | | |
| P2-01 | Separate scoring rank from admission rank (B1) | todo | | |
| P2-02 | Reconcile `budget_cost` semantics (B2) | todo | | |
| P2-03 | Reuse or remove `is_a_real_upgrade` (B4) | todo | | |
| P2-04 | Capture-id uniqueness invariant (B5) | todo | | |
| P3-01 | Verify once, share the bridge fixture (C1, C8, E1-E3) | todo | | |
| P3-02 | Bridge server correctness and locking (C8, E4-E6) | todo | | |
| P3-04 | Reuse retention per pair (C2) | todo | | |
| P3-05 | Avoid assembly copy-through (C6) | todo | | |
| P3-06 | Build `ChunksByFile` once and use `HashMap` (C5) | todo | | |
| P3-07 | Early A16 exit and larger digest buffer (C3, C4) | todo | | |
| P3-08 | Symlink containment and termination (C9) | todo | | |
| P3-09 | One definition of A12 (C7) | todo | | |
| P4-01 | One acceptable-state table (D1) | todo | | |
| P4-02 | Remove remaining dead fields (D2) | todo | | |
| P4-03 | Parse `getBuild` once (D3) | todo | | |
| P4-04 | Allocation cleanups (C10, C11, C12) | todo | | |

## Review batches

| Batch | Tasks | Status | Review commits | Notes |
|---|---|---|---|---|
| RC-1 | P0-01..P0-03 | done | | |
| RC-2 | P1-01..P1-06 | done | | |
| RC-3 | P1-07..P1-12 | pending | | |
| RC-4 | P1-13..P1-18 | pending | | |
| RC-5 | P2-01..P2-04 | pending | | |
| RC-6 | P3-01..P3-02 | pending | | |
| RC-7 | P3-04..P3-09 | pending | | |
| RC-8 | P4-01..P4-04 | pending | | |

## Rework

Tasks the reviewer reverted, with the reason and the suggested correction. Pick
these before the follow-up phase.

| ID | Original task | Reason | Status |
|---|---|---|---|
| | | | |

## Follow-ups

Non-blocking items recorded by reviewers or implementers. Do not implement
until the main tasks and rework are done.

| ID | Source | Area | Description | Status |
|---|---|---|---|---|
| F-01 | P0-02 | manifest diff counts (`diff/paths.diff.json`) | `Counts::folders` is serialized but never incremented; folder rows are tallied as added/common, so the published `folders` count is always 0 even when folders exist. Increment it in `diff_field` or remove the dead field. Group with folder-policy / dead-fields work. | todo |
| F-02 | P1-02 | test comparison helper (`tests/bundle_diff.rs`) | `normalize` (~L423) gunzips any path ending in `.gz` — the heuristic removed from production. Would fail comparing a non-gzip captured `.gz` asset; apply the same document-aware rule. | todo |
| F-03 | P1-05 | diagnostics wording (`src/assemble/ranges.rs`) | `Layout::of` (~L226) error says "the manifest declares" even on the verify path where the figure comes from a patchmap side. Tighten the wording for that caller. | todo |

## Follow-up batches

Filled in by the follow-up batching session, then executed batch by batch.

| Batch | Follow-ups | Status | Notes |
|---|---|---|---|
| | | | |

## Notes

- Review checkpoints and batch sizes are defined in `INDEX.md`.
- `completed` means the task and its review batch are done.
- `reverted` means the original work was undone and a rework row was added.
