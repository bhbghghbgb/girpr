---
id: T13
title: Decompose copy_one around its guards
batch: E
checkpoint: E
report: [L437-L456]
design_notes: "Every copy guard is load-bearing; a destination with more than one hard link is refused before truncation"
depends_on: [T06]
size: M
risk: medium
---

# T13 — Decompose `copy_one` around its guards

## Goal

Split `copy_one` into a readable pipeline of private helpers without changing
the order of the checks, the error text, or the guard semantics:

1. link-ancestor containment;
2. final-component link containment;
3. destination type cleanup;
4. hard-link refusal;
5. the copy / flush / stamp / verify pipeline.

The reader should be able to see “guard, prepare, copy, flush, stamp, verify”
without reconstructing it from comments.

## Why

`copy_one` mixes five concerns and interleaves their control flow. Every step is
load-bearing, and the report warns that reordering the “remove destination
directory” and “count hard links” steps changes what is protected. Extraction
gives future safety checks one insertion point, but it must not make accidental
reordering easy.

## Files to read

- `src/commands/sync/apply/copy.rs` — `copy_one` and its tests
- `src/commands/sync/apply/phases.rs` — the caller and its permit ordering
- `src/link.rs` — `link_ancestor_above`, `try_is_link`, `remove`
- `src/hardlink.rs` — `count`, `info`
- `src/util.rs` — `path_within`, `mtime_ns_of`
- Existing sync tests for link ancestry, hard links, type conflicts, mtime,
  flush failure, and verify mismatch

## Constraints

- **Preserve order exactly.** In particular:
  - the link-ancestor check precedes the final-component link check;
  - the destination-directory cleanup precedes `create_dir_all`;
  - the hard-link count happens after the directory cleanup and before
    `File::create`;
  - the src stat precedes the src open;
  - the dst verify re-read happens after flush and stamp.
- Preserve every error string byte-for-byte, including the contextual
  `with_context` wrappers. Tests and operators match on these.
- Keep the T06 improvements: `AlgoSet` passed in, mtime stamped on the open
  handle.
- The helpers are **private to `copy.rs`**. No trait, no generic copy strategy.
- Do not merge the two independent reads or the `sync_all`.
- Do not change `copy_files`'s permit boundary or cache-row timing.

## Implementation

1. Extract, in source order:
   - `fn refuse_link_ancestor(dst_root: &Path, rel: &str) -> Result<()>`
   - `fn refuse_final_link_into_src(d: &Path, rel: &str, src_canonical: &Path) -> Result<()>`
   - `fn clear_blocking_dir(d: &Path, rel: &str) -> Result<()>` (the
     `is_dir` + `try_is_link` + unlink/remove_dir_all block)
   - `fn refuse_hard_linked_dst(d: &Path, rel: &str) -> Result<()>`
   - `fn copy_and_stamp(...) -> Result<VerifyInputs>` or keep the pipeline
     inline in `copy_one` if a return type would obscure it. The goal is
     readable grouping, not a type for its own sake.
2. Keep `copy_one` as the ordered call list:
   ```rust
   refuse_link_ancestor(...)?;
   let s = ...; let d = ...;
   refuse_final_link_into_src(...)?;
   clear_blocking_dir(...)?;
   create_parent(...)?;
   refuse_hard_linked_dst(...)?;
   let smeta = ...;
   // open, create, copy, flush, stamp, drop
   // verify size/mtime/hash
   ```
3. Use a small private `struct SourceStat { size: u64, mtime_sys: Option<SystemTime>, mtime_ns: i64 }`
   only if it makes the stat-to-verify hand-off clearer; otherwise pass the
   three locals as today.
4. Add unit tests for the extracted guards where a temp fixture is cheap:
   - a final-component link into `src_canonical` is refused with the same text;
   - a directory at the destination is removed, a link to a directory is
     unlinked rather than recursed;
   - a hard link count above one is refused before any `File::create`.
   The existing end-to-end sync tests remain the primary coverage; do not
   duplicate them wholesale.
5. Leave `copy_and_digest`, `verify_digests`, `flush_file`, and the test hooks
   untouched.

## Commit plan

1. `refactor(sync): extract copy_one's link and type guards`
2. `refactor(sync): extract copy_one's hard-link guard and copy pipeline`
3. `test(sync): pin the extracted copy guards' order and messages`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync apply::
cargo test -p girsync --test sync
cargo test -p girsync
```

## Acceptance

- `copy_one` reads as guard -> prepare -> copy -> flush -> stamp -> verify.
- Check order and error text are unchanged.
- The two reads and the propagated flush remain.
- Helpers are private and have no trait/generic layer.
- `copy_files` is unchanged except for the call signature.

## Reviewer focus

- Can any two guards be reordered by the new call list?
- Does the hard-link check still happen after the directory cleanup and before
  `File::create`?
- Is the mtime stamped on the still-open handle after the flush?
- Did any `with_context` string change?
- Are the link and hard-link tests still exercising the real path?
