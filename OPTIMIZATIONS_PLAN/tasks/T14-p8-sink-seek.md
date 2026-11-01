# T14 - P8: remove the per-piece seek in the chunk sink

Report: `../../OPTIMIZATIONS_REPORT.md` lines 459-478 (P8).

Dependencies: none.

## Goal

`repair::file::temp::ChunkFileSink::write` seeks to `offset + written` before
every decoded piece, then writes. The sink is the only user of the file handle
during a streamed chunk and it writes sequentially, so the seek is redundant
after construction and after `restart`. Removing it removes one syscall per
piece; buffering is optional and is not required by this task.

## Where to start

- `src/repair/file/temp.rs`: `ChunkFileSink` (around line 341), `new`,
  `Write::write`, `Write::flush`, `ChunkSink::restart`; the construction in
  `write_downloaded_chunk` (around line 432).
- The streamed-chunk tests in `src/sophon/stream.rs` and the temp tests.

## Steps

1. Make `ChunkFileSink::new` seek the file to `offset` and return
   `std::io::Result<Self>` (or an `anyhow::Result` if the caller wants context).
   If the seek fails, the caller must fail the chunk before streaming.
2. Remove the `seek` from `Write::write`. Write at the current position, then
   advance `written` only after `write_all` succeeds. `flush` stays a flush.
3. Keep the seek in `restart`: a retry starts the chunk over at `offset` and
   resets `written` to zero. This is the one place the cursor must be
   repositioned.
4. Update the construction site to handle the new `Result` with context naming
   the chunk/file. Do not silently fall back to the old per-write seek.
5. Do not add a `BufWriter` in this task. The report says buffering is optional;
   it complicates `restart` (buffered bytes must be discarded) and the digest
   finalization. Removing the seek is the required gain.
6. Assert the cursor invariant in a debug-only way if it is cheap: after
   construction, `stream_position()` may be checked once in a unit test, not on
   every write.

## Tests

- The existing streamed-chunk tests must pass; they exercise the sink through
  `write_downloaded_chunk`.
- Add a focused test if the sink can be constructed with a temporary file and a
  minimal `FileCtx`: writing two pieces lands at `offset..offset+len` and
  `written()` reports the total. A retry calls `restart` and overwrites from
  `offset`, not appends.
- Do not assert that no syscall happened; that is an implementation detail.
  Assert the bytes and the `written` count.

## Acceptance

- `Write::write` performs no seek.
- A streamed chunk with a retry still overwrites from its offset.
- The write ceiling and `written` accounting are unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `perf(temp): seek once per chunk instead of once per piece`
2. `test(temp): pin sink offsets and restart behavior`

## Notes

- The exclusive borrow already guarantees no other writer moves the cursor
  during the stream. If that changes, the sink must own the seek rule again.
- Do not change the `ChunkSink` trait in `sophon`; `restart` is the existing
  contract.
