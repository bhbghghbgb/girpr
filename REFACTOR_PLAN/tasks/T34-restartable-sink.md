# T34 — Generalized `RestartableSink` with blanket impls

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F39 (lines 547–555) — two near-identical
  restartable byte-sink traits exist in two shared crates; candidate boundary is one
  generalized `RestartableSink` with blanket impls. Confidence high.
- Assumptions to validate: §A12 (114–121), §A2 (29–36), §A14 (129–135).

## Scope
- `girpr-digest` (`BodyFile: Write + Send` with `restart()`, placed to break a cycle),
  `girpr-fetch` (`ChunkSink: Write` with `restart()`, `Vec<u8>`/`&mut T` impls),
  `gircap-http` (re-exports `BodyFile`). Locate before editing.

## Implementation plan
1. Validate §F39: confirm the two traits and the `BodyFile` cycle-breaking placement.
2. If attempted, define a generalized sink with blanket impls; decide whether `Send` is
   required and what `restart()` means for each implementor (truncate a file vs `clear()` a Vec).
3. Migrate consumers directly (§A2); keep compatibility re-exports only if needed, recorded
   in the note. Move through §A14 seams.
4. If unifying couples fetch's zstd path to the HTTP body seam the digest leaf deliberately
   hosts, scope down or stop.

## Constraints / validation
- High-risk (§F39): the traits differ in `Send` bounds and `restart()` semantics; unifying
   couples fetch's zstd path to the digest-hosted HTTP body seam (§A12). Do not break the
   cycle that motivated `BodyFile`'s placement.
- Byte contracts on streamed bodies must stay unchanged (§A5).

## Done when
A single sink is shared with both semantics preserved and the cycle not reintroduced, or a
validated skip with the one-line reason is recorded.
