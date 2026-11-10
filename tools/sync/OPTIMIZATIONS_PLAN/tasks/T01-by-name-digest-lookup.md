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

Make `get_algo` find a known algorithm's foreign entry under **any** ASCII-case
spelling, and make `get_named` agree with it.

## Why

A row may carry a digest under a name this build knows but under a different
case — `MD5` rather than `md5`. `insert_named` routes it to the foreign arm
because its length is unexpected. That row is then invisible to the typed
lookup:

- `get_algo(Md5)` matches the foreign arm by the **canonical** spelling only,
  so it never sees a foreign `MD5`.
- `get_named("MD5")` finds it, because the foreign arm is matched by the
  spelling it was given. But `get_named("md5")` — the spelling the planner
  would ask for — does not.

So the row is found by one spelling and not the other, and the planner, which
calls `get_algo`, sees a row it believes holds nothing. The result is a false
"must compute" (benign but wasteful) or, on a record side, a false coverage
refusal.

The fix has a natural shape here. `HashAlg::from_str` already parses ASCII-case
insensitively, so "MD5" and "md5" are the same algorithm to the parser.
Resolving a foreign entry by **parsing its stored name** and comparing the
parsed algorithm makes the two lookups agree, matches the crate's
ASCII-case-only fold everywhere else, and still cannot match an unknown name:
a name that does not parse to an algorithm is not an algorithm.

## Files to read

- `src/hash/digestset.rs` — `get`, `get_algo`, `get_named`, `insert_named`
- `src/hash/algo.rs` — `HashAlg::from_str`, and its ASCII-case-insensitive
  parse
- `src/hash/tests.rs` — the existing wrong-length and foreign cases
- `src/planner/plan.rs` — `HashPlan::missing`
- `src/planner/pairs.rs` — `obtainable`, `pick_one`, the coverage tally
- `src/diff/mod.rs` — `hashes_differ`
- `src/commands/sync/apply/copy.rs` — `verify_digests`
- `src/cache/codec.rs` — how a foreign name is decoded

## Constraints

- **Unknown names stay exact.** A foreign name that does not parse to a known
  `HashAlg` must not be matched by any known-algorithm lookup.
- The fold is **ASCII case only**, matching `util::path_key` and
  `HashAlg::from_str`. Do not introduce a Unicode `to_lowercase` comparison
  here: it would map the Kelvin sign `K` onto `k` and merge two spellings the
  filesystem keeps apart.
- Do not change `insert_named`'s routing rule: a known name with the right
  length still goes to its slot, whatever its case.
- Do not change the planner's pairing, the coverage policy, or `HashPlan`'s
  shape.
- `get_named`'s contract for an **unknown** name stays exact.
- The first match wins if more than one foreign entry resolves to the same
  algorithm. Document that; do not make it order-dependent in a way callers
  can observe.

## Implementation

1. Add one private helper in `digestset.rs`:

   ```rust
   fn foreign_known(&self, a: HashAlg) -> Option<&[u8]>
   ```

   It scans the foreign arm for the first entry whose stored name parses to
   `a`, and returns that digest. Parsing is `HashAlg::from_str`, so the match
   is ASCII-case-insensitive and an unknown name never matches.

   Document the widening: a known algorithm's foreign entry is now found
   whatever its ASCII case, and the first such entry wins.

2. Use the helper in `get_algo`, replacing the canonical-spelling match.

3. Use it in `get_named` for the known-name path, so `get_named("md5")` and
   `get_algo(Md5)` agree. The unknown-name path stays an exact spelling
   match.

4. Check the callers. `rg "\.get\("` in the planner, diff, and copy paths must
   not find a digest lookup that bypasses `get_algo` / `get_named`.

5. Add unit tests in `src/hash/tests.rs`:
   - a foreign `MD5` carrying 8 bytes is found by `get_algo(Md5)` and by
     `get_named("md5")`;
   - an unknown name stays exact (`blake3` is not `blake3`);
   - the known-name-with-correct-length routing is unchanged;
   - a foreign name that differs only in a **non-ASCII** way is still not
     matched — the Kelvin-sign case, so a Unicode fold cannot creep in.

6. Add the end-to-end planner test from report §5.1: a record side whose
   known algorithm is stored under an unusual spelling and an unexpected length
   must be **accepted by name**, not refused for coverage. Put it beside the
   existing lowercase-foreign planner tests.

   Assert that specific outcome. A test that passes whether the planner
   accepts or refuses the row tests nothing.

## Commit plan

1. `fix(sync): resolve known digest names in the foreign arm`
   (`digestset.rs` + unit tests)
2. `test(sync): cover a mixed-case foreign digest on a record side`
   (planner/coverage test)

## Verification

```text
cargo fmt --all -- --check
cargo nextest run -p girsync hash::
cargo nextest run -p girsync planner::
cargo nextest run -p girsync
```

## Acceptance

- A known algorithm stored under any ASCII-case spelling in the foreign arm is
  found by both the typed and the named lookup.
- Unknown algorithm names remain exact and are never matched by a known
  algorithm's query.
- The non-ASCII lookalike case is still not matched.
- The new mixed-case test fails on the old code and passes now.
- No pairing or coverage policy changed.

## Reviewer focus

- Does the widening accidentally fold unknown names?
- Is the fold ASCII-only, or did a Unicode `to_lowercase` creep in?
- Is the first-match rule documented?
- Does the new test assert a real outcome, not "ok or error"?
- Are `hashes_differ` and `verify_digests` still reading through the by-name
  route?
