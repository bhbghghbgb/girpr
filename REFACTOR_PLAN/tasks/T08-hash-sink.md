# T08 — Streaming hash-sink leaf

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F3 (lines 125–136) — structurally
  identical "hash bytes as they pass through" sinks.
- Assumptions to validate: §A7 (62–67), §A3 (29–35), §A2 (21–27).
- Related: T01 (`girpr-digest`), T09 (bounded zstd decode).

## Scope
- Candidate shared crate `crates/hash` (`girpr-hash`) or an extension of
  `girpr-digest` (T01) if that completed — choose the smaller logic-free boundary.
- Report names: `src/util/md5.rs` (reader → MD5, 512 KiB buffer),
  `src/sophon/stream.rs` (`HashingWriter<W>`, byte ceiling, hashes accepted bytes
  only), `tools/capture/src/hash/mod.rs` (`DualDigest`, `HashingFile` with
  mandatory `restart`), `tools/capture/src/emit/index.rs` (`digest_file`, 64 KiB
  cap), `tools/sync/src/hash/stream.rs` (optional MD5/SHA by `AlgoSet`).
  Locate before editing.

## Implementation plan
1. Validate the structural identity in §F3 and the ordering policies that must
   stay with each owner: limit-before-forward, write-before-hash, and
   restart-on-retry. A naive shared writer would produce plausible-but-wrong
   digests (§F3 cites the exact sites).
2. Expose a shared `Write`/reader sink parameterised by buffer size, algorithm
   set and reset contract, plus the digest result accessor. Keep it a leaf that
   depends only on digest mechanics (T01), not on any program.
3. Migrate the named sites, keeping their own byte ceilings, accepted-bytes
   rules and reset/restart handling at the call site. Do not move those policies
   into the shared crate.
4. Leave girsync's `DigestSet`/`AlgoSet` and girpr's scanner semantics intact
   (§A7, §N5).

## Constraints / validation
- Digest outputs must be byte-identical (§A3); the cited "plausible-but-wrong"
  ordering traps are the main risk.
- No single digest semantics (§A7); the shared sink returns bytes/hex mechanics
  only.
- A new gircap leaf dep requires §A2 doc tables to be updated.

## Done when
The common sink mechanics are shared, each caller keeps its ordering/reset
policy, and all digest, stream and cache byte tests pass unchanged.
