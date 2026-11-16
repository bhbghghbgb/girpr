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
| P1-07 | Schema and path validation (A9) | completed | 6bed8f7d 646cd70b bf4b3592 | |
| P1-08 | Cross-check patchmap plan and index (A10) | completed | 7a310d1b 216f8959 | |
| P1-09 | `list` descends past non-bundles (A11) | completed | 1536bd72 956f1422 | |
| P1-10 | Explicit folder policy (A3) | completed | 8a44db22 988d4b9c 36ae4d8f | |
| P1-11 | Duplicate-path TSV truth (A4) | completed | 3bb0786b 0aeb530f | |
| P1-12 | `decode_counts` empty-chunk non-folders (B3) | completed | 45a3f8e2 886a650b | |
| P1-13 | Record repeated response headers (A12) | completed | 424e385d 34cb81b4 | |
| P1-14 | Reserve error-body budget before reading (A13) | completed | 831c6601 a190e31d | |
| P1-15 | Deterministic rejection cap (A14) | completed | 00fe91aa d4b97a04 | |
| P1-16 | Blob diagnostic fixes (A15, A16) | completed | 6021ebc5 7fb61599 0b8b45e7 | |
| P1-17 | Numeric and tag-length honesty (A17, D4) | completed | 38ff3525 9bbccabc cabe7dfc | |
| P1-18 | Summary log and escaped keys (A18, A19) | completed | 4686a977 24d6b608 d58b1c73 | |
| P2-01 | Separate scoring rank from admission rank (B1) | completed | d09de8ed b3dc98f8 2ef26ef5 | |
| P2-02 | Reconcile `budget_cost` semantics (B2) | completed | 77c98275 578d5301 cf4e1c78 | |
| P2-03 | Reuse or remove `is_a_real_upgrade` (B4) | completed | 0debdd91 3e7c981f | |
| P2-04 | Capture-id uniqueness invariant (B5) | completed | 2588220d 7ceaeb32 | |
| P3-01 | Verify once, share the bridge fixture (C1, C8, E1-E3) | completed | c4224d3e a2cfdfb6 | |
| P3-02 | Bridge server correctness and locking (C8, E4-E6) | completed | 791f0b24 f6184c5a f8520b74 | |
| P3-04 | Reuse retention per pair (C2) | completed | e46b6657 22939ee2 | |
| P3-05 | Avoid assembly copy-through (C6) | completed | a3b81a8e 9ca856c0 efc3c694 | |
| P3-06 | Build `ChunksByFile` once and use `HashMap` (C5) | completed | 77ab4a58 3bfa3d46 602ef311 | |
| P3-07 | Early A16 exit and larger digest buffer (C3, C4) | completed | 2db0cd0e 3b5a1e4f 02aff616 | |
| P3-08 | Symlink containment and termination (C9) | completed | 635422a0 723ba9c9 1d8dab54 | |
| P3-09 | One definition of A12 (C7) | completed | 6f288b25 d2fc7add b78e4c67 | |
| P4-01 | One acceptable-state table (D1) | todo | | |
| P4-02 | Remove remaining dead fields (D2) | todo | | |
| P4-03 | Parse `getBuild` once (D3) | todo | | |
| P4-04 | Allocation cleanups (C10, C11, C12) | todo | | |

## Review batches

| Batch | Tasks | Status | Review commits | Notes |
|---|---|---|---|---|
| RC-1 | P0-01..P0-03 | done | | |
| RC-2 | P1-01..P1-06 | done | | |
| RC-3 | P1-07..P1-12 | done | | |
| RC-4 | P1-13..P1-18 | done | | |
| RC-5 | P2-01..P2-04 | done | | |
| RC-6 | P3-01..P3-02 | done | | |
| RC-7 | P3-04..P3-09 | done | | |
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
| F-04 | P2-04 | select ordinals (`src/select/mod.rs`) | A forced path rejected in phase 0 can share its `<class>` capture id with a different, cheaper phase-2 pick of the same class; the debug guard only detects this, release still emits a misleading rejection record. Fix ordinal assignment so ids are unique. | todo |
| F-05 | P2-02 | candidates doc comment (`src/select/mod.rs`) | The `candidates` doc comment (~L405-409) still describes `by_cost`'s `total_cost`+moved/reused order and says one-sided paths are placed "by their own size"; the comparator is now `budget_cost`/field/path and `added` charges `download_bytes`. Update the comment. | todo |
| F-06 | P3-02 | bridge HTTP layer (`tests/artifact_bridge/server.rs`) | Keep-alive: accept multiple requests per connection with `Content-Length`/framing and send `Connection: keep-alive` unless the client closes. Currently `Connection: close` per response; deliberately skipped under the task's optional allowance. | todo |
| F-07 | P3-02 | bridge `ManifestEntry` (`tests/artifact_bridge/server.rs`) | Remove the now-dead `id` field (set at insert, never read; the map key already carries it). Trivial pre-existing cleanup. | todo |
| F-08 | P3-05 | assemble hardening (`src/assemble/...`, `patch_flow`) | `patch_flow` is now `pub(crate)` and a direct call with a gapped layout zero-fills (old code errored on the length check). Unreachable via `assemble`, but a future in-crate caller could get a silently zero-filled file. Re-validate A11 inside `patch_flow` or keep it private behind a validated wrapper. | todo |
| F-09 | P3-06 | docs/index cleanup (`src/blobs/mod.rs`) | `ChunkIndex` docs claim blobs are "ordered by id" (justifying binary search), but `build` does not sort; correctness relies on the fallback scan. Fix the doc or sort. Pre-existing, outside P3-06. | todo |
| F-10 | P3-08 | test/CI hardening (`emit::index::walk`, `verify::list`) | Six filesystem symlink tests self-skip on Windows; only unit-level containment coverage here. Run them in a Unix CI job or add an injectable canonicalize seam. | todo |
| F-11 | P3-08 | verify/list hardening (`src/verify/list.rs`, `child_dirs`) | `child_dirs` is still `pub` and uses `std::fs::metadata`; enforcement lives only in `list_with`, so a future non-test caller could bypass containment. Make it private or pass the canonical root into it. | todo |
| F-12 | P3-08 | future containment task (`src/verify/bundle.rs`) | `MANIFEST.json` is stat'ed with `metadata` (follows links) and read one-shot, so `list` can surface bytes from a symlinked manifest outside the root. Canonicalize/check the manifest path against the root before reading. Pre-existing, out of scope. | todo |

## Follow-up batches

Filled in by the follow-up batching session, then executed batch by batch.

| Batch | Follow-ups | Status | Notes |
|---|---|---|---|
| | | | |

## Notes

- Review checkpoints and batch sizes are defined in `INDEX.md`.
- `completed` means the task and its review batch are done.
- `reverted` means the original work was undone and a rework row was added.
