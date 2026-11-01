---
id: T06
title: Copy-path round trips
batch: C
checkpoint: C
report: [L306-L342]
design_notes: "A copy is two independent reads; a failed flush is a failed copy; do not optimise those away"
depends_on: []
size: S
risk: low-medium
---

# T06 — Copy-path round trips

## Goal

Remove the two cheapest per-file filesystem round trips on the copy path:

- build `AlgoSet` once in `copy_files` and pass it down, instead of converting
  the algorithm slice inside every `copy_one`;
- set the destination mtime on the destination handle that is already open,
  instead of dropping it and reopening the file just to call `set_modified`.

Do **not** merge the destination metadata reads, and do not touch the two
independent reads or the per-file `sync_all`.

## Why

Per copied file the current path opens dst, writes, flushes, drops, reopens dst
only to stamp mtime, stats dst, and then re-reads dst for the verify. The reopen
is pure overhead on the common path, and the `AlgoSet` conversion is one
allocation-free but repeated operation per file. On a tree of many small files,
especially on Windows, both are measurable.

## Files to read

- `src/commands/sync/apply/copy.rs` — `copy_one`, `copy_and_digest`,
  `verify_digests`
- `src/commands/sync/apply/phases.rs` — `copy_files`
- `src/hash/digestset.rs` — `AlgoSet` construction/iteration
- README `### What a copy costs` and `DESIGN-NOTES.md`'s copy notes

## Constraints

- **Both reads stay.** src is digested as the copy streams; dst is re-read from
  disk after the flush and compared. A single-pass digest would compare a value
  with itself.
- **`sync_all` stays propagated.** A failed flush is a failed copy; the verify
  must not run after an ignored flush.
- Set mtime **after** the flush and **before** the dst handle is dropped. Do not
  reorder those three operations.
- Preserve every error context string in `copy_one` (open/create/copy/flush/set
  mtime/stat/hash/verify).
- Do not start merging `link_ancestor_above`, `try_is_link`, `is_dir`, and
  `hardlink::count` into a single handle helper. That is a larger change and is
  left for a follow-up; this task is only the two easy wins.

## Implementation

1. In `copy_files`, compute
   `let algos = AlgoSet::from_iter(self.common.algos.iter().copied());`
   once before `install`.
2. Change `copy_one` to take `algos: AlgoSet` (by value; it is `Copy`) and use
   it for `copy_and_digest`. Keep the slice form only where the request order or
   the `verify_digests` walk needs it. If `verify_digests` still takes `&[HashAlg]`,
   pass `&self.common.algos` at the call site, or keep the slice beside the set
   and document why.
3. Replace the reopen-and-stamp block:
   ```rust
   if let Some(st) = smtime_sys {
       fout.set_modified(st)
           .with_context(|| format!("set mtime {}", d.display()))?;
   }
   drop(fout);
   ```
   `set_modified` takes `&self`, so the existing `fout` is enough. Keep the
   `test_no_fsync` branch's flush behaviour unchanged.
4. Update the unit tests that call `copy_and_digest` only if its signature
   changes; leave the digest-of-a-copy, empty-file, reused-buffer, and
   verify-difference tests semantically identical.
5. Add or extend an end-to-end copy test only if the existing HDD fixtures do
   not already assert the preserved mtime. The observable property is “mtime
   matches src after the copy”; do not assert that no second open happened
   (that is an implementation detail).

## Commit plan

1. `perf(sync): build the copy algorithm set once per phase`
2. `perf(sync): set the copied file's mtime on its open handle`
3. `test(sync): keep the copy mtime and verify assertions explicit` (only if
   a test needs adjusting; otherwise fold into commit 2)

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync apply::
cargo test -p girsync --test sync
cargo test -p girsync
```

## Acceptance

- The copy still streams src once and re-reads dst once after the flush.
- A failed flush still aborts before the verify.
- The copied file's mtime still matches src.
- `--hash none` still uses `io::copy`'s fast path.
- The algorithm set is built once per `copy_files` call, not once per file.

## Reviewer focus

- Is `set_modified` called on a handle that is still open and after the flush?
- Did the change accidentally drop or move the dst verify re-read?
- Is `sync_all` still propagated with the same context?
- Does the `AlgoSet` value agree with the requested slice for every copy?
- Are the hard-link and link guards untouched?
