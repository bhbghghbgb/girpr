# T19 — Test fixture helper consolidation

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Consolidate the small test-only helpers duplicated across binaries and fixture
crates by widening the existing test-support surfaces, and resolve the
`girpr-cache*` filter disagreement with a dedicated leaf.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:217-236` — X12, duplicated test mechanics.
- `docs/09-shared-code-refactor-findings.md:315-322` — R-07, tempdir helpers.
- `docs/09-shared-code-refactor-findings.md:324-332` — R-08, bundle fixture I/O.
- `docs/09-shared-code-refactor-findings.md:334-340` — R-09, JSON key-order scanner.
- `docs/09-shared-code-refactor-findings.md:349-355` — R-11, `RunCtx` literals.
- `docs/09-shared-code-refactor-findings.md:444-459` — APP-09/10/11, lock/token helpers.
- `docs/09-shared-code-refactor-findings.md:679-684` — SYNC-07, cache-name filter.
- `docs/09-shared-code-refactor-findings.md:563-567` — LF-06, `strip_ansi`.

## Boundary / approach

Widen `girpr-tempdir::test_support` (predicate-parameterized `entry_names`,
`strip_ansi`, `set_mtime`), `girpr-test-fixtures` (JSON/zstd helpers),
`girpr-report-core` behind a `test_support` feature (`json_keys_in_order`), and
add a documented `ctx_default` builder in the `girpr::test_support` seam. Add a
small `girpr-cache-names` leaf for the prefix and predicates.

## Guardrails

- The tempdir `entry_names` copy filters `girpr-cache*` while the other four do
  not; the predicate must be a parameter.
- Production folds ASCII case on Windows while the test helper does not; the
  new leaf must reconcile this deliberately.
- `CARGO_BIN_EXE_girpr` is integration-only; pass the exe path explicitly if a
  helper leaves the integration root.
- Re-export through `tests/common/mod.rs` rather than changing the four test roots.
- `ctx_default` must not absorb `Args::into_ctx` validation or its warning/exit
  ordering; the fixture's lexical path replay stays local.
- The `args_take_lock` CLI-token rule and parse-or-panic wrappers stay local.
- Comments rejecting `preserve_order` stay (A10). Blast radius is tests only.

## Commit breakdown

1. Widen `tempdir::test_support` with `entry_names`/`strip_ansi`/`set_mtime`.
2. Expose JSON/zstd fixture helpers on `girpr-test-fixtures`; re-export in `tests/common`.
3. Add feature-gated `json_keys_in_order` on `girpr-report-core`.
4. Add `ctx_default` builder in `girpr::test_support`.
5. Add cache-names leaf and adopt lock/kv-token helpers.

## Validation

- Test compilation model: four roots, one process per test, `CARGO_BIN_EXE_girpr`
  bound only to integration targets.
- `cargo nextest` across all test surfaces; snapshots must be unchanged.

## Done when

Each helper has one home, the cache-name predicates agree, and no test behavior
changed.
