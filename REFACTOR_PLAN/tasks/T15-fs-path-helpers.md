# T15 — Filesystem/path helper adoptions

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Adopt existing leaf helpers at bypassing call sites and generalize the repeated
metadata classification into one `app/util-path` primitive.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:299-308` — R-05, metadata classification.
- `docs/09-shared-code-refactor-findings.md:584-590` — LF-09, device-name sets.
- `docs/09-shared-code-refactor-findings.md:600-606` — LF-11, separator rewrites.
- `docs/09-shared-code-refactor-findings.md:494-499` — APP-16, case-folded rel keys.
- `docs/09-shared-code-refactor-findings.md:508-512` — APP-18, first-out-of-scope predicates.
- `docs/09-shared-code-refactor-findings.md:514-519` — APP-19, two `Md5` import paths.
- `docs/09-shared-code-refactor-findings.md:686-690` — SYNC-08, path-prefix iteration.

## Boundary / approach

Add `classify_metadata(io::Result<Metadata>) -> MetadataFact` in `app/util-path`
with `FileLen`/`is_file` as thin views. Add named device-rule constants in
`girpr-fs-names`, an inverse `native_separator` beside `forward_slash`, a shared
ancestor walk, a private generic first-out-of-scope helper, and path-prefix
helpers in `girsync-leaves`.

## Guardrails

- `scope/report` uses `symlink_metadata` while the other sites follow links.
- `Unreadable` must not be counted as `Absent`; `Failed` must not collapse into `NotAFile`.
- `girpr-filter`'s `Backslash::Escape` policy depends on the `Cow` shape and must
  be preserved.
- *When to fold* case stays at call sites; the fixture's lexical `.`/`..` replay
  and relative anchoring stay local.
- The three out-of-scope predicate names and docs are a public contract; use a
  private generic helper in the same module, no new crate.
- Standardize the `Md5` import on `girpr_digest`; removing the `util-hash`
  re-export is a separate API decision (report 893-896) and is not done here.
- Path-prefix membership/disk-stat questions and short-circuit order stay local.

## Commit breakdown

1. Add `MetadataFact`/`classify_metadata` with tests for absent/unreadable/not-a-file/failed.
2. Adopt at the four classification sites.
3. Add device-rule constants and `native_separator`; adopt bypassing call sites.
4. Adopt case-fold helper, ancestor walk, out-of-scope helper and `Md5` import.
5. Add `path_prefixes`/`strict_path_prefixes` in `girsync-leaves`; adopt callers.

## Validation

- Presence/size decision tests in check/temp/post/scope.
- Path-shape/filter tests; `cargo nextest` for util-path, fs-names, girsync leaves.

## Done when

Metadata classification has one implementation and shared path helpers are
adopted without changing policy at the call sites.
