# T10 — Move streaming decode/write off async workers

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §5 **NET-07** (lines 404–413) — `stream_body` writes
  through `HashingWriter<&mut ChunkFileSink<W>>` where `W` is `std::fs::File`, and
  call sites in `temp.rs` await it directly; each in-flight chunk blocks a Tokio
  worker on disk I/O (same class as S-02).
- Change suggested in the report: move decode+write to `spawn_blocking`, bridging only
  the network stream into it; or document and enforce that `stream_chunk_to` must be
  called from a blocking task.
- Report lines 139 and 404–413 explicitly pair this with S-02 / T01 — keep the two
  consistent.
- Keep items: async network path, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/fetch/src/sophon/stream.rs` (`stream_body` ~505, sink writes ~535–540).
- `src/repair/file/temp.rs` (call sites ~96–102, 510–536, 147–155).
- Locate current call sites before editing; the list above is a hint.

## Implementation plan
1. Confirm which parts of `stream_chunk_to` are genuinely blocking in the current
   tree and which call sites run on async workers.
2. Move decode and file writes to `spawn_blocking`, bridging only the network byte
   stream into the blocking task; keep the network receive async.
3. If a full offload is not bounded safely, instead document and enforce the
   blocking-context requirement at every call site rather than silently leaving work
   on async workers.
4. Coordinate with T01's blocking/I/O policy (see `STATE.md` Persistent context).
5. Deliver in one or a few commits.

## Constraints / validation
- `reqwest` futures must stay `Send` and be awaited outside the blocking task; only
  the network stream crosses the boundary.
- Do not change chunk bytes, digests or data-plane output.
- Do not change any §12 non-finding.

## Done when
Decode/write no longer runs on async workers (or the blocking-context contract is
explicitly enforced at all call sites), chunk bytes are unchanged, and tests pass. If
the boundary cannot be bridged safely, **stop** and record `skipped` with a one-line
reason.
