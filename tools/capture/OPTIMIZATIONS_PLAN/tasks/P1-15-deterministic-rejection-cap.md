# P1-15 — Deterministic rejection cap

Report: `../../OPTIMIZATIONS_REPORT.md` lines 296-312 (A14) and 928-948
(tradeoffs).

Dependencies: P0-02.

## Goal

Make `diagnostics/rejections.json` deterministic under concurrent manifest
fetching. Today `Rejections::record` keeps the first `MAX_ROWS` arrivals and
counts the rest, so which rows survive depends on scheduling.

## Where to start

- `src/rejections.rs`: `Rejections`, `State { rows, omitted }`, `record`
  (around line 111), `snapshot`, `document`, `sorted`, `cap_value`,
  `MAX_ROWS`, tests.
- Callers: `src/manifest/fetch.rs` and manifest decode paths; concurrency comes
  from `fetch_all`.
- `docs/01-artifact-schema.md` for the rejection document's `total`/`omitted`
  contract.

## Decision

Use a bounded deterministic top-k by the same key the document already sorts
by: `(source, value, rule)`. Keep the smallest `MAX_ROWS` keys. This is bounded
memory and deterministic regardless of arrival order. Ties (identical rows)
are interchangeable; treat them as one key for retention and count extras as
omitted.

## Steps

1. Keep `rows` sorted by `(source, value, rule)` at all times, or insert with
   `binary_search` and pop the largest when over capacity.
2. On `record`:
   - build the row and its key;
   - if `rows.len() < MAX_ROWS`, insert it in key order;
   - else if the new key is smaller than the current maximum, remove the
     maximum, increment `omitted`, and insert the new row;
   - else increment `omitted`.
3. `total` should remain the number of rows recorded, including omitted.
   `omitted` should remain the number not retained. Confirm the schema's
   definitions and keep them.
4. `snapshot`/`document` can return the already-sorted rows; do not sort twice
   unless the data structure is not maintained sorted.
5. Update the comment in `record`: the cap is "smallest keys", not "first
   arrivals".
6. If `Rejection` grows fields later, the key must stay explicit, not derive
   from all fields accidentally.

## Tests

- Feed `MAX_ROWS + N` distinct rows in reverse and forward order; both produce
  the same retained rows and the same `omitted`.
- A new row smaller than the current maximum displaces it; a larger row is
  omitted.
- Duplicate identical rows with a full store increment `omitted` and do not
  grow the document.
- Existing `snapshot`/`document` tests still pass.
- If a concurrency test exists, two interleavings must agree.

## Acceptance

- Two captures of the same hostile input publish the same rejection sample
  regardless of fetch completion order.
- Memory stays bounded by `MAX_ROWS`.
- `total`/`omitted` remain truthful.
- `cargo test -p gircap` passes.

## Commit plan

1. `fix(rejections): keep a deterministic bounded top-k sample`
2. `test(rejections): cover arrival-order independence and displacement`

## Notes

- Sorting all rows before capping would be unbounded memory; do not do that.
- Do not change `cap_value` or the row fields.
- Do not reference the report or plan in code comments.
