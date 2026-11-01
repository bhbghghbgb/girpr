# T35 - D9: replace parameter bags with structs

Report: `../../OPTIMIZATIONS_REPORT.md` lines 794-811 (D9).

Dependencies: T13 (temp signature), T29 (retry/fetch signatures). Do not
combine with T34's decomposition.

## Goal

Four sites carry `#[allow(clippy::too_many_arguments)]`:

- `report::report_record`;
- `file::temp::try_reuse_slice`;
- `sophon::stream_chunk_to` / `try_stream_chunk`;
- `file::temp::write_downloaded_chunk` (the streaming download bag, even if the
  allow has moved).

Give each related group a named struct, document it, and remove the allow.
Fewer positional mistakes, easier extension.

## Where to start

- `src/report/emit.rs`: `report_record` (around line 74), its caller in
  `repair::plan_phase`, and `src/report/tests.rs`.
- `src/repair/file/temp.rs`: `try_reuse_slice` (around line 239),
  `write_downloaded_chunk` (around line 407), their callers in the per-chunk
  loop.
- `src/sophon/stream.rs`: `stream_chunk_to` (around line 306),
  `try_stream_chunk` (around line 357), `ChunkSink`, the retry policy, callers
  in `temp.rs`.

## Steps

1. `report_record`: define `ReportProvenance<'a>` (or a similar name) with the
   local/latest versions, biz, `GameConfig`, `GameBranchPackage`,
   `ChunkBuild`, and audio langs. Take one `&ReportProvenance<'_>`. Keep the
   field order and output identical. Update the caller and tests.
2. `try_reuse_slice`: define a struct for the per-file scratch and per-chunk
   inputs that T13 left, for example `ReuseCall<'a>` with `out`, `tmp_len`,
   `src_handle`, `src`, `c`, `end`, and the summary. Take `&mut ReuseCall` or
   a small set of `&mut` fields if that is clearer. Do not put the summary
   behind a second lock.
3. `write_downloaded_chunk`: define `DownloadedChunk<'a>` for the chunk, sink,
   URL/prefix, summary, and attempt data. Take one struct.
4. `stream_chunk_to` / `try_stream_chunk`: define `ChunkFetch<'a>` (the fetch
   context: client, URL, chunk id, expected digest, sink, sleep, attempts).
   `try_stream_chunk` can take `&ChunkFetch` plus the attempt number. The
   report notes this makes it natural to carry the retry policy; do not move
   T29's policy into it unless that is a small, obvious win.
5. Remove the `#[allow(clippy::too_many_arguments)]` attributes at the converted
   sites. If another function still needs one, leave it and record it as a
   follow-up rather than widening this task.
6. Keep lifetimes explicit and minimal. The structs borrow; do not clone the
   chunk, the sink, or the summary to satisfy a struct.
7. Do not change any retry counts, timeouts, or byte accounting. This is a
   signature refactor.

## Tests

- The report tests, temp reuse/download tests, and sophon stream tests all pass
  unchanged except for construction.
- Add one test per struct only if it has a nontrivial constructor; otherwise
  the existing behavior tests are enough.
- `cargo clippy -p girpr --all-targets` (or the repo's clippy command) must not
  report the removed allows.
- `cargo test -p girpr` passes.

## Acceptance

- The four listed sites take named structs, not positional bags.
- Their `too_many_arguments` allows are gone.
- Output, retries, and accounting are unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(report): group the report provenance`
2. `refactor(temp): group the reuse and download inputs`
3. `refactor(sophon): group the chunk fetch context`
4. `test: update call sites for the parameter structs`

## Notes

- A params struct is not an excuse to make every field public; keep fields
  private unless the caller constructs them directly.
- T29's retry helper may already have changed these signatures. Adapt to the
  actual shape; do not reintroduce the old bag.
