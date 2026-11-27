# T08 — Ledger CAS/refund helpers in `girpr-util-counter`

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Extend `girpr-util-counter` with structured charge/refund primitives and adopt
them in the five hand-written CAS/saturating loops of `app/report/src/ledger.rs`.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:392-401` — APP-02, five loops.

## Boundary / approach

Add `try_charge`/`try_refund` (and any shared saturating step) to
`girpr-util-counter`, returning a structured refund outcome so `Meter` can keep
its own state transitions. Do not move `Meter` policy into the leaf.

## Guardrails

- `Meter` keeps the `u64::MAX` sentinel, the live/settled distinction, the
  `fetch_max` peak update and all warnings.
- The reservation being intentionally local is a keeper (report 893-896): only
  CAS mechanics are shared.
- Boundary tests pin exact limit semantics; preserve them byte-for-byte in
  behavior.

## Commit breakdown

1. Add counter primitives with unit tests.
2. Adopt in `charge_live`, `refund_live` and `settle`.
3. Adopt in `reserve_ram_bytes` and `release_ram_bytes`.

## Validation

- Ledger boundary tests (`ledger.rs:653-677,833-851,931-958,968-992`).
- `cargo nextest` for util-counter and app/report.

## Done when

All five loops call the shared primitives while limit outcomes and warnings are
unchanged.
