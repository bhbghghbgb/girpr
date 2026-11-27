# T11 — Shared `clear_readonly` mechanics

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Move the stat → readonly check → clear → set_permissions sequence into one fs
leaf that returns a structured outcome, and keep the three different error
policies at their call sites.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:142-149` — X6, three copies, two policies.
- `docs/09-shared-code-refactor-findings.md:387-390` — APP-01.
- `docs/09-shared-code-refactor-findings.md:295-298` — R-04.

## Boundary / approach

Add `clear_readonly(path) -> ReadOnlyOutcome` to an fs leaf (extend the most
natural existing one rather than creating a crate for a single function).
Callers keep logging, fallbacks and fatality.

## Guardrails

- `app/fetch/src/persistent.rs` and `src/repair/persistent_exec.rs` warn and
  continue; `src/repair/file/mode.rs` propagates. Do not unify them.
- Caller fallbacks (temp sibling, delete, retry open) and warning text stay local.
- This mutates real game files: preserve the exact permission change.

## Commit breakdown

1. Add leaf outcome type and function with tests.
2. Adopt in fetch persistent and repair persistent_exec (warn-and-continue).
3. Adopt in repair file/mode (propagate).

## Validation

- Readonly-mode tests in all three call sites.
- `cargo nextest` for the fs leaf, fetch and root repair.

## Done when

The mutation sequence has one implementation and callers retain their own error
handling.
