# Sixth adversarial pass — `girsync` (`tools/sync`)

Scope: `tools/sync/src/**`, `tools/sync/tests/**`, `README.md`, `DESIGN-NOTES.md`,
`Cargo.toml`, on branch `fix-gaps/girsync` at `9442823` (tree clean before this
note). Judged against the crate's stated principles: strict validation,
fail-fast, and no silently omitted errors.

Method: read the current tree (not only the branch diff), re-ran the recorded
reproductions against the current binary, probed filter/plan/apply parity with
new fixtures, and ran the full verification suite. No source, test, README,
DESIGN-NOTES, or Cargo file was changed; this note is the only added file.

## Verdict

| ID | Severity | Area | Verdict |
| --- | --- | --- | --- |
| P6-1 | medium | plan/apply parity, filters | **Valid, open.** `ensure_dirs_applicable` reads the filtered dst map, so a filtered-out non-directory at a strict ancestor is invisible; the dry run prints a plan that apply cannot execute (`os error 183`). |
| P6-2 | high (data loss) | hard links | **Valid, open — should be closed.** A dst hard link is written through by `File::create`; the run exits 0 after destroying the bytes of every other name for that inode, including a src file. |
| P6-3 | low | docs | **Valid, open.** `README.md` still claims 352 test functions (351 run); the tree has 374 (373 run by default). |

The fifth pass's fixes P5-1 (`fix_dirs` before `mkdirs`) and P5-2
(`validate_dst_links` plus the final-component recheck in `copy_one`) hold for
the cases they cover; no regression was found in the new link, rename, apply,
or cache-pruning code beyond the hard-link class below. The branch is **not
sound as-is** because of P6-1 and P6-2.

## P6-1 — A filtered-out dst blocker at an ancestor is invisible to the plan guard

**Reproduction** (`E:\Symlinks\Temp\rev6-filter-final`, current binary,
`GIRSYNC_TEST_CACHE=memory`): `src/foo/bar.txt`, and `dst/foo` is a regular file
containing `blocker`.

```
girsync sync --src ...\src --dst ...\dst --include "*.txt" --dry-run
MISSING foo
MISSING foo/bar.txt
MKDIR foo
COPY foo/bar.txt because=missing
SUMMARY renamed=0 mkdir=1 copied=1 deleted=0 rmdir=0 ... dry_run=true
DRY_EXIT=0

girsync sync --src ...\src --dst ...\dst --include "*.txt"
MISSING foo
MISSING foo/bar.txt
ERROR girsync: fatal error="mkdir ...\dst\foo: Cannot create a file when that
file already exists. (os error 183)"
REAL_EXIT=3
```

`dst/foo` is untouched (still the 7-byte file). The failure is at the first
apply phase, after the dry run has already promised the plan would land.

**Why the guard misses it.**

- `build_plan` plans `mkdir` for every src directory absent from `dm`
  (`plan.rs:236-241`). With `--include "*.txt"`, `foo` does not match, so it is
  absent from `dm`, while `sm` keeps it because it is an ancestor of the
  included file.
- `ensure_dirs_applicable` (`plan.rs:373-394`) walks the strict prefixes of each
  planned path and consults **`dm`** (`dm.get(prefix)`), not the disk. A filtered
  path is absent from `dm`, so the guard sees no blocker.
- The ancestor retention that keeps `foo` in `sm` deliberately keeps only
  **directories** (`effective/phase_a.rs:87-111`): the `dirs` set is built from
  entries with `is_dir`, so a non-directory ancestor is never pulled back in.
  The filter removes it on both sides (`filter.rs:29-67`).
- Apply then calls `create_dir_all` (`apply/mod.rs:113-128`, line 118), which
  fails with `AlreadyExists` when the path is a file.

The guard's own premise (`plan.rs:364-369`) is "a file ancestor of a src path is
by construction a type conflict". That is true for an unfiltered tree; a filter
breaks the premise precisely because it hides the file from both the planner and
the guard. The comment above the call (`commands/sync/mod.rs:333-341`) says a
plan that cannot be applied must not be printed by a dry run; P6-1 is a
counterexample.

