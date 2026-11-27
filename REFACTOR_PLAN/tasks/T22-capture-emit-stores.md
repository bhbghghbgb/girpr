# T22 — Capture emit and bounded-store internals

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Factor the common bodies of capture's streaming writers and bounded stores, and
inject the zstd error formatter instead of unifying error text.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:817-821` — CAP-13, deliberate zstd duplication.
- `docs/09-shared-code-refactor-findings.md:823-827` — CAP-14, two JSON+gzip writers.
- `docs/09-shared-code-refactor-findings.md:851-857` — CAP-18, bounded stores/schema reads.

## Boundary / approach

Add one `serialize_pretty_to` + `gzip_from_reader` pair with a caller-chosen
count-first vs single-serialization driver, an internal `bounded_store` helper
with injected keyed/byte-budget policy, a single `declared_schema_version`
helper, and an injected error formatter for the zstd measurement loops.

## Guardrails

- `BufWriter` placement is measured (12 s vs 46 s) and byte-equality is pinned;
  only the common body may be factored.
- The zstd error strings are deliberately distinct and pinned by offline-bundle
  tests; the boundary is inside `gircap-hash`, not a workspace crate.
- Row order and the `omitted`/`total_retained_bytes` fields are published.
- The three stores genuinely differ; inject their policies rather than merging them.

## Commit breakdown

1. Factor the common serialize/gzip body, keeping both drivers.
2. Add `bounded_store` with injected policy; adopt the three stores.
3. Add `declared_schema_version`; adopt schema reads.
4. Inject the zstd error formatter in `gircap-hash`.

## Validation

- Byte-equality goldens for all emitted documents.
- Pinned error-string tests; `cargo nextest` for capture and gircap-hash.

## Done when

Common bodies are shared, published bytes and error strings are unchanged, and
the three stores keep their distinct policies.
