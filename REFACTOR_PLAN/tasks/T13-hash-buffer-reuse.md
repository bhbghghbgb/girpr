# T13 — Reuse hash scratch buffers and tighten range handling

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §4 **MEM-03** (lines 320–327) — each hash call does
  `vec![0u8; MD5_BUF]`; private `*_with_buf` helpers already exist. Expose them and
  keep one reusable buffer per worker via thread-local or task-owned storage.
- §4 **MEM-04** (lines 328–336) — read buffers are `resize(len, 0)`d immediately
  before being fully overwritten; reserve capacity and read into spare capacity
  (`spare_capacity_mut` + `MaybeUninit` + `set_len`, or `Read::read_buf`).
- §8 **MIC-09** (lines 614–619) — `scan_reader_with_buf` interns ranges and sorts a
  fresh `Vec` every call; pass pre-sorted, deduplicated ranges from `scan_ranges`
  (chunk-order invariant) so the sweep skips the sort.
- §8 **MIC-10** (lines 620–623) — `RangeState::is_ok` clones the whole MD5 context per
  verdict; finalize once when a range is last touched and compare stored bytes.
- Keep items: byte-for-byte digests, safety of uninitialized reads, §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `app/util-hash/src/md5.rs` (buffer allocation ~124–127, 286–289, 336–338;
  zero-fill ~251–255; range interning ~140–157, 209–237; `RangeState::is_ok`
  ~85–90).
- `src/repair/file/mod.rs` (scratch buffer ~114–119), `scan_ranges` producers.
- Locate current helpers/callers before editing; the list above is a hint.

## Implementation plan
1. Confirm the current allocation/zero-fill sites and the existing `*_with_buf`
   helpers. If a buffer is already reused, skip only that sub-change.
2. Expose `*_with_buf` variants and keep one reusable buffer per worker (thread-local
   or task-owned); update callers to pass it in.
3. Replace zero-fill-then-read with reserve + read into spare capacity, using safe
   `Read::read_buf` where possible; if `MaybeUninit`/`set_len` is used, prove no
   uninitialized byte is ever read.
4. Pass pre-sorted/deduplicated ranges from `scan_ranges` so the intern sweep skips
   the sort; finalize each range's MD5 once and compare stored bytes.
5. Deliver in several commits if useful: reusable buffers, read-into-capacity, range
   changes.

## Constraints / validation
- Digests must be byte-for-byte identical for every input; add equivalence tests.
- Any unsafe `set_len`/spare-capacity code must never expose uninitialized bytes;
  prefer safe APIs and document any invariant.
- Do not change data-plane output or any §12 non-finding.

## Done when
Scratch buffers are reused without zero-fill, ranges are pre-sorted and MD5 finalized
once, and digest-equivalence tests pass. If safety or equivalence cannot be proven,
**stop** and record `skipped` with a one-line reason.