**Scope.** The blocker must be a non-directory, non-link entry. An excluded dst
*directory* at the ancestor is harmless: `create_dir_all` is a no-op and the run
succeeds (though it reports `MKDIR` for a directory that already existed — see
observations). An excluded dst *link* at the ancestor is caught earlier by
`ensure_link_safe`, which stats the disk (`plan.rs:316-353`). The bug applies to
any strict prefix (not only the first), and to `--missing-only` as well, since
`mkdir` is still planned there. `--keep-extra` does not affect it.

**Suggested fix.** Ask the disk, as `ensure_link_safe` already does. For each
`mkdir`/`copy`/`fix_dirs` rel, `symlink_metadata` each strict prefix under `dst`;
if a prefix exists and is not a directory, refuse at plan time with a message
that names the filtered path and the remedy (remove it, `--exclude` it, or widen
the filter). For `mkdir`, also check the action path itself: it is exactly the
path that `create_dir_all` will touch. Do **not** fix this by retaining
non-directory ancestors in `dm`: the planner would then see a type conflict and
delete a dst entry the filter promised to leave untouched.

Two implementation notes:

- The check runs after `rename_to_src_casing` (`mod.rs:292` vs `341`), so in
  insensitive mode a refusal can follow a case-only rename. That is not data
  loss, but it contradicts the guard's "before the first filesystem change"
  wording (`plan.rs:367-369`); either accept it and amend the wording, or run a
  stat-based preflight before the rename.
- Add a regression test with `src/foo/bar.txt`, `dst/foo` a file, and
  `--include "*.txt"`: both `--dry-run` and the real run must exit 3 at plan
  time, name `foo`, and leave `dst/foo` unchanged. The existing unit tests
  (`plan.rs:552,568`) cover only the unfiltered map and therefore pass today.

## P6-2 — A dst hard link is written through, destroying the shared inode

**Reproduction** (`E:\Symlinks\Temp\rev6-hard-final`): `src/a.txt = "AAAA"`,
`src/b.txt = "BBBBBBBBBB"`, `cmd /c mklink /H dst\a.txt src\b.txt`, then
`sync --exclude b.txt` (the exclude keeps the run from re-copying `b.txt` and
turning the corruption into a verify failure):

```
{"event":"changed","path":"a.txt"}
{"because":"changed","event":"copy","path":"a.txt"}
{"copied":1,...,"event":"summary",...}
EXIT=0

src/a.txt = AAAA
src/b.txt = AAAA          <- was BBBBBBBBBB
fsutil hardlink list src\b.txt
\Symlinks\Temp\rev6-hard-final\src\b.txt
\Symlinks\Temp\rev6-hard-final\dst\a.txt
```

The run reports success while truncating the file that `dst/a.txt` shares an
inode with. Without the exclude the source is still destroyed; the run may then
fail verification or converge to the wrong bytes depending on order.

**Why the existing guards miss it.** A hard link is not a reparse point, so
`link::try_is_link` returns false (`link.rs:53`); `validate_src_links` and
`validate_dst_links` only canonicalize entries with `is_link`
(`commands/sync/mod.rs:482,536`); `ensure_link_safe` only looks for link
ancestors; and `copy_one`'s final-component check is also `try_is_link`
(`copy.rs:204-225`). `File::create(&d)` (`copy.rs:252`) then truncates the
inode. Verification reads the same inode back (`copy.rs:281-304`), so size and
digest both agree and the run exits 0. Under `--hash none` the size+mtime arm
also passes, because the mtime is set on the shared inode (`copy.rs:275-279`).

This is broader than "into src": a dst hard link to any file outside `dst`
(e.g. another checkout) is truncated just the same.

**Decision: close it, not merely document it.** A mirror that exits 0 after
destroying its own input violates fail-fast and the tool's central promise; the
fifth pass correctly recorded the hazard (`ADVERSARIAL_REVIEW.md:1161-1176`) but
left it outside the fix scope, and `README.md:1032-1050` does not mention hard
links at all. If the maintainer consciously declines to close it, the minimum is
an explicit README non-guarantee; that is a fallback, not the decision.

**Suggested fix.** Before truncating, compare the identity of `d` with the
source file being copied:

- Unix: `std::os::unix::fs::MetadataExt::{dev, ino}` is stable and cheap.
- Windows: `std::os::windows::fs::MetadataExt::{volume_serial_number,
  file_index}` are still behind the unstable `windows_by_handle` feature on
  rustc 1.98.1 (verified: a scratch compile fails with `E0658`), so closing this
  needs either a `windows-sys` dependency calling `GetFileInformationByHandle`
  (or `GetFileInformationByHandleEx`) or a conservative refusal when the link
  count is greater than one.

