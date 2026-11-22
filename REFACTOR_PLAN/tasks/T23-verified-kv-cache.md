# T23 — Generic verified KV cache assessment (likely skip)

## Goal / pointers
- `docs/11-shared-crate-generalization-research.md` §F28 (lines 437–446) — a generic
  cache/DB abstraction would fight an intentionally missing product cache; candidate
  boundary is a parameterized "verified KV cache" implemented by both `girpr-sync-cache`
  and a future product cache. **High-risk**: literal-byte contract and version gate behind
  a generic row trait, and the abstraction would tempt reinstating a forbidden cache.
  Confidence medium.
- Assumptions to validate: §A5 (57–67), §A13 (122–128), §N15 (758–761).

## Scope
- `crates/sync-cache` (`entries`/`meta` schema, `FileRec`, version gate, positional
  postcard codec, pinned redb `=5.0.0`, sealed `db/handle.rs` local trait). Product has no
  manifest cache by design. Locate before editing.

## Implementation plan
1. Validate §F28: confirm the sync cache is one specialized schema and the product
   deliberately has no cache.
2. Assess only: would a generic layer hide the literal-byte contract/version gate (§A5)
   or tempt a forbidden product cache (§A13)? If so, record a validated skip with the
   one-line reason. Note that redb's sealed handle API forces a local, non-reusable trait.
3. Implement only on hard evidence of a safe, byte-preserving boundary; do not reopen N15.

## Constraints / validation
- Literal-byte pin, `FMT_V2`, `CACHE_VERSION`, closed `FileRec.kind` registry and
  1-syscall gaps are closed decisions (N15); F28 does not reopen them.
- Do not reinstate a forbidden product cache (§F28, §N5). redb is pinned `=5.0.0`.

## Done when
A validated decision is recorded — expected `skipped` citing the byte contract and
forbidden-cache risk — or a safe generic layer with byte-identical behavior.
