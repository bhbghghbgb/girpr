# P1-13 — Record repeated response headers

Report: `../../OPTIMIZATIONS_REPORT.md` lines 264-276 (A12).

Dependencies: P0-02.

## Goal

Stop losing all but the last value of a repeated response header. A response
with repeated `Set-Cookie`, `Link`, `Warning`, or `Cache-Control` must keep
every value in `api/requests.ndjson`.

## Where to start

- `src/http/record.rs`: `header_map` (around line 172), `RequestRecord`
  (`resp_headers: BTreeMap<String, String>`), `req_headers`.
- `src/http/fetcher.rs`: the two `header_map(resp.headers())` call sites and
  `record(...)` that takes `resp_headers`.
- `src/http/tests.rs`: header recording and JSON-shape tests.
- `docs/01-artifact-schema.md` for the documented header value form.

## Decision

Keep the document shape (`object<string,string>`) and join repeated values with
a documented delimiter. Use `\n` (a header value cannot contain a raw newline,
so it is unambiguous and does not collide with comma-bearing `Set-Cookie` or
`Cache-Control` values). State the delimiter in the schema doc and in
`header_map`'s comment. A single value is byte-identical to today.

If the implementer finds a consumer that treats the value as a single header
and cannot tolerate a newline, stop and return the tradeoff instead of changing
the document shape to arrays without approval.

## Steps

1. Iterate the `HeaderMap` by key, not by value. `HeaderMap::iter()` yields each
   value; collect all values per name.
2. For each name, convert each value with the existing UTF-8-or-hex rule, then
   join with `"\n"`. Preserve the existing hex fallback for non-UTF-8 bytes.
3. Keep keys lowercase and the `BTreeMap` sorted output.
4. Update the doc comment: a repeated header is one key with newline-joined
   values; the order is the order the map yields, which is insertion order for
   repeated values.
5. Update `docs/01-artifact-schema.md` where `resp_headers` is described.
6. Do not change `req_headers` construction; it has one value per header.

## Tests

- Unit: a `HeaderMap` with two `set-cookie` values records one key and both
  values joined by `\n`.
- Unit: a single header value is unchanged.
- Unit: a non-UTF-8 value still records hex (existing test).
- JSON-shape test: a repeated header round-trips as one string with the
  documented delimiter.
- Flip the P0-02 characterization test.

## Acceptance

- Response evidence keeps every value of a repeated header.
- Single-value headers and the document shape are unchanged.
- The delimiter is documented.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(http): keep every value of a repeated response header`
2. `test(http): cover repeated and single-valued response headers`

## Notes

- Do not change request headers or the fetcher's retry logic.
- Do not reference the report or plan in code comments.
