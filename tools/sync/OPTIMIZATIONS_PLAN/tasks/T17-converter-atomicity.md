---
id: T17
title: Converter atomicity and failure tests
batch: G
checkpoint: G
report: [L166-L189, L587-L589]
design_notes: "A failed conversion must not leave a destination that looks valid; --force must not destroy the previous destination before the new one is complete"
depends_on: []
size: M
risk: medium
---

# T17 — Converter atomicity and failure tests

## Goal

Make `sled_to_redb` write the new database to a temporary sibling, then move it
into place only after the conversion has fully succeeded. A failed conversion
must leave the previous destination untouched (under `--force`) or absent
(without it), and must not leave a valid-looking partial destination.

## Why

The current code, under `force`, removes the destination **before** it opens
the sled source and converts. A corrupt source or a failed write therefore
destroys the previous cache and leaves no replacement. Report §1.5/§5.5 asks
for atomicity and for tests that pin both halves.

## Files to read

- `src/convert.rs` — `sled_to_redb`, the recs collection, `open_db`, the write
  batch
- `src/cache/open.rs` — `remove_cache_path`, `open_db`
- `src/cache/db/cachedb.rs` — `open_file`, `init_meta`
- `tests/convert.rs` — the existing success/force/missing-source cases
- `README.md` — the converter note and legacy-cache remedy

## Constraints

- Keep the source sled directory untouched.
- Without `--force`, an existing destination is still refused **before** any
  work.
- With `--force`, the old destination must remain readable until the new file
  is complete and ready to replace it.
- The final replacement must be a rename/move when the destination is a file.
  If the destination is a legacy sled **directory**, remove it only after the
  new file is complete, then move; document that this step is not atomic.
- A failed conversion must clean up its temporary sibling. Do not leave
  `girpr-cache.tmp*` behind, and do not let the temp name look like a live
  `girpr-cache` (it must not be auto-read as a cache).
- Do not change the converted rows, codec, counts, or the `Converted` summary.
- Streaming the sled rows into the temp DB instead of collecting `recs` is
  allowed but not required. If you keep the `Vec`, say why in a comment; the
  atomicity is the goal.

## Implementation

1. Refuse a missing source and an existing destination without `--force`
   before creating anything, as today.
2. Build a temporary sibling next to the destination, for example
   `parent/.girsync-convert-<pid>-<ts>.redb`, claimed with
   `OpenOptions::create_new`. Do not use a name beginning with
   `girpr-cache`; `scan.rs` reserves that prefix.
3. Convert into that temp path:
   - read the sled rows and decode them (keep the `Vec` or stream; either is
     fine);
   - `open_db(temp, case_sensitive, ReadWrite { ignore_cache: false, backup_first: false })`;
   - write every row, commit, and **drop the cache handle** so the redb file
     lock is released before the rename.
4. Replace the destination:
   - if it is a file, `std::fs::rename(temp, dst)` (replaces on Unix and
     Windows);
   - if it is a directory under `--force`, `remove_cache_path(dst)` and then
     `rename`;
   - if the rename fails, remove the temp and return the error, leaving the old
     destination in place when it was a file.
5. On any error before the rename, remove the temp file (best effort; report a
   cleanup failure beside the primary error the way `rename.rs` does) and
   return.
6. Add tests in `tests/convert.rs`:
   - a source with an invalid hex digest or invalid kind fails, the destination
     is **absent** (no `--force`), and no temp sibling remains;
   - with a valid existing destination and `--force`, a failing conversion
     leaves the old destination byte-identical and readable;
   - a successful `--force` conversion replaces the old destination and leaves
     no temp sibling;
   - a successful no-force conversion creates the destination and no temp.
   Use the existing `build_sled_db` helper and add one variant that writes a
   bad row.
7. Update `README.md`'s converter note: conversion is atomic for a file
   destination; a failure leaves the previous cache in place under `--force`;
   the legacy-directory replacement is the non-atomic case.

## Commit plan

1. `fix(sync): write the converted cache to a temp sibling and rename`
2. `test(sync): pin conversion failure and --force preservation`
3. `docs(sync): document converter atomicity`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync --test convert
cargo test -p girsync
```

## Acceptance

- A failed conversion never leaves a destination that opens as a valid cache.
- Under `--force`, a failed conversion leaves the previous destination intact.
- A successful conversion is a single rename for a file destination.
- No temp sibling survives success or failure.
- Counts, rows, and the summary record are unchanged.

## Reviewer focus

- Is the destination removed anywhere before the new file is complete?
- Is the redb handle dropped before the rename (file lock)?
- Can the temp name be picked up by `walk_live`/`is_cache_rel`?
- Does the error path clean up the temp without hiding the primary error?
- Do the tests fail on the old implementation (especially the `--force`
  preservation case)?
