---
id: T01
title: By-name digest lookup and its test
batch: A
checkpoint: A
report: [L59-L87, L572-L575]
design_notes: "A digest is looked up by name, not by slot"
depends_on: []
size: S
risk: low
---

# T01 — By-name digest lookup

## Goal

Make every "does this row hold algorithm X" query see a known algorithm name
that was stored in the `foreign` arm under a different spelling or an
unexpected length. Add the missing mixed-case foreign test.

## Why

`DESIGN-NOTES.md` says the by-name lookup is mandatory. Today the planner,
`hashes_differ`, and `verify_digests` call `DigestSet::get_algo`, which finds a
foreign entry only by its exact canonical spelling. A row holding `MD5` with a
wrong length is therefore invisible, producing a false "must compute" or a
false coverage refusal — and, on the comparison side, a false clean.

## Files to read

- `src/hash/digestset.rs` — `get`, `get_algo`, `get_named`, `insert_named`
- `src/hash/tests.rs` — the existing wrong-length and foreign cases
- `src/planner/plan.rs` — `HashPlan::missing`
- `src/planner/pairs.rs` — `obtainable`, `pick_one`, the coverage tally
- `src/diff/mod.rs` — `hashes_differ`
- `src/commands/sync/apply/copy.rs` — `verify_digests`
- `src/cache/codec.rs` — how a foreign name is decoded

## Constraints

- Do **not** make unknown algorithm names case-insensitive. Only a name that
  parses to a known `HashAlg` may be matched by spelling variants.
- Do not change `insert_named`'s routing rule: a known name with the right
  length still goes to its slot.
- Do not change the planner's pairing, coverage policy, or `HashPlan` shape.
- Keep `get_named`'s contract for unknown names exact.

## Implementation

1. Add one private helper in `digestset.rs`, for example
   `fn foreign_known(&self, a: HashAlg) -> Option<&[u8]>`, that scans the
   `foreign` arm for an entry whose name parses to `a`. Use it from
   `get_algo` after the slot lookup. Document the deliberate widening: a
   known algorithm's foreign entry is now found whatever its spelling, and if
   more than one matches, the first recorded wins.
2. Make `get_named` use the same helper for the known-name path, so
   `get_named("md5")` and `get_algo(HashAlg::Md5)` agree on a foreign `MD5`
   entry. Keep the unknown-name path as an exact spelling match.
3. Leave the callers alone if `get_algo` now does the right thing. Verify
   there is no remaining slot-only query: `rg "\.get\("` in the planner, diff,
   and copy paths must not find a digest lookup that bypasses `get_algo` /
   `get_named`.
4. Add unit tests in `src/hash/tests.rs`:
   - a foreign `MD5` carrying 8 bytes is found by `get_algo(Md5)` and by
     `get_named("md5")`;
   - an unknown name stays exact (`Blake3` is not `blake3`);
   - the known-name-with-correct-length routing is unchanged.
5. Add the end-to-end planner test from report §5.1: a record side with a
   known algorithm under an unusual spelling and an unexpected length is
   either accepted by name (the desired answer) or refused for the documented
   coverage reason. Put it beside the existing lowercase-foreign planner
   tests. Assert the specific outcome the implementation now guarantees; do
   not write a test that passes either way.

## Commit plan

1. `fix(sync): resolve known digest names in the foreign arm`
   (`digestset.rs` + unit tests)
2. `test(sync): cover a mixed-case foreign digest on a record side`
   (planner/coverage test)

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync hash::
cargo test -p girsync planner::
cargo test -p girsync
```

## Acceptance

- A known algorithm stored under any spelling in `foreign` is found by both
  typed and named lookups.
- Unknown algorithm names remain exact and are never matched by a known
  algorithm's query.
- The new mixed-case test fails on the old code and passes now.
- No pairing/coverage policy changes.

## Reviewer focus

- Does the widening accidentally fold unknown names?
- Is the first-match rule documented?
- Does the new test assert a real outcome, not "ok or error"?
- Are `hashes_differ` and `verify_digests` still reading through the by-name
  route?
