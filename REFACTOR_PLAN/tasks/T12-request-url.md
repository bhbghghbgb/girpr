# T12 — Request URL assembly and query rewrite leaf

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Consolidate the HYP/Sophon query grammar and CDN path-segment append into one
request-URL leaf, preserving the literal query spelling used by recorded
requests.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:189-200` — X10, URL assembly.
- `docs/09-shared-code-refactor-findings.md:428-433` — APP-07, CDN path append.
- `docs/09-shared-code-refactor-findings.md:733-739` — CAP-01, HYP/Sophon URLs.
- `docs/09-shared-code-refactor-findings.md:765-771` — CAP-06, blob-id shape rules.
- `docs/09-shared-code-refactor-findings.md:844-849` — CAP-17, query lookup/rewrite.

## Boundary / approach

Add a `girpr-request-url` leaf with pair building, encoding, URL build and query
value replacement over the shared `QUERY_VALUE_SET`, plus a URL path-segment
append helper. Share only the grammar mechanics.

## Guardrails

- Literal `game_ids[]` key, separator choice, percent-encoding, trailing-slash
  trim and the ordered `branch, package_id, password, tag` pairs are the grammar.
- `Biz::resolve`, base overrides, `needs_channel`, prefix selection and girpr's
  unencoded/hardcoded-language spelling stay at call sites.
- Golden URLs and recorded NDJSON rows are byte contracts. A `Url`-based parser
  would escape `game_ids[]`; do not use one for the query rewrite.
- Tightening girpr's blob-id shape to capture's strict rule could reject real
  ids; rejection rule IDs and `Url::join` assertions stay local.
- Error types and prefix selection stay local.

## Commit breakdown

1. Add leaf with pair/encode/build/replace + tests for the literal key.
2. Adopt in `app/fetch/src/hyp.rs`.
3. Adopt in capture `biz` and `http/probe.rs`.
4. Adopt path-segment append in `app/dispatch` and `app/persistent-plan`.
5. Adopt query lookup/rewrite helpers in capture/test-fixture callers.

## Validation

- Golden URL and recorded-request byte tests for both programs.
- `cargo nextest` for fetch, capture, dispatch, persistent-plan.

## Done when

URL assembly and query rewriting have one implementation and every recorded URL
is unchanged.
