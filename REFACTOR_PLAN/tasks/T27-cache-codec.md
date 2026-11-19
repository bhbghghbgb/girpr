# T27 — Cache wire codec → `postcard` / `bincode`

## Goal / pointers
- `docs/08-hand-rolled-code-reduction-findings.md` §E1 (lines 515–543).
- Assumption to validate: §I.9 (lines 840–843).

## Scope
- Package: `tools/sync`, `src/cache/codec.rs` (`encode_rec`, `push_hash`,
  `decode_rec`, `encode_meta`, `decode_meta`), `src/cache/schema.rs`,
  `src/cache/write.rs`, `src/cache/db/cachedb.rs`, `src/cache/open.rs`
  (lines 2033–2051), plus the manifest.

## Implementation plan
1. Derive `serde` on `FileRec`/`Meta` (`cache/schema.rs`).
2. Model `DigestSet` with a custom serde impl that preserves `iter()` then
   `foreign_iter()` order and the forward-compatible "unknown algorithm names
   carried verbatim" rule — that rule is **not** free from a derive (§E1).
3. Adopt `postcard` (or `bincode` 2.x with fixed-width LE ints to preserve the
   exact byte layout); replace codec calls and delete `codec.rs`.
4. Keep the existing version gate (postcard is positional).

## Constraints / validation
- Cache format breaks; old caches must be rebuilt via `girsync update --dir`
   — confirm that is acceptable (§I.9).
- Tests named in §E1 (lines 533–538), especially the byte-identical map test,
   unknown-name carry order, unknown-algorithm preservation and
   truncated/bad-version rejection.

## Done when
The hand-rolled codec is gone, the forward-compat rule survives, and the tests
pass under the new format. (Large task — own review checkpoint.)
