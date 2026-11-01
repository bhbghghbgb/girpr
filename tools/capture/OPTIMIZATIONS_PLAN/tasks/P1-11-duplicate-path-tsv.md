# P1-11 — Duplicate-path TSV tells the truth

Report: `../../OPTIMIZATIONS_REPORT.md` lines 115-131 (A4).

Dependencies: P0-01.

## Goal

`manifest::dump::paths_tsv` must emit one row per manifest entry, including
duplicate paths, with that entry's own values. Today it sorts path strings and
then uses a `HashMap<&str, &SophonChunkFile>`, so every duplicate row prints the
last entry's size/md5/chunk count/folder flag.

## Where to start

- `src/manifest/dump.rs`: `paths_tsv` (around line 226) and its tests.
- `src/manifest/mod.rs`: `decode_counts` counts `duplicate_paths`; do not change
  that counter here.

## Steps

1. Build a `Vec<&SophonChunkFile>` of all entries.
2. Sort by `(file, original index)` so equal paths have a stable, defined
   order. The original index is the entry's position in `entries`; use
   `sort_by` with `enumerate` or a tuple.
3. Render directly from each entry. Remove the `HashMap` lookup.
4. Keep the header, `\n` endings, column order, and no-tab-escaping behavior
   exactly as they are.
5. Update the doc comment to state that duplicate paths produce one row per
   entry in entry order (or path then entry order), because the sidecar is a
   per-entry document, not a path index.

## Tests

- Unit: two entries with the same path but different size/md5/chunk count/
  folder flag produce two rows with their own values.
- Unit: entries with distinct paths still sort by path.
- Unit: duplicate rows are deterministic across input order for the same set
  (except the defined entry-order tie-break).
- Flip the P0-01 characterization test.

## Acceptance

- The sidecar is truthful on malformed duplicate-path input.
- Non-duplicate output is byte-identical to today.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(manifest): render one sidecar row per entry, not per path`
2. `test(manifest): cover duplicate paths in the paths sidecar`

## Notes

- Do not change `manifest::diff::paths_tsv`; the two writers are intentionally
  separate.
- Do not add tab escaping.
- Do not reference the report or plan in code comments.
