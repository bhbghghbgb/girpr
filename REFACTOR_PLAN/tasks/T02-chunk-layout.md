# T02 — Half-open byte range and tiling leaf

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Consolidate the three independent models of half-open byte ranges and chunk
tiling into one logic-free leaf, with a typed problem representation that each
caller maps to its own diagnostic.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:161-174` — X8, range algebra.
- `docs/09-shared-code-refactor-findings.md:592-598` — LF-10, `girpr-byte-range` candidate.
- `docs/09-shared-code-refactor-findings.md:435-442` — APP-08, chunk-range validation.
- `docs/09-shared-code-refactor-findings.md:501-506` — APP-17, `layout` vs `is_tiled`.
- `docs/09-shared-code-refactor-findings.md:758-763` — CAP-05, three tilings.

## Boundary / approach

Add a `girpr-chunk-layout` leaf (or widen an existing narrow leaf if a better
home is found) with a half-open range type offering `at`, `new`, `len_sat`,
`is_empty`, `is_inverted`, `contains`, `overlaps`, `gap_to`, `checked_end`, plus
one canonical tiling fold and a typed problem enum. Use T01's arithmetic
primitives.

## Guardrails

- Capture's A11 violation strings are published in `verify/selfcheck.json`.
- Repair's gap records drive disk writes; their type and decisions stay local.
- Capture's `file_size` bound is not known at its validation site and stays there.
- Signedness and published `start`/`end` keys must not change silently.
- APP-17's allocation-free fast path must be preserved.
- The latent `offset + len` overflow in `app/repair-plan/src/plan.rs:93-94` is a
  separate decision; note it, do not let it mutate contract bytes.

## Commit breakdown

1. Add leaf types, predicates, tiling fold and tests.
2. Adopt in `app/repair-plan` (`layout`, `is_tiled`, range validation).
3. Adopt in capture `assemble/ranges` and manifest validation.
4. Adopt remaining repair/fetch tuple sites, preserving gap-record shapes.

## Validation

- Byte-equality checks for `verify/selfcheck.json` and golden NDJSON.
- Repair-plan tiling/gap tests; overflow tests for signed cursors.

## Done when

Tiling, gap and overlap algebra has one implementation, and every caller still
emits its own published diagnostics and disk decisions.
