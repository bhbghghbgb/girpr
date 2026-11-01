---
id: T05
title: Cache meta open fast path
batch: B
checkpoint: B
report: [L145-L164]
design_notes: "DESIGN-NOTES.md: meta's case flag is informational; the cache's rows are authoritative"
depends_on: []
size: S
risk: low-medium
---

# T05 — Cache meta open fast path

## Goal

Make a read/write cache open read the `meta` row before it begins a write
transaction, and skip the write entirely when the row already matches. Keep the
fresh-file and version/case-mismatch paths writing exactly as they do now.

## Why

Every read/write open currently begins a write transaction, opens both tables,
reads `meta`, and then drops the transaction when nothing changed. Dropping
avoids the commit, but the write lock and redb's page setup have already
happened. On HDD or under concurrency this is a measurable open-time cost on the
common path where the cache is already correct.

## Files to read

- `src/cache/db/cachedb.rs` — `init_meta`, `open_file`, `open_temp`,
  `open_memory_cache`
- `src/cache/open.rs` — `read_meta`, `require_current_version`, `open_db`
- `src/cache/schema.rs` — `Meta`, `CACHE_VERSION`
- `src/cache/db/cachedb.rs` tests — the read-only/version cases

## Constraints

- A fresh file must still get **both** tables (`ENTRIES` and `META_TBL`).
- The version error text must stay exactly as it is today, including the
  `sled2redb` remedy.
- A case-mode mismatch must still rewrite the row.
- A missing `meta` table must still be treated as a fresh cache, not corruption.
- Do not change read-only opens; they already never write.
- The write path may re-read the row inside its transaction. The fast path is
  an optimization, not a second source of truth.

## Implementation

1. In `init_meta`, add a read-only preflight before `db.begin_write()`:
   - call the existing `read_meta` (or a small local equivalent that preserves
     the exact `parse cache meta (corrupt? use --ignore-cache)` context);
   - if it returns `None`, fall through to the write path so both tables are
     created and the row is inserted;
   - if the version is not `CACHE_VERSION`, return the current `bail!` message
     unchanged;
   - if `case_sensitive` matches, return `Ok(())` without opening a write
     transaction;
   - otherwise fall through to the write path.
2. Keep the existing write-path logic after the preflight: open both tables,
   re-read the row, insert on `None`/case mismatch, validate version, commit
   only when changed. The re-read is deliberate: another handle or process may
   have changed the row between the preflight and the transaction.
3. Make sure `open_temp` and `open_memory_cache` still initialize a fresh
   in-memory database through the write path.
4. Add unit tests:
   - a second read/write open in the same case mode succeeds and preserves a
     row;
   - an open in the opposite case mode still rewrites `meta` (the existing
     test covers the read-only half; add the read/write assertion);
   - a fresh database still has both tables and a current-version `meta`;
   - a future-version cache is still refused with the existing message.
   Do not assert wall-clock time; the observable property is "the same row and
   the same tables, without a write".

## Commit plan

1. `perf(sync): read cache meta before beginning a write transaction`
2. `test(sync): pin the meta fast path's fresh and mismatch cases`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync cache::
cargo test -p girsync
```

## Acceptance

- A normal open with a matching `meta` performs no write transaction.
- A fresh cache still creates both tables and commits the meta row.
- A case mismatch still rewrites the row.
- A version mismatch still fails with the same message.
- Read-only opens are untouched.

## Reviewer focus

- Is there any path where the preflight returns success but a table is missing?
- Does the write path still re-check the row under its transaction?
- Is the error text preserved byte for byte where tests or users may match it?
- Does `open_temp` still create a usable fresh cache?
