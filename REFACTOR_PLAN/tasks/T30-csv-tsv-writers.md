# T30 — TSV writers → csv

## Goal / pointers
- `docs/08-third-party-delegation-findings.md` §CAP-7 (lines 738–750) — two hand-written
  TSV writers become `csv::Writer` with
  `WriterBuilder::delimiter(b'\t').has_headers(true)`.
- Validate assumptions first: §A1 (15–16), §A4 (22–23), §A5 (26–28). Keep items:
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/src/manifest/dump.rs` — `paths_tsv` (`:209–243`) and tests `:553–691`.
- `tools/capture/src/manifest/diff.rs` — second `paths_tsv` (`:656–713`), its doc
  (`:670–673`) and tests `:1499–1533`.
- `tools/capture/Cargo.toml`. Locate both writers before editing; the list above is a hint.

## Implementation plan
1. Validate the two writers produce the same header, row shape and sort/tie-break rules;
   note the existing doc admits neither escapes a tab (§CAP-7 lines 740–741).
2. Define small row structs and serialize through `csv::Writer` configured with a tab
   delimiter and headers. Keep the existing sort and tie-break logic in the callers.
3. Keep the pinned header, LF-only newlines, row content and column count where they are
   asserted; quoting of tab-containing fields is an allowed beneficial change
   (§CAP-7 line 748).
4. Confirm no current fixture path contains a tab (§CAP-7 line 750), regenerate the two
   test suites' expectations if needed, and verify the tool still builds.

## Constraints / validation
- LF-only newlines must survive — configure the writer accordingly rather than relying
  on platform defaults.
- Do not change the sort/tie-break semantics or which rows are emitted.
- `csv 1.4.0` is not currently locked; verify a resolved build.

## Done when
Both hand-written writers use `csv`, header/LF/column-count pins pass, and the tool
builds with tab-containing fields correctly quoted. If a premise is stale or a keeper
would change, **stop** and record `skipped`.
