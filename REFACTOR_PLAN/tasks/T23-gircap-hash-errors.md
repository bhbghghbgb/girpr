# T23 — Make gircap declared-size mismatch errors honest

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §9 **TOOL-21** (lines 796–805) — when the index declares
  a decompressed length smaller than the actual zstd frame, `zstd_decompress_from`
  limits decoding to the declaration and reports "decoded output exceeds the N B
  decode limit", conflating a lying index with a decompression bomb;
  `zstd_measure_from` uses the absolute ceiling.
- Change suggested in the report: distinguish "expected < decoded" from
  "decoded > ceiling" in the error text, naming both values; or measure first when the
  declaration is suspicious.
- Keep items: error coverage (do not silently accept a real decompression bomb),
  §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/hash/src/lib.rs` (`zstd_decompress_from` ~354–356, 421–435;
  `zstd_measure_from` for comparison).
- Locate current error construction before editing; the list above is a hint.

## Implementation plan
1. Confirm the current error conflation and the absolute ceiling used by
   `zstd_measure_from`.
2. Distinguish the two cases in the error text, naming both the declared/expected size
   and the actual/ceiling values; or measure first when the declaration is suspicious.
3. Keep the hard ceiling enforcing a real decompression bomb; do not weaken it.
4. Deliver in one commit with a test for the mismatch case.

## Constraints / validation
- A real "decoded > ceiling" case must still be rejected.
- Do not change any §12 non-finding or artifact bytes.

## Done when
Declared-size mismatches and genuine decode-limit violations produce distinct,
informative errors and both are covered by tests. If the distinction cannot be made
without weakening the ceiling, **stop** and record `skipped` with a one-line reason.
