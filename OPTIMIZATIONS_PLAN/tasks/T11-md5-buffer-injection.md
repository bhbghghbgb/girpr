# T11 — Inject the MD5 buffer; shrink 513 KiB files

Priority P1 · root product unit tests / production code · checkpoint CP4 ·
report finding C2

## Pointers (give these to the implementing session)

* Report finding: `docs/09-test-scale-reduction-research.md:L369–L391` (§C2).
* Report synthesis: `docs/09-test-scale-reduction-research.md:L797–L799`
  (pattern 2), `L815–L816` (Wave 2).
* This plan: `OPTIMIZATIONS_PLAN/tasks/T11-md5-buffer-injection.md`.
* Process: `OPTIMIZATIONS_PLAN/ORCHESTRATION.md`, section "Implementing session".
* Constraints: `OPTIMIZATIONS_PLAN/STATE.md`, "Known constraints from the report".

## Goal

Replace ~1 MB of file IO and two ~513 KiB heap buffers with <1 KB by injecting a
small buffer through a private seam — no behavior change.

`src/util/md5.rs` `a_file_larger_than_one_block_is_scanned_across_the_boundary`
(:664–676) writes `MD5_BUF + 1000` bytes (:669) to cross the 512 KiB internal
buffer; `a_handle_digest_agrees_with_the_file_digest_at_any_position` (:678–702)
writes one `MD5_BUF + 17` file plus two tiny ones (:681–688). Production
`scan_reader` (:211–281, buffer at :233) and `md5_reader_digest` (:370–382,
buffer at :372) allocate `vec![0u8; MD5_BUF]`.

## Implementation plan

Code anchors (validate before editing): `src/util/md5.rs:211–281`, `:233`,
`:370–382`, `:664–702`.

Steps (one commit):

1. Extract private `scan_reader_with_buf(...)` and
   `md5_reader_digest_with_buf(...)` that accept `&mut [u8]`.
2. Public/production functions allocate `vec![0u8; MD5_BUF]` and delegate.
3. Tests pass a 64-byte buffer with a ~100-byte file and keep the straddle
   assertion (`start = buf.len() - 10`, len 20).

## Acceptance criteria

* The same `block_end`/`start`/`end` loop and range arithmetic is exercised —
  crossing a 64-byte boundary covers the same logic as crossing `MD5_BUF`.
* ~1 MB file IO → <1 KB; no RFC-1321 vector or ignored throughput bench touched
  (report §C non-findings, L461–L466).
* Production behavior byte-for-byte unchanged. Affected unit suite passes.

## Guardrails

* This is a private signature extraction with no behavior change; if the buffer
  ownership makes a clean seam impossible, stop and report `skipped` rather than
  rewriting MD5 logic.
* Do not alter the protected md5 RFC vectors or the two ignored benches. Report
  follow-up ideas without implementing them.
