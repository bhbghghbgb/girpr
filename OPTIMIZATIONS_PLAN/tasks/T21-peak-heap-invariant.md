# T21 — `peak_heap.rs` measurement → scale-free invariant

Priority P2 · root product integration tests · checkpoint CP10 · report finding A5

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L143–L170` (§A5).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L800–L804`
  (pattern 3), `L818–L820` (Wave 3).
* Report constraints (12 ignored tests; this task removes one):
  `docs/09-test-scale-reduction-research.md:L826–L838`.
* This plan: `OPTIMIZATIONS_PLAN/tasks/T21-peak-heap-invariant.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Delete a 357-line ignored, non-asserting measurement file and replace it with an
always-on scale-free structural invariant — dropping the workspace's only
`unsafe` exemption in the process.

`tests/peak_heap.rs` is an `#[ignore]`d measurement: a custom `#[global_allocator]`
(:88–133), a streaming HTTP server (:223–266), a 256 KiB incompressible payload
(:198–210) and a printed MiB ratio (:276–357). It is the sole reason the
workspace carries an `unsafe` exemption (`Cargo.toml:26–29`; file-level
`#![allow(unsafe_code)]` at :60). The correctness it protects — streamed decode
equals buffered decode — is already pinned scale-free by
`the_streamed_decode_agrees_with_the_buffered_one` using 64 KiB and
`futures_util::stream::iter`, no socket, no allocator (`src/sophon/stream.rs:634–659`).

## Implementation plan

Code anchors (validate before editing): `tests/peak_heap.rs` (whole file),
`src/sophon/stream.rs:634–659`, the `ChunkFetch`/`ChunkSink` seam and
`stream_chunk_to`, `Cargo.toml:26–29` and the file-level `#![allow(unsafe_code)]`.

Steps (one or two commits):

1. Add an always-on invariant test using the existing `ChunkFetch`/`ChunkSink`
   seam: a counting sink records each `write` call while `stream_chunk_to`
   streams a multi-piece in-memory body. Assert piece count > 1, accumulated
   bytes equal the payload and no single write equals the whole payload (so
   reintroduced buffering fails).
2. Delete the allocator, TCP server, incompressible payload and `#[ignore]`.
3. Optionally drop the workspace's `unsafe` exception and its documentation.

## Acceptance criteria

* The invariant is structural and does **not** pass vacuously — verify it fails
  if `stream_chunk_to` is changed to buffer the whole body (report's stated
  risk).
* The streamed-decode-equals-buffered property remains pinned (existing test
  untouched).
* −357-line test file, −1 server task, −256 KiB payload and zstd encode, −1 log
  sink, −1 workspace `unsafe` exemption.
* If the exception is removed, clippy/build gates still pass. Affected suites
  pass.

## Guardrails

* This trades a human-read quantitative heap ratio for a structural bound; state
  that clearly in the commit. No runtime win is claimed (the test was ignored).
* Do not touch the other `#[ignore]` tests. Report follow-up ideas without
  implementing them.
