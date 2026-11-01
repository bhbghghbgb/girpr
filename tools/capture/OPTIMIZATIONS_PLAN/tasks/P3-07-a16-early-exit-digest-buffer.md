# P3-07 — Early A16 exit and larger digest buffer

Report: `../../OPTIMIZATIONS_REPORT.md` lines 519-529 (C3) and 531-542 (C4).

Dependencies: P1-06.

## Goal

Two low-risk runtime fixes:

1. `Checksums::a16` calls `findings(root).into_iter().next()`, but `findings`
   scans the whole tree and re-hashes every file before returning. Make the
   capture's first-finding path stop at the first finding.
2. `digest_file` uses `std::io::copy`'s default buffer (currently 8 KiB). Use a
   larger buffer (64 KiB) for the identity path and for the gzip decoder's
   input, without changing the bytes hashed.

## Where to start

- `src/emit/index.rs`: `a16` (around line 390), `findings` (around line 409),
  `walk`, `digest_file` (around line 218); `Checksums::scan_reusing` uses
  `digest_file`.
- `src/hash/mod.rs`: `DualDigest`, `Encoding`.
- Existing A16/digest tests in `src/emit/index.rs`.

## Steps

### C3: early exit

1. Refactor the body of `findings` into a helper that accepts a limit (or
   yields findings in order). `a16` asks for the first finding; `findings`
   keeps returning all of them.
2. Preserve the finding order: extras, ghosts, count/total mismatch, then
   per-entry re-hash. A first-finding result must be the same first item the
   full list would produce.
3. Stop the re-hash loop once the limit is reached.
4. Keep the full `findings` behavior for `verify`; do not change its messages.

### C4: buffer

1. In `digest_file`, read the identity path through a
   `BufReader::with_capacity(64 * 1024, file)` (or an explicit 64 KiB copy
   loop). For gzip, wrap the `File` in a `BufReader` before `GzDecoder::new`,
   or use `GzDecoder::new(BufReader::with_capacity(...))`.
2. Use a `BufWriter`/`DualDigest` directly; `DualDigest` is the writer, so the
   buffer only applies to the reader.
3. Keep the one-pass dual digest; do not hash twice.
4. Do not change the checksum output, encoding handling, or file ordering.
5. Update the misleading test comment that claims a 64 KiB buffer, or make it
   true.

## Tests

- A16 first-finding result equals `findings(root).first()` on the same
  corrupted tree.
- A16 does not re-hash after the first finding if a counter/seam is available;
  otherwise the equality test is the acceptance.
- Digest tests: identity and gzip digests are unchanged for small and large
  files; gzip still digests decompressed bytes.
- Existing A16 tests still pass.

## Acceptance

- Capture A16 stops early on an early mismatch.
- Verify still reports all findings.
- Digest bytes and documents are unchanged.
- `cargo test -p gircap` passes.

## Commit plan

1. `perf(emit): stop A16 at the first finding`
2. `perf(emit): use a larger read buffer when digesting`
3. `test(emit): pin first-finding order and digest equivalence`

## Notes

- The two changes are independent; keep separate commits so a regression can
  be reverted alone.
- Do not reference the report or plan in code comments.
