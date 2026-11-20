# T01 — Digest newtypes + hex sweep (`girpr-digest`)

## Goal / pointers
- `docs/08-shared-crate-refactor-research.md` §F2 (lines 118–123), §F4 (138–144),
  §F5 (146–151) — the repeated hex spelling, the cross-crate digest mirror, and
  the duplicated test oracles.
- Assumptions to validate: §A1 (13–19), §A2 (21–27), §A3 (29–35), §A7 (62–67).
- Guidance: `REFACTOR_PLAN/guidance/DO-NOT-TOUCH.md` (§N5, §A7).

## Scope
- Candidate shared crate `crates/digest` (`girpr-digest`), named after the leaf
  convention in §A1.
- Report names: `src/util/md5.rs` (`parse_hex`, `to_hex`, `Display`, `EMPTY`,
  RFC vectors), `tools/capture/src/hash/mod.rs` (`md5_hex`, decode boundary),
  `tools/sled2redb/src/convert.rs`, `tests/common/digest.rs`,
  `tools/sync/tests/helpers.rs`. Locate each in the current tree before editing.

## Implementation plan
1. Validate that the three digest representations truly must stay distinct
   (§A7): girpr's strict 16-byte newtype where `EMPTY` means "unknown, not a
   match", girsync's raw bytes plus verbatim foreign names, gircap's lowercase
   hex strings. Design the shared layer so it supplies mechanics only — hex
   encode/decode, `matches_hex`, `Display`, and shared test vectors — and does
   **not** become a single semantic `Digest` type used by all three.
2. Create `girpr-digest` with the hex encode/decode layer (all sites already use
   `hex` 0.4.3 per §F2) and `Md5([u8;16])` / optional `Sha256([u8;32])`
   mechanics with `EMPTY`, `parse_hex`, `to_hex`, `matches_hex`, `Display`.
   Keep it a leaf with no program dependency.
3. Adopt it where semantics are unchanged: girpr's `md5` hex boundary, gircap's
   decode/publish hex, sled2redb's conversion spelling, and the duplicated test
   oracles (`EMPTY_MD5`, `md5hex`, RFC 1321 vectors). Keep gircap's string
   publish shape and girsync's `DigestSet`/`AlgoSet` untouched (§N5).
4. If gircap gains this as a new leaf dependency, update the independence tables
   per §A2 in the same work (see `guidance/CONVENTIONS.md`).

## Constraints / validation
- No byte, key-order or hex-casing change in any published artifact (§A3).
- Do not fold `DigestSet`, `AlgoSet`, `HashAlg` or the girpr range scanner into
  the shared type (§N5, §A7).
- Parsing policies differ (strict 32-digit vs decode-with-context); keep both
  policies at their call sites, share only the primitive.
- If a single shared type would force girsync to drop foreign names or gircap to
  drop its publish shape, stop — this task must not collapse §A7 semantics.

## Done when
Hex spelling and digest test oracles are sourced from `girpr-digest`, the three
semantic representations remain distinct, and all digest/wire byte tests pass.
