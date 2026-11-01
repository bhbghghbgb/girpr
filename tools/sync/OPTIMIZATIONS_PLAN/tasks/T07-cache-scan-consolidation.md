---
id: T07
title: Cache scan consolidation
batch: C
checkpoint: C
report: [L280-L304, L580-L583]
design_notes: "Same rows, same commit points; a read must not move across a write"
depends_on: []
size: S
risk: low-medium
---

# T07 — Cache scan consolidation

## Goal

Remove the N+1 read pattern in the rename re-key pass: read every moved row in
one read transaction instead of calling `CacheDb::get` once per key. Add a
small cache API for that, and a test that a no-op run does not scan the cache
more than once where the code can observe it.

The broader idea of combining `prune_dst_cache`, `predrop_cache_entries`, and
the rename scan into one pass is explicitly **not** in this task: those passes
straddle writes, and combining them would move a read across a write.

## Why

`rename_to_src_casing` currently does `load_keys`, then `get` per changed key,
then a write transaction. On a large cache that is one read transaction per
renamed path. The row set and commit point are unchanged if those reads share
one transaction.

## Files to read

- `src/commands/sync/rename.rs` — the re-key block after `case_renames`
- `src/cache/db/cachedb.rs` — `get`, `load_keys`, `load_all`
- `src/cache/db/handle.rs` — read-transaction helpers
- `src/commands/sync/apply/phases.rs` — `prune_dst_cache`
- `src/commands/sync/apply/mod.rs` — `predrop_cache_entries`

## Constraints

- The rename write transaction must still begin **after** all reads and must
  contain the same `remove`/`put` pairs in the same order.
- Do not decode rows a caller does not need. `load_keys` must stay key-only.
- A missing `ENTRIES` table stays an empty answer, not an error.
- `prune_dst_cache` and `predrop_cache_entries` keep their own scans and their
  own commit points; only reads *within one pass* may be consolidated.
- Do not change `DiskSpelling`, `MoveIndex`, or which rows move.

## Implementation

1. Add a read-transaction API on `CacheDb`, for example:
   ```rust
   pub fn get_many<'a>(&self, rels: impl IntoIterator<Item = &'a str>)
       -> Result<Vec<(String, Option<FileRec>)>>
   ```
   It opens one read transaction, opens `ENTRIES` once, and returns the decoded
   row for each requested key. A missing table means every answer is `None`.
   Keep `get` as the one-key convenience wrapper over the same logic, or leave
   it and have `get_many` share a private helper.
2. In `rename_to_src_casing`, first compute the `(from, to)` list from
   `final_moves`, then call `get_many` over the `from` keys, then build the
   `moves` vector and start the write transaction. Do not call `dst_db.get`
   inside the loop.
3. Add tests:
   - `get_many` returns exactly what a loop of `get` calls returns, including
     an absent key and a missing table;
   - a rename fixture with several keys still moves every row and leaves no
     old key behind;
   - a no-op sync (empty plan, unchanged tree) does not call the rename re-key
     path at all. If instrumentation is needed, use a test-only counter or a
     log assertion rather than wall-clock timing. The report's §5.3 calls
     cache-scan counts the observable property.
4. Leave `remove_unknown_dirs`' scan and `prune_dst_cache`' scan as they are.
   Add a short comment to `cache/db/cachedb.rs` explaining that `get_many` is
   for callers that need values and that a key-only caller must use
   `load_keys`.

## Commit plan

1. `perf(sync): read renamed cache rows in one transaction`
2. `test(sync): pin get_many and the no-op cache scan`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync cache::
cargo test -p girsync rename::
cargo test -p girsync
```

## Acceptance

- The rename re-key does one read transaction for all moved rows.
- The same rows move and the same commit point remains.
- `load_keys` still decodes nothing.
- A no-op run does not gain an extra cache scan.
- `prune` and `predrop` are not merged across their writes.

## Reviewer focus

- Does `get_many` hold one read transaction across every key, including a
  missing key?
- Is the write transaction still opened only after the reads finish?
- Could a key be read from a state the old per-key loop would not have seen?
- Is any full-table decode introduced where only keys were needed?
- Does the no-op test observe a count rather than a duration?
