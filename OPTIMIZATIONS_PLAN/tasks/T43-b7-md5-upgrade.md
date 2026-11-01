# T43 - B7: evaluate the md-5 upgrade (conditional)

Report: `../../OPTIMIZATIONS_REPORT.md` lines 1060-1076 (B7).

Dependencies: none. This is a **conditional** task: benchmark first, and do not
change code without a measured benefit. Ending as "no change, measured" is a
successful outcome.

## Goal

`md5 0.7` is old but stable. The maintained RustCrypto `md-5` may offer a
hardware-accelerated backend and a maintained dependency. Digest correctness is
load-bearing: every scan, reuse slice, and verify compares MD5s. A mechanical
replacement is wide and needs a benchmark to justify it.

## Decision gate

1. Establish the baseline on the current dependency:
   - hash a large fixture (for example 1 GiB of existing test data, or a
     repeated 8 MiB buffer) with `util::md5`'s current API;
   - measure throughput and the wall-clock share of hashing in a scan/verify
     run if a large fixture exists.
2. Build a throwaway branch with `md-5 = "0.10"` and the equivalent
   `Md5::new`/`update`/`finalize` calls. Measure the same workload. Keep the
   branch out of the final commit if it loses.
3. Apply the change only if:
   - the new backend is at least ~5-10% faster on the same workload, or
   - the maintenance argument is accepted explicitly (the crate is
     unmaintained) and the hash output is proven identical.
   If neither holds, **stop**: record the numbers and "no change" in the
   return. That is the default outcome.

## If the gate passes

- Replace `md5 = "0.7"` with `md-5 = "0.10"` in the main package.
- Adapt `src/util/md5.rs`: `Md5`, `Context`, `compute`, hex parsing, and the
  `scan`/`scan_reader` digest paths. Keep the hex encoding identical
  (lowercase, 32 chars) and the empty-file constant.
- `tools/` use `md5 0.7` independently; do not change them unless the workspace
  feature unification requires it. A different crate name means no conflict.
- Keep the public `util::md5` API stable where possible; this is an
  implementation change, not a data-plane change.
- Add a known-vector test (the RFC 1321 vectors) and an empty-file test.

## Tests (only if changed)

- RFC 1321 vectors: `""`, `"a"`, `"abc"`, `"message digest"`, the alphabet,
  and a million `a`s.
- The scan/reuse/verify suites pass; the hashes are byte-identical.
- `cargo test -p girpr` passes.
- A size/throughput note in the return, not a timing test in CI.

## Acceptance

- Either "no change" with the measured reason, or a complete replacement with
  identical digests and a measured win/maintenance rationale.
- No partial migration (two MD5 crates used by the same code path).
- `cargo test -p girpr` passes.

## Commit plan

Only if changed:
1. `build(deps): replace md5 with md-5`
2. `refactor(md5): adapt the digest helpers`
3. `test(md5): pin the RFC vectors`

If skipped, no commit; record the benchmark and decision.

## Notes

- MD5 is not being replaced for security; it is the protocol's checksum. Do not
  switch algorithms.
- A hardware-accelerated backend may not exist on all targets; measure on this
  host and say so.
