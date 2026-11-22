# T42 — Shared IO read-buffer policy

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F47 (lines 625–632) — the 512 KiB hash
  read chunk is duplicated in two shared crates; candidate boundary is a shared IO
  read-buffer policy. Confidence medium.
- Assumptions to validate: §A5 (57–67), §A2 (29–36), §A14 (129–135).

## Scope
- `crates/util-hash/src/md5.rs` (`MD5_BUF = 512 * 1024`), `crates/sync-cache/src/hash/stream.rs`
  (`CHUNK = 512 * 1024`), and `crates/digest/src/stream.rs` (`hash_reader` deliberately takes
  a caller-supplied buffer). Locate before editing.

## Implementation plan
1. Validate §F47: confirm the duplicated constant and the deliberate caller-owned-buffer
   seam in `girpr-digest`.
2. If attempted, expose the 512 KiB policy as a named constant/policy while preserving the
   caller-owned-buffer seam — do not make `hash_reader` own its buffer.
3. Migrate callers directly (§A2); move through §A14 seams where relevant.
4. If the shared policy contradicts the caller-owned seam, scope down to a named constant
   or stop.

## Constraints / validation
- High-risk (§F47): changing read boundaries affects throughput and the small-buffer
   equivalence test (`util-hash/src/md5.rs` pins boundary behavior) (§A5). Do not change the
   buffer size or alignment.
- Preserve the caller-owned-buffer seam; a wrong change breaks digest streaming.

## Done when
The buffer size is shared as a named policy with identical read boundaries and the
caller-owned seam intact, or a validated skip with the one-line reason is recorded.