A structural alternative is to copy to a temporary file in `dst` and rename it
over the target: rename replaces the directory entry, so the other name for the
inode survives. That also improves crash safety, but it changes the documented
"truncate + write in place" and the sanctioned dst-file-link write-through, so it
is a design decision rather than a drop-in guard.

Regression test: create the hard link with `std::fs::hard_link` (no privilege
needed on this host; `mklink /H` is equivalent), run with `--exclude b.txt`, and
assert exit 3 with a message naming the shared inode and `src/b.txt` unchanged.
Also assert the same refusal under `--hash none`.

## P6-3 — README test count is stale

`README.md:1103` says:

```
cargo nextest run -p girsync   # 352 test functions, one of them #[ignore]d (351 run by default)
```

Current tree: `cargo nextest list -p girsync` lists 373, and
`--run-ignored all` lists 374; `cargo test -p girsync` reports 373 passed and 1
ignored. `git log -S "352 test functions"` shows the number was introduced at
`ee04b1d`, when the tree did have 352; 22 tests were added afterwards without
updating it. Fix: 374 functions, 373 run by default. The count is the only stale
part of that paragraph; the nextest advice itself is correct.

## Surviving decisions and observations

- **`--max-depth` still skips silently.** Documented (README, `--max-depth`) and
  now pinned end-to-end (`tests/sync.rs:772`). This pass treats it as the
  previously accepted F-4 decision, not a new finding, but it remains the one
  place a successful run can omit content by design.
- **An excluded dst directory at an ancestor reports `MKDIR` without creating
  anything.** `create_dir_all` is a no-op on an existing directory, so the run
  succeeds and the dry/real records agree; only the claim "created" is
  inaccurate. Folding a `symlink_metadata` check into P6-1's fix would let the
  planner report it correctly (or refuse), but it is not a parity failure.
- **`ensure_dirs_applicable` runs after the rename pass** (`mod.rs:292` then
  `341`). Today the guard's premise makes it unreachable on an unfiltered tree,
  so the ordering has no consequence; once P6-1 makes it fire, a refusal can
  follow case-only renames. Decide the ordering when implementing P6-1.

## Verification

Run on Windows 11, branch `fix-gaps/girsync`, current tree:

- `cargo build -p girsync` — `Finished dev profile`, no warnings, exit 0.
- `cargo test -p girsync` — **373 passed, 0 failed, 1 ignored**. Breakdown:
  lib 133, cli_dispatch 1, compare 13, compare_self 12, convert 4, hash_mode 18,
  helpers 12, lazy 27, migration_lock 11, output 16, rw_threads 15, stat_trust
  19, symlink 24, sync 21 (+1 ignored), sync_plan 19, update 3, why 25,
  doc-tests 0.
- `cargo test -p girsync --test symlink -- --nocapture` — **24 passed, 0
  failed**; six file-link cases print the `os error 1314` skip line, every
  junction case runs.
- `cargo clippy -p girsync --all-targets` — clean, no warnings, exit 0.
- `cargo fmt -p girsync --check` — exit 0.

Reproductions above were re-run against `target\debug\girsync.exe` built from
this tree, with `GIRSYNC_TEST_CACHE=memory`; fixtures are under
`E:\Symlinks\Temp\rev6-filter-final`, `rev6-filter-dir`, and
`rev6-hard-final`.

## Not verified

- P6-1 and P6-2 are reported, not fixed. No regression test for either exists in
  this tree.
- P6-2 was exercised only through a Windows hard link; no POSIX `hard_link` run,
  no hard link to a file outside both roots, and no `--hash none` run.
- The Windows file-identity API (`windows_by_handle`) is unstable on this
  toolchain; only the compile-time rejection was checked, not an implementation.
- Native POSIX symlinks and the file-link paths remain unrunnable here (`os
  error 1314`); the junction cases stand in for directory links only.
- `--keep-extra` and `--missing-only` variants of P6-1 were reasoned about but
  not all run; the reproduction used the default flags plus `--include`.
- The test count was checked with `cargo nextest list`, not by running nextest
  itself; `cargo test` independently confirms 373 run + 1 ignored.
