# P1-07 — Schema and path validation for every document

Report: `../../OPTIMIZATIONS_REPORT.md` lines 207-227 (A9).

Dependencies: P0-03.

## Goal

Validate the schema version of every bundle document that carries one, not just
`MANIFEST.json`, and validate `patchmap.expectations.result_path` like the other
document-supplied paths.

Documents to cover: `chunks/index.json`, `verify/selfcheck.json`,
`api/error_shapes.json`, `api/error_responses.json`,
`diagnostics/rejections.json`, `diagnostics/path_map.json`, and every
`patchmaps/*.json`.

## Where to start

- `src/verify/bundle.rs`: `Bundle::open` (around line 71), `read_json`
  (around line 322), `schema_version_of` (around line 399), `read_patchmaps`,
  `validate_document_paths` (around line 248).
- Every document type already has `schema_version: u32`; `crate::SCHEMA_VERSION`
  is the expected value.
- `src/paths.rs`: `relative_path_problem`, `logical_path_problem`,
  `field_component_problem`.

## Steps

1. Add one generic reader that reads a document as a JSON value (or a minimal
   version struct) first, compares `schema_version` to `crate::SCHEMA_VERSION`,
   and only then deserializes the full type. It must name the file, the found
   version, and the expected version. Do not parse the full document first:
   the version is the diagnosis and a missing field is a symptom.
2. Apply it to the six secondary documents and the patchmaps. Keep
   `MANIFEST.json`'s existing version-first path, or route it through the same
   helper if that does not lose its tailored error message.
3. For `read_patchmaps`, use the versioned reader per file; a bad patchmap
   should name that file, not the directory.
4. Extend `validate_document_paths` to check `map.expectations.result_path`
   when `Some`. `result_path` is bundle-relative and includes a tree component
   (`expected/<path>`); validate it with the relative-path rule (no absolute,
   drive, `..`, empty component, backslash) and reject an escape. Keep
   `patchmap.path` on the logical-path rule.
5. Do not change the shape of any document. Do not add fields.
6. Update `docs/01-artifact-schema.md` only if it states that only
   `MANIFEST.json` is version-checked; the permanent docs are the right place
   for the contract.

## Tests

- Unit/integration: each secondary document with a wrong `schema_version` makes
  `Bundle::open` fail with a message naming that file and both versions.
- Unit: a valid bundle with all current versions still opens.
- Unit: `result_path = "../escape"`, `C:evil`, `expected/..`, or a backslash
  path is refused; `expected/a.bin` is accepted.
- Patchmap: a wrong version in one patchmap fails before any rebuild.
- If P0-03 has a characterization test for secondary versions/result_path,
  flip it.

## Acceptance

- Schema drift is diagnosed at the document that caused it.
- `result_path` is validated at the door.
- Old bundles that predate the check are refused with a clear version message
  (or, for documents without a version today, keep current behavior).
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(verify): validate every bundle document's schema version`
2. `fix(verify): validate a patchmap's result path`
3. `test(verify): cover secondary schema drift and result-path escapes`

## Notes

- Keep the error text actionable: say which document and which version.
- Do not silently apply current semantics to an old document.
- Do not reference the report or plan in code comments.
