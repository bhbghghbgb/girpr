# P1-02 — Carry encoding, not a `.gz` suffix

Report: `../../OPTIMIZATIONS_REPORT.md` lines 76-95 (A2).

Dependencies: P0-01.

## Goal

Stop treating every captured path ending in `.gz` as a gzip document.
`emit::index::encoding_of` must return `Gzip` only for documents this tool
itself gzip-compressed. A captured game asset named `*.gz` must be digested as
the bytes on disk (`Identity`).

## Where to start

- `src/emit/index.rs`: `encoding_of` (around line 489), `digest_file`,
  `Checksums::scan_reusing`, `Checksums::findings`, `FileEntry`.
- `src/manifest/dump.rs` (around line 82-160): the only production writer that
  chooses gzip, via `hash::should_gzip` and `emit::write_json_gz`; it writes
  `manifests/<tag>/<field>.manifest.json[.gz]`.
- `src/emit/mod.rs`: `write_json_gz` (around line 478), `write_doc`,
  `write_bytes`, and the tree module.
- `src/hash/mod.rs`: `Encoding`, `should_gzip`.

## Decision

Use the narrow, durable-enough rule the report permits: the writer already knows
which documents it compresses. Restrict `encoding_of` to the document family
`manifests/<tag>/<field>.manifest.json.gz` (or the exact path shape produced by
`manifest::dump::write`). Every other `.gz` path is `Identity`.

Do not add encoding plumbing through every write result in this task. The
targeted rule is smaller and fixes the only production gzip writer; state the
invariant in the function comment so a future compressed document must extend
the rule deliberately.

## Steps

1. Replace the `ends_with(".gz")` test with a predicate that recognizes only
   the manifest document shape. Keep it private to `emit::index`.
2. Make the predicate match the writer's actual output, including the
   `.manifest.json.gz` suffix and the `manifests/` prefix. Decide explicitly
   whether nested tag/field components are allowed; the writer uses
   `manifests/<tag>/<field>.manifest.json.gz`, so a prefix check plus suffix
   check is enough.
3. Update the doc comment: it currently says extension decides encoding. It
   must say only the manifest writer compresses, and a captured `.gz` asset is
   `Identity`.
4. Keep `digest_file` unchanged; it already honors the `Encoding` it is given.
   A16 re-hashes with the stored `e.encoding`, so the fix applies to both the
   first scan and verification.
5. Check test fixtures that assert `checksums.json` encoding for `.gz` paths;
   update only those that are not manifest documents.

## Tests

- Unit: `encoding_of("manifests/7.1.0/game.manifest.json.gz")` -> `Gzip`.
- Unit: `encoding_of("game/assets/pack.gz")` -> `Identity`.
- Unit: `encoding_of("manifests/7.1.0/game.manifest.json")` -> `Identity`.
- End-to-end (if the P0-01 fixture exists): capture a game file named `*.gz`
  that is not gzip; close-out succeeds and its checksums entry has
  `encoding: "identity"` and the digest of the on-disk bytes. Flip the P0-01
  characterization test to this intended behavior.
- Existing manifest tests still see `encoding: "gzip"` for a large manifest.

## Acceptance

- A non-gzip captured `.gz` file does not abort close-out and is not hashed as
  decompressed bytes.
- A real compressed manifest document is still `Gzip` and still digested after
  decompression.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(emit): classify only manifest documents as gzip`
2. `test(emit): cover captured .gz assets and manifest gzip encoding`

## Notes

- This changes `checksums.json` for existing bundles containing non-manifest
  `.gz` files; that is the intended correction. No manifest document changes.
- Do not reference the report or plan in code comments.
