# T24 — Workspace package metadata inheritance

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Introduce `[workspace.package]` and `[workspace.dependencies]` and migrate all
members off restated `version`, `edition` and duplicated dependency versions.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:249-259` — X14, manifest duplication.
- `docs/09-shared-code-refactor-findings.md:488-492` — APP-15.
- `docs/09-shared-code-refactor-findings.md:718-721` — SYNC-13, lint-table caveat.
- `docs/09-shared-code-refactor-findings.md:859-862` — CAP-19, narrow feature lists.

## Boundary / approach

Add workspace-level package/dependency tables, migrate each member manifest to
`workspace = true` where appropriate, and leave the tools' reduced lint block
local.

## Guardrails

- Exact pins (`redb = "=5.0.0"`, `bitflags = "=2.13.2"`) must be preserved.
- Narrow feature lists must not widen; explanatory isolation comments stay local.
- Cargo cannot inherit a *subset* of workspace lints, so the tools' opt-out
  stays local rather than weakening the product lint table.
- `[lib]` sections are per-member and stay local.

## Commit breakdown

1. Add workspace tables in the root manifest.
2. Migrate root and `app/*` manifests.
3. Migrate `crates/*` manifests.
4. Migrate `tools/sync*`.
5. Migrate `tools/capture*`.
6. Refresh the lockfile and run a full workspace check.

## Validation

- Full workspace `cargo check` and `cargo nextest`.
- Diff dependency versions/features before and after; no version may drift.

## Done when

All 39 members inherit shared metadata and the resolved dependency graph is
unchanged.
