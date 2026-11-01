# P1-10 — Explicit folder policy in diff and selection

Report: `../../OPTIMIZATIONS_REPORT.md` lines 97-113 (A3).

Dependencies: P0-02.

## Goal

Stop treating a folder manifest entry as a zero-byte file candidate. The report
offers two policies; this task chooses **exclude folders from the candidate
pool while keeping them in the diff documents**.

## Where to start

- `src/manifest/diff.rs`: `PathRow` (around line 125), `diff_field` (around
  line 282), `score`, `by_cost`, `paths_tsv`.
- `src/select/mod.rs`: `candidates` (around line 393),
  `candidates_document` (around line 874).
- `src/manifest/proto.rs` / `src/manifest/mod.rs`: `SophonChunkFile.is_folder`.
- `src/assemble/assembly.rs`: the no-old-side / no-chunks paths, to confirm a
  folder can no longer reach admission.

## Steps

1. Add `is_folder: bool` to `PathRow`, populated from the manifest entry on
   both the new-side (common/added) and old-side (removed) loops.
2. Keep folder rows in `rows`, `counts`, `paths.diff.json`, and `paths.tsv` so
   aggregate counts and the sidecar remain truthful. Do not hide them from the
   diff.
3. Do not add a folder to `scores` (or `by_cost`): a folder has no patch cost
   and must not appear as a scored candidate. If a common folder currently
   gets a score, skip the `score(...)` call and push no `PathScore`; keep the
   row's state.
4. In `select::candidates`, skip `row.is_folder`. In `candidates_document`,
   `by_cost` already excludes unscored rows, so folders disappear from the
   scoring table; document that the table is common scored paths only.
5. Add a debug/assertion or test that no `SizedCandidate` has `is_folder`.
   If `PathRow` is used elsewhere, keep the new field `pub` and serde-serialized
   only if the document already carries it; otherwise keep it internal.
6. Do not change `decode_counts` (P1-12 owns the empty-chunk rule).

## Tests

- Diff: a manifest pair with a folder entry and a normal change keeps the
  folder in `rows`/counts and does not put it in `scores`.
- Select: `candidates` returns no candidate for the folder path.
- Selection: a run with only a folder change does not admit a capture for it.
- Existing tests with `is_folder: false` rows are unchanged.
- Flip the P0-02 characterization test for folders.

## Acceptance

- A folder never becomes a file capture.
- Folder rows remain visible in diff counts/sidecars.
- No selection or assembly behavior changes for non-folder paths.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(diff): carry the folder flag on path rows`
2. `fix(select): exclude folder entries from the candidate pool`
3. `test(diff,select): cover folder entries in diffs and selection`

## Notes

- This is the lower-risk of the report's two policies. Do not implement folder
  materialization end-to-end here.
- If folder counts are asserted somewhere as "files", keep the existing
  semantics and only stop admission; do not rename counters.
- Do not reference the report or plan in code comments.
