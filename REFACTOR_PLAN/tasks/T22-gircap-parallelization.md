# T22 — Parallelize gircap verification and capture assembly

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §9 **TOOL-14** (lines 740–744) — `verify`'s per-blob
  MD5 + SHA-256 + zstd measure is serial and CPU-bound; `rayon::par_iter()` over
  `index.blobs`, collecting rows and sorting by id for determinism, paired with T20 so
  each worker streams its own file handle.
- §9 **TOOL-15** (746–752) — capture assembly is serial; parallelize with rayon,
  appending `(rows, patchmap, bytes, digests)` in index order to keep artifact bytes
  deterministic, and verify `path_map`'s interior mutability is thread-safe before
  sharing it.
- §11 **assumption 17** (lines 885–888) — preserve documented write order and
  deterministic artifact bytes; the close-out fixpoint passes must be unaffected.
- Keep items: deterministic artifacts, thread safety, §12,
  `guidance/DO-NOT-TOUCH.md`.

## Scope
- `tools/capture/src/verify/checks.rs` (~30–96), `emit/fixture.rs` (~156–186).
- The `path_map` interior-mutability type and any shared state used during assembly.
- Locate current code before editing; the list above is a hint.

## Implementation plan
1. Validate assumption 17 and confirm `path_map`'s interior mutability is thread-safe
   (or make it so) before sharing across workers. If it cannot be made safe, skip the
   assembly parallelization and keep only verification.
2. Parallelize per-blob verification with `rayon`, collecting `(id, row)` results and
   sorting by id for deterministic output; each worker opens its own file handle
   (coordinate with T20's streaming).
3. Parallelize capture assembly, appending results in index order so artifact bytes
   and write order stay deterministic.
4. Deliver in several commits if useful: verify parallelism, assembly parallelism.

## Constraints / validation
- Artifact bytes and write order must be identical to serial behavior; add an
  ordered-output comparison if one is not present.
- Thread safety must be proven before sharing mutable state; do not rely on luck.
- Do not change close-out fixpoint behavior or any §12 non-finding.

## Done when
Verification and assembly are bounded-parallel with byte-identical, deterministically
ordered outputs and thread-safe shared state. If thread safety or determinism cannot
be guaranteed, **stop** and record `skipped` with a one-line reason.
