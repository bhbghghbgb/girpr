# T14 — Claim a unique sibling, then publish

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Extract the unique-sibling claim and cleanup protocol shared by four crash-safe
publish paths, without merging their genuinely different publish mechanisms.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:238-247` — X13, four implementations.
- `docs/09-shared-code-refactor-findings.md:557-561` — LF-05, atomic publish.
- `docs/09-shared-code-refactor-findings.md:672-677` — SYNC-06, protocols differ.

## Boundary / approach

Extend `app/util-path::atomic` with a claim/publish primitive: reserve a unique
name beside the target, hold it across a write, publish with an
overwrite/no-clobber choice, and clean up on error. Share only claiming and
cleanup.

## Guardrails

- The protocols differ: hard-link publish, one-shot `persist`, and the two-rename
  case-fix protocol. Do not collapse them.
- Prefixes, sidecar cleanup, the two-step case rename and error context stay at
  call sites.
- Tests pin exact residue names on failure; these are crash-safety paths.

## Commit breakdown

1. Add claim/publish primitive to `app/util-path::atomic` with cleanup tests.
2. Adopt in `app/util-path` existing atomic callers.
3. Adopt in `tools/sync/cache-json/src/convert.rs`.
4. Adopt in cache backup publish and sync rename (claim/cleanup only).

## Validation

- Residue-name tests and crash-safety tests in all adopting paths.
- `cargo nextest` for util-path and the sync members.

## Done when

Name claiming and error cleanup have one implementation, and each publish
protocol keeps its own mechanism and residue names.
