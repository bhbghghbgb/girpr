# T04 — `girpr-digest` internal consolidation

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Remove duplication inside and around `girpr-digest`: the restartable byte-sink
trait, the `Md5`/`Sha256` newtype bodies, and the caller-buffered hash/copy read
loop. Do this after T03 has finished widening the crate.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:525-533` — LF-01, two restartable sink traits.
- `docs/09-shared-code-refactor-findings.md:535-541` — LF-02, caller-buffered hash loop.
- `docs/09-shared-code-refactor-findings.md:608-613` — LF-12, duplicated newtype methods.
- `docs/09-shared-code-refactor-findings.md:705-710` — SYNC-11, `copy_and_digest`.

## Boundary / approach

Introduce a `RestartableWrite` base trait in `girpr-digest` with the generic
`Vec<u8>`/`&mut T` impls; template `Md5` and `Sha256` with an internal macro or
const-generic body; add one `copy_with_sink`/hash-read loop helper in
`crates/digest/src/stream.rs` and delegate the cache and sync callers.

## Guardrails

- `BodyFile` must keep `Send` for its `dyn` use; `ChunkSink` is publicly
  re-exported from the root crate.
- Coherence with existing explicit impls is the primary hazard.
- Public signatures, error spelling and the no-allocation parse property used
  by `girpr-chunk-identity` must survive unchanged.
- Algorithm-set translation, chunk size (512 KiB) and digest comparison stay in
  the cache; write-before-hash ordering and the `io::copy` empty-set fast path
  stay at call sites.
- Move the empty-buffer refusal into the leaf only if it does not change error
  text visible to tests.

## Commit breakdown

1. Add `RestartableWrite` and move generic impls; keep `BodyFile`'s `Send`.
2. Template `Md5`/`Sha256` preserving signatures and error strings.
3. Add shared read/copy-with-sink helper in `digest::stream`.
4. Delegate `tools/sync/cache` hash loop and `copy_and_digest`; move the
   empty-buffer refusal.

## Validation

- Digest/stream/cache/sync `cargo nextest` suites.
- Golden digest tests and no-allocation parse tests; verify no public API break.

## Done when

The restart contract, newtype methods and buffered read loop each have one
implementation, with identical observable behavior.
