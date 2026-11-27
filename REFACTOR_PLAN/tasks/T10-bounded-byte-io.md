# T10 — Bounded byte-copy, zero-fill and capture mechanics

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Share the bounded-piece write loop between the two correctness-critical repair
writers, and promote the reserved-capacity buffered read into a public
primitive.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:285-293` — R-03, bounded byte-copy/zero-fill.
- `docs/09-shared-code-refactor-findings.md:462-469` — APP-12, reserved-capacity read.

## Boundary / approach

Add an `app/util-io` leaf with `copy_bounded`, `zero_bounded` and
`capture_bounded`, each taking a caller-supplied write-ceiling charge callback.
Promote `read_into_vec` in `girpr-util-hash` to `read_up_to`/`reserve_exact`.

## Guardrails

- `copy_within` reads each piece before writing so an overlapping source
  survives; preserve that ordering.
- `capture_slice` uses a bounded `Take` to avoid zero-filling spare capacity.
- Short-read semantics must not change.
- Both writers produce on-disk bytes; byte-equality is the acceptance test.
- Do not merge the distinct failure classifications at the call sites.

## Commit breakdown

1. Add `app/util-io` primitives and tests (overlap, ceil charge, short write).
2. Promote `read_up_to`/`reserve_exact` in `girpr-util-hash`.
3. Adopt in `src/repair/file/mod.rs` and `src/repair/file/temp.rs`.
4. Adopt in `src/repair/file/inplace.rs`, which consumes both.

## Validation

- Repair file writer tests and produced-file byte equality.
- `cargo nextest` for util-io, util-hash and root repair.

## Done when

The bounded-piece loop and reserved-capacity read each have one
implementation, with identical on-disk output.
