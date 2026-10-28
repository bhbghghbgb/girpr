# ADVERSARIAL REVIEW — `girsync` (`tools/sync`)

**Baseline.** `HEAD = 64dc69d` (“revert: drop the adversarial review series (tree back
to 7f2d69)”). `64dc69d^{tree}` and `7f2d69c^{tree}` are both
`66985753edf618610a3df629bacdf2b6b70609fa`, so the working tree is byte-identical to
the pre-fix tree. The reverted series (`f9352f7` for the document, `b68ca17` for the
code and tests) is treated here only as a list of claims to check — not as a starting
point, and not as text to restore.

**One-line verdict.** The symlink *walk* is correct; the symlink *deletion* path is
not. A directory symlink (or junction) inside `dst` makes `DELETE`/`RMDIR` reach
through the link, which can delete files outside `dst` and can delete a file that
`src` also has (**F-1**). The reverted “G-1: the symlink walk is broken” finding was
false; the reverted “G-2: follow the link, no code change” decision was incomplete.

---

## 1. Scope and method

**Scope.** The `tools/sync` package: `src/`, `tests/`, `README.md`, and the
`walkdir` dependency as resolved by `Cargo.lock`. The shared leaf crates
(`crates/log`, `crates/tempdir`) were read only where girsync consumes them.

**Method.** Source reading, `git show`, `git grep`, `git log`, and file reads of the
current tree. I also read the local `walkdir 2.5.0` registry source to settle the
symlink question, because the reverted review turned on a claim about
`DirEntry::file_type()`.

**No tests were run.** I did not run `cargo build`, `cargo test`, `cargo nextest`,
`cargo clippy`, or the `girsync` binary; I did not create symlinks or probe the
filesystem. Therefore:

- **source-verified** means “I read the cited lines in the current tree and the
  control flow follows from them”.
- **not verified** means “plausible from the source, but I did not execute it”.
- No pass/fail counts, timings, or “it works on my machine” claims appear in this
  document. Any test cited is cited as *existing code*, not as a result I observed.

**Severity scale.** `high` = data loss or writes/deletes outside the mirror root;
`medium` = a run fails or produces the wrong result; `low` = documentation or test
gap with no incorrect result on the covered paths; `none` = a class explicitly ruled
out.

**Constraint respected.** Only this file was created. No source, test, Cargo,
README, or other project file was modified; nothing was staged or committed.

---

## 2. Findings

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| F-1 | **high** | bug | `src/scan.rs:36-39`, `src/commands/sync/plan.rs:144-151`, `src/commands/sync/apply/phases.rs:19-29` | `DELETE` follows a directory symlink in `dst` and removes files in the link target — possibly outside `dst`, and possibly a file `src` also has |
| F-2 | **medium** | bug | `src/commands/sync/apply/copy.rs:119-122,132` | A `dst` symlink-to-directory at a path where `src` has a file: the guard skips clearing it, then `File::create` fails, aborting the run with exit 3 |
| F-3 | **medium** | test gap | `src/scan.rs:27-43`; `tests/` (no symlink or junction anywhere) | Zero symlink/junction coverage; the README’s “followed and materialized” claim is unpinned, and the only symlink tests ever added were reverted with `b68ca17` |
| F-4 | low | test gap | `src/scan.rs:38`, `src/cli.rs:187-189`, `README.md:1095` | `--max-depth` silently skips deeper files; the unit test covers the walk, but no end-to-end `sync` test asserts the skip |
| F-5 | low | test gap | `src/diff/mod.rs:65-80,82-91`, `src/commands/mod.rs:39-44` | A case-only `compare` reports `total_diff=0` yet exits 4; the predicates are unit-tested, but no binary/`cmd_compare` test pins the exit code |
| F-6 | low | intentional-but-uncovered | `src/commands/sync/apply/copy.rs:147-151` | If mtime restoration fails after a correct write, the whole run aborts; the bytes are already durable and the failure mode is undocumented |
| F-7 | low | test gap | `src/commands/sync/apply/phases.rs:150-173`, `src/cache/write.rs:17-29,192-238` | A mid-copy failure discards the uncommitted cache tail; only a code comment states this, and no test pins it |

Non-findings are in §3: the symlink walk, dry-run, exit codes, the pre-drop commit,
the coverage check, case-only renaming, and the path-traversal negative.

---

### F-1 — `DELETE` reaches through a directory symlink in `dst` (high, bug)

**Behavior (source-verified).**

1. `walk_live` follows symlinks: `src/scan.rs:36-39` sets
   `.follow_links(true)`. walkdir therefore yields a directory link as a directory
   and descends into its target, emitting the target’s children under the link’s
   relative path (see §3 for the walkdir evidence).
2. `diff_maps` puts **every** dst-only key in `Diff::extra`, including keys under a
   followed link: `src/diff/mod.rs:241-248` (exact pairing) and `285-286`
   (insensitive pairing).
3. `build_plan` turns every extra path whose dst kind is `file` into
   `plan.delete_files`: `src/commands/sync/plan.rs:144-151`.
4. `delete_extras` joins the relative path onto `dst` and calls
   `std::fs::remove_file`: `src/commands/sync/apply/phases.rs:19-29`. A symlink in an
   **intermediate** path component is resolved by the OS, so
   `remove_file(dst/link/x)` removes `x` in the link target, not the link.

**Two concrete shapes.**

- *Inside `dst`.* `src` has `real/x`; `dst` has `real/x` and a directory symlink
  `link -> real`. The walk yields `real/x`, `link`, and `link/x`. `real/x` pairs and
  compares equal; `link/x` is `EXTRA` and enters `delete_files`; `link` is `EXTRA`
  and enters `rmdir`. `delete_extras` removes `dst/link/x`, which is the same file as
  `dst/real/x`. The copy plan is empty for `real/x` (it was equal), so nothing
  recreates it. `remove_unknown_dirs` then removes the link. The run exits 0 with
  `real/x` gone — a file that `src` also has.
- *Outside `dst`.* `dst` has a directory symlink `link -> D:\external`; `src` has no
  `link`. The walk yields `link/x`; it is `EXTRA`; `remove_file(dst/link/x)` deletes
  `D:\external\x`. The run exits 0.

The `rmdir` of the link itself is not the data-loss step; the `remove_file` under the
link is. The link’s own removal (via `remove_dir_all` on the link path) is a separate
question and is not what this finding depends on.

**Why it matters.** `sync` is a mirroring tool whose default is to delete extras
(`README.md:1091`). A directory symlink or Windows junction in `dst` — a common way
to graft a mod directory or another drive into a game tree — turns “delete extras”
into “delete the target’s files”. `--keep-extra` avoids it, but the default does not.
`--missing-only` does **not** avoid it: that flag suppresses changed/type-conflict
copies (`plan.rs:114-132`), not deletes (`plan.rs:145`).

**Documentation mismatch.** `README.md:1004-1006` says symlinks are followed and
“link targets outside the root are followed and materialized as plain files/dirs”.
That sentence describes the copy/materialization direction. It does not say that
`DELETE` and `RMDIR` reach through a `dst` link, and it does not warn that the
target can be outside `dst`. The reverted `b68ca17` README sentence — “removals remove
the link itself, not the target” — is **false for descendants of a followed link**
and must not be re-added as written.

**Fix.** Make deletion symlink-aware. The clean shape is to carry link-ness from the
walk (`walkdir::DirEntry::path_is_symlink()` is available in 2.5.0) into `LiveEnt`
and `EffRec`, and in `build_plan` replace any extra path under a symlink ancestor
with the topmost symlink itself: schedule the link for removal with a
platform-correct unlink/rmdir helper (on Unix `remove_file` removes either link kind;
on Windows a file link needs `remove_file` and a directory link needs `remove_dir`,
because `DeleteFileW` rejects a directory entry) and drop its descendants from
`delete_files`/`rmdir`. A runtime belt-and-braces check in
`delete_extras` — `symlink_metadata` on each ancestor, and remove the link instead of
the descendant — is cheap insurance. Do **not** “fix” this by refusing to follow dst
links unless the README and the comparison semantics are changed with it; the
documented behavior is to follow.

If the maintainers decide the current behavior is intentional, then the README must
say, in the symlink bullet and the gotchas, that `DELETE`/`RMDIR` traverse directory
links in `dst` and can remove files outside `dst`. The safer default is still the
guard.

**Test.** `dst/link -> dst/real`, `src/real/x`, `dst/real/x`: assert `dst/real/x`
survives and the link is removed, and assert the `DELETE` record names the link (or
is absent), not `link/x`. Repeat with an external target and with `--keep-extra`
(where nothing is deleted). This test does not exist today (F-3).

**Verification.** Source-verified from the cited lines. Not runtime-verified: no
binary was run and no symlink was created.

---

### F-2 — A `dst` symlink-to-directory blocks a file copy and aborts the run (medium, bug)

**Behavior (source-verified).**

- When `src` has a file and `dst` has a directory at the same rel, `diff_maps` puts
  the path in `type_conflict` (`src/diff/mod.rs:250-251`), and `build_plan` adds a
  copy because `src_is_file` is true (`src/commands/sync/plan.rs:123-131`).
- `copy_one` has the guard `if d.is_dir() && !d.is_symlink()` before
  `remove_dir_all` (`src/commands/sync/apply/copy.rs:119-122`). For a directory
  symlink, `d.is_dir()` is true and `d.is_symlink()` is true, so the guard is false:
  the link is **not** cleared.
- The next filesystem call is `std::fs::File::create(&d)` (`copy.rs:132`). On a
  symlink to a directory this fails (Unix `EISDIR`; Windows `ERROR_ACCESS_DENIED`
  from `CreateFileW` without directory semantics). The error propagates through
  `copy_files` (`phases.rs:135`) and `main` maps it to exit 3
  (`src/main.rs:34-39`).

**Why it matters.** This is the reverse direction of the documented type-conflict
rule (“resolve toward src kind”, `README.md:731`). The other direction is handled:
`src` directory vs `dst` file/symlink goes through `fix_dirs`, which explicitly
removes a symlink (`src/commands/sync/apply/mod.rs:131-139`). A `dst` symlink to a
*file* is followed and written through (`copy.rs:132`), which the README sanctions.
Only the directory-link-at-a-file-path case fails, and it fails the whole run rather
than resolving the conflict.

**Fix.** In `copy_one`:

```rust
if d.is_dir() {
    if d.is_symlink() {
        // remove the link, not the target. Platform-specific:
        //   Unix:    std::fs::remove_file(&d)   // unlink works on either link kind
        //   Windows: std::fs::remove_dir(&d)    // RemoveDirectoryW on a dir link
        remove_dir_link(&d)
            .with_context(|| format!("unlink {}", d.display()))?;
    } else {
        std::fs::remove_dir_all(&d)             // the existing behavior
            .with_context(|| format!("rmdir {}", d.display()))?;
    }
}
```

Do **not** reach for `remove_file` unconditionally: on Unix it removes a directory
symlink, but on Windows `DeleteFileW` rejects a directory entry, so the “fix” would
swap one failure for another. `std::fs::remove_dir_all` on the link path is an
alternative *if* its documented “does not follow symbolic links” behavior is trusted;
I did not verify that at runtime (see §5).

Keep the `d.is_dir()` outer test: a symlink to a *file* must still be followed and
truncated by `File::create`, because that is the documented write-through behavior.
For a Windows junction the `FileType` bits are not portable; see the open question in §5.

**Test.** `src/a` is a regular file; `dst/a` is a directory symlink to an external
directory. `sync` must exit 0, remove the link (not the target), and leave a regular
file `dst/a` with `src`’s bytes. This test does not exist today (F-3).

**Verification.** Source-verified from the cited lines and the documented behavior of
`File::create` on a directory. Not runtime-verified.

---

### F-3 — No symlink or junction tests at all (medium, test gap)

**Behavior (source-verified).**

- `git grep -n symlink -- tools/sync/tests` returns nothing. There is no
  `symlink_file`, `symlink_dir`, `std::os::windows::fs`, `std::os::unix::fs::symlink`,
  or junction helper anywhere under `tools/sync/tests`.
- The only symlink tests ever written for this package were in the reverted
  `b68ca17`: a file link walked as a file, a directory link walked as a directory
  with its subtree, and a dangling link aborting. Those tests asserted behavior that
  the current tree already has (see §3); they were reverted with the dead code they
  accompanied.
- The current walk tests are `src/scan.rs:146-170` (cache pruning) and
  `src/scan.rs:175-192` (depth cap). Neither creates a link.

**Why it matters.** `README.md:1004-1006` makes three testable claims — links are
followed, targets outside the root are materialized, and dangling links abort — and
none is pinned. More importantly, the two real symlink defects (F-1, F-2) live
exactly in the untested paths, so a regression test is the only way to keep them
fixed.

**Fix.** Add tests, in the smallest environment that can create a link:

1. `walk_live` on a file link: entry is a file with the target’s size and mtime.
2. `walk_live` on a directory link: entry is a directory and the subtree under the
   link is walked (including a target outside the root).
3. `walk_live` on a dangling link: the walk aborts and the error names the link.
4. `sync` with a `src` file link: `dst` gets a plain file, not a link.
5. `sync` with a `src` directory link: `dst` gets a plain directory and its files.
6. `sync` with a `dst` file link: the write lands on the target (documented
   write-through).
7. `sync` with a `dst` directory link and a `src` file: F-2’s regression.
8. `sync` with a `dst` directory link and extras: F-1’s regression.
9. A junction variant for 7/8 if the helper can create one without elevation
   (`cmd /c mklink /J` on Windows).

The `b68ca17` helper pattern — try to create a probe link, `eprintln!` a skip and
return when creation fails — is the right shape on Windows. A silent skip is not:
log it, and keep the non-link assertions in a separate test so a skipped link test
does not hide a walk regression.

**Verification.** Source-verified that no test creates a link. Not runtime-verified
(no tests run; link-creation privilege not probed).

---

### F-4 — `--max-depth` has no end-to-end test (low, test gap)

**Behavior (source-verified).** The default is 10 (`src/cli.rs:187-189`), threaded
into `CommonOpts` (`src/config/common.rs:121-122,289`) and applied by
`walk_live` (`src/scan.rs:38`). A file deeper than the cap is absent from the
effective map, so `sync` neither copies it nor reports it, and the run exits 0.
`README.md:1095` documents the gotcha. The unit test at `src/scan.rs:175-192`
asserts the walk’s own set at depth 2; it does not assert that `sync` leaves a deep
file unmirrored.

**Why it matters.** This is the “silent data loss” path: a user with a real game tree
deeper than 10 gets a green run and an incomplete mirror. The unit test proves the
walk is bounded; only an end-to-end test proves the *command* accepts that as
success. The reverted `b68ca17` added exactly this test
(`run_sync_max_depth_skips_deeper_files`: a file at depth 11, default cap, `cmd_sync`
returns 0, deep file absent, shallow file present). It is test-only and safe to
re-land.

**Fix.** Re-land that test (or write an equivalent): `src` with a depth-11 file and a
top-level file; `sync` with default options; assert exit 0, deep path absent,
top-level path present. Optionally follow with a `compare` that reports the deep file
as `MISSING`.

**Verification.** Source-verified. Not runtime-verified.

---

### F-5 — The case-only exit code is not pinned at the command/binary level (low, test gap)

**Behavior (source-verified).** `Diff::total()` deliberately excludes
`case_mismatch` (`src/diff/mod.rs:65-80`); `Diff::is_empty()` consults it
(`src/diff/mod.rs:82-91`); `report_diff` computes the exit code from `is_empty()`
(`src/commands/mod.rs:39-44`). So a case-only pair with equal content produces
`SUMMARY … case_mismatch=1 total_diff=0` and exits 4. That is the documented and
correct behavior: casing is a difference for `compare`, and `sync` renames before
diffing so a clean mirror does not exit 4 on spelling (`README.md:737-746`).

**What is covered.** The predicate divergence is unit-tested at
`src/diff/mod.rs:503-514` (`total() == 0`) and at `tests/why/identical.rs:63-82`
(`!is_empty()`). The record shape (`total_diff: 0`, `case_mismatch: 1`) is asserted
at `tests/lazy/compare.rs:306-338`.

**What is not covered.** No test asserts the *exit code* for a case-only pair.
`tests/lazy/main.rs:83-85` reimplements `is_empty()` in the test helper, so it cannot
catch a change in `report_diff`; `tests/lazy/compare.rs:419-439` exercises
`cmd_compare` only on a fixture with four real differences. A regression that changed
`report_diff` to use `total()` would pass every current case-only test and silently
turn a case-only audit into exit 0.

**Why the old “doc error” label was wrong.** The `total()` doc at
`src/diff/mod.rs:65-66` already says it ignores case mismatches, which are “still a
difference”; `is_empty()`’s doc at `82-91` explains why the two predicates differ.
There is no doc fix to make. The gap is test coverage, not documentation.

**Fix.** Add a `cmd_compare` or binary test: `src/a.txt` and `dst/A.txt` with
identical bytes; insensitive mode; assert exit 4; parse the JSON `SUMMARY` and assert
`case_mismatch == 1` and `total_diff == 0`. Add the inverse (`--case-sensitive`
reports `MISSING` + `EXTRA`) if the fixture is reused. This is safe to re-land and
does not depend on any behavior fix.

**Verification.** Source-verified. Not runtime-verified.

---

### F-6 — mtime-restore failure aborts an otherwise-good copy (low, intentional-but-uncovered)

**Behavior (source-verified).** `copy_one` writes and (unless the test hook is on)
`sync_all`s the bytes (`src/commands/sync/apply/copy.rs:140-145`), then restores the
source mtime via `OpenOptions::new().write(true).open(&d)` and `set_modified`
(`copy.rs:147-151`). A failure in either call is propagated with `?`; `copy_files`
turns it into a run error (`phases.rs:135`), and `main` exits 3. The bytes are already
correct and durable at that point; only the mtime is wrong, so a rerun sees an mtime
difference and copies again.

**Why it matters.** It is fail-fast rather than silent corruption, which is
defensible, but the failure mode is not in the README’s copy step (`README.md:732-735`
says “preserve mtime, verify-after-copy”) and it is not tested. The reverted
`b68ca17` added a four-line comment above the write saying mtime restore is
load-bearing; that comment is accurate and safe to re-add (unlike the commit’s false
symlink-deletion note).

**Fix.** Document it: either a sentence in the README copy step or a doc comment on
`copy_one`, saying that a failed mtime restore aborts the copy even though the bytes
are correct, and that a rerun recopies. A deterministic test needs a way to make
`set_modified` fail while the write succeeds (read-only/ACL manipulation), which is
platform-specific; mark the test optional rather than pretending it is portable.

**Verification.** Source-verified. Not runtime-verified; no failure was injected.

---

### F-7 — The uncommitted cache tail on a mid-copy failure is untested (low, test gap)

**Behavior (source-verified).** `copy_files` writes each verified copy’s row as it
completes (`phases.rs:150-153`). `CacheWrite::put` commits when one of the three
`CommitLimits` thresholds trips — 50 operations, 256 MiB, or 5 seconds
(`src/cache/write.rs:17-29,192-203`). The final partial batch is committed only at
the end of the phase (`phases.rs:167-173`). If any copy fails, the `?` at
`phases.rs:162-165` returns **before** that final commit; the `CacheWrite` is dropped
with its live transaction uncommitted, so every row since the last threshold is
discarded. The code comment at `phases.rs:167-169` says exactly this.

**Why it matters.** The reverted review claimed “completed copies have their cache
rows written, so a rerun skips them”. That is false for the tail: a rerun still
converges (phase A stats dst, size+mtime match, the pair is decided equal), but it
may do so with a stat-only row and a later `compare` may rehash. No data is lost, but
the claim about durability is wrong, and a future change that makes cache rows
load-bearing would turn this into a correctness bug. There is no test that a failure
mid-copy leaves the tail uncached; the existing test at `tests/sync.rs:632-696` only
asserts that a threshold commits *during* a successful copy.

**Fix.** Add a failure-injection test (or a `GIRSYNC_TEST_FAIL_COPY` hook) with
`GIRSYNC_TEST_COMMIT_OPS` lowered: N copies, a failure after the first threshold,
then assert the committed prefix is present, the tail is absent, and a rerun
converges. If a hook is too invasive, at minimum correct the README wording: “No
resume” (`README.md:735`) should not be read as “everything copied so far is
durable”. Do **not** re-add the reverted “completed copies … rerun skips them”
sentence.

**Verification.** Source-verified. Not runtime-verified; redb’s drop-aborts-the-
transaction behavior is assumed from its API and from the code comment, not observed.

---

## 3. Verified-correct behavior

These are the claims I checked and found correct in the current tree. They are
included so the review does not read as a list of defects only.

### 3.1 The symlink walk is correct — the reverted G-1 premise was false

`walk_live` sets `.follow_links(true)` (`src/scan.rs:36-39`). In walkdir 2.5.0:

- `lib.rs:840-847` (`handle_entry`) checks the original entry with
  `file_type().is_symlink()`; when `follow_links` is on and the entry is a link, it
  replaces the entry via `follow`.
- `lib.rs:961-963` (`follow`) constructs the replacement with
  `DirEntry::from_path(self.depth, dent.path().to_path_buf(), true)`.
- `dent.rs:230-249` (Windows) and `dent.rs:252-273` (Unix) use `fs::metadata` when
  `follow` is true, then store `ty: md.file_type()`.
- `dent.rs:150-160` documents `file_type()`: “If this is a symbolic link and
  `follow_links` is `true`, then this returns the type of the target.”

So at `src/scan.rs:74-95`, a file link is `is_file`, a directory link is `is_dir`,
and a dangling link is an error from walkdir’s own arm (`scan.rs:43`) — the documented
“dangling links abort” behavior. `ft.is_symlink()` is never true for an entry the
loop sees: `handle_entry` has already replaced every link with its target’s type (and
the depth-0 root is skipped at `scan.rs:45-47`). That is why the `b68ca17` branch that
tested it was unreachable dead code.

The correct API if link-ness is needed is `DirEntry::path_is_symlink()` (walkdir
2.5.0 `dent.rs`), which F-1’s fix can use.

The practical consequence: the old “any symlink aborts the walk” finding was wrong,
and the code change that “fixed” it was a no-op at runtime. The symlink tests, on the
other hand, were valid and are worth keeping (F-3).

### 3.2 `--dry-run` writes nothing

`open_folder_cache` is the single place a folder cache is opened
(`src/effective/side.rs:143-187`). Under `dry_run` it opens an existing cache
read-only (`side.rs:185-186`) and serves a missing one from an in-memory DB
(`side.rs:181-183`); `--ignore-cache` is treated as absent (`side.rs:125-127`).
`cmd_sync` returns before the apply phases in dry-run (`src/commands/sync/mod.rs:330-345`),
so no copy/delete/mkdir runs. `compare-self` never writes at all
(`src/commands/compare_self.rs:102-120`). The tests at `tests/compare.rs:525-526`
assert a byte-identical cache and no backup sibling for a dry-run `compare`; they are
cited as existing intent, not as a result I ran.

### 3.3 Exit codes

`main` maps a `run` error to exit 3 (`src/main.rs:34-39`); clap handles parse errors
before `main` (exit 2); `report_diff` returns 0 or 4 from `is_empty()`
(`src/commands/mod.rs:39-44`). `sync` returns `Ok(0)` on success
(`src/commands/sync/mod.rs:361-381`) and propagates errors to exit 3; it never uses
exit 4. This matches `README.md:1007-1012`.

### 3.4 The pre-drop commit happens before any filesystem change

`Applier::apply` runs `mkdirs`, `fix_dirs`, `predrop_cache_entries`, then
`delete_extras`, `copy_files`, `remove_unknown_dirs` (`src/commands/sync/apply/mod.rs:78-90`).
`predrop_cache_entries` removes the planned copy/delete rows and commits before the
first delete or copy (`mod.rs:174-187`). The rationale in the doc comment — a rerun
re-resolves rather than trusting a half-written file — is consistent with the code.

### 3.5 The coverage check prevents a silent “clean”

`plan_pairs` returns a `Coverage` error when a record side cannot supply a digest for
an undecided pair (`src/planner/pairs.rs:367`), with the message built in
`src/planner/coverage.rs`. This is checked before phase C hashes content, so a
record that cannot answer does not fall back to size+mtime and report a confident
“equal”. The reasoning and the `--hash-any-of` nuance are documented in
`pairs.rs:165-226`.

### 3.6 Case-only rename before diff

`cmd_sync` plans (`src/commands/sync/mod.rs:197`), renames dst paths to src casing
(`mod.rs:276`), and only then diffs (`mod.rs:291`). That is the ordering the README
explains at `README.md:737-746`; pairing by exact key first would make a case-only
pair look one-sided and skip the digest it needs.

### 3.7 `--keep-extra` spares directories as well as files

`build_plan` gates `delete_files` on `!keep_extra` (`plan.rs:145`) and gates `rmdir`
on `!keep_extra` (`plan.rs:189-205`). The test at `tests/sync.rs:188` covers the
directory half. This is the documented behavior and is consistent with the flag name.

### 3.8 Non-dry-run `compare` backs up both caches — documented, not a defect

`open_side` calls `open_folder_cache(root, common, mode, true)` for a folder side
(`src/effective/side.rs:86-100`), so a real `compare` opens both caches read-write
with `backup_first = true`. `README.md:435-440` says `compare` writes the caches and
that `--dry-run` is how to leave them byte-identical; `README.md:1093` says backups
accumulate. The old review’s “documented only obliquely” is therefore too strong.
No code change is needed; at most a one-line cross-reference.

### 3.9 Path traversal via `..` or an absolute `rel` is not an exposure

Every `rel` is produced internally by the walk (`src/scan.rs:53-59`, from
`strip_prefix(root)`) or by a cache key; no user-supplied manifest supplies a
relative path. The roots themselves are validated (`src/commands/sync/mod.rs:385-395`),
and `is_record_path` rejects record inputs. The `../../escape` class does not apply
here. This confirms the reverted G-11 negative.

---

## 4. Corrections to the reverted review series

Each row names the old claim, what the current tree actually does, and what that means
for re-landing. Consequence vocabulary: **no-op** = the old claim led to no behavior
change or to a test that is already covered; **unverified** = the old claim was never
demonstrated; **side effect** = the claim described a real side effect but drew the
wrong conclusion; **wrong claim** = the current tree contradicts the claim.

| Old ID | Old claim (paraphrased) | Actual truth in the current tree | Consequence |
|--------|-------------------------|----------------------------------|-------------|
| G-1 | walkdir’s `file_type()` does not follow, so any symlink aborts the walk; README contradicted; medium bug | `follow_links(true)` (`scan.rs:37`) makes walkdir use `fs::metadata` and set `ty = md.file_type()` (walkdir `dent.rs:230-273`); `file_type()` returns the target type (`dent.rs:150-160`). The walk handles links correctly; `b68ca17`’s `ft.is_symlink()` branch was unreachable dead code, and its tests pinned already-correct behavior. | **No-op (dead code).** Drop the code change; keep the tests. The real symlink defects are F-1 and F-2. |
| G-2 | `copy_one` follows dst symlinks; intentional per README; environment-gated test only | A dst symlink-to-*file* is followed and written through (`copy.rs:132`), true. A dst symlink-to-*directory* at a file path is not handled: the guard skips removal and `File::create` fails (F-2). A directory link in dst also makes deletes reach the target (F-1). | **Unverified → bug.** “No code change” is wrong; F-2 needs a guard, F-1 needs a deletion guard. |
| G-3 | One apply failure aborts with exit 3 and leaves a partial dst; completed copies have cache rows, so a rerun resumes | The abort and partial dst are real and documented (`README.md:719-721`, `1091-1092`). But rows are durable only up to the last `CommitLimits` threshold; a failure before the final commit drops the tail (`phases.rs:150-173`, `cache/write.rs:17-29,192-238`). | **Wrong claim (side effect misread).** Do not re-add “completed copies are cached so a rerun skips them”; the rerun still converges, but the tail is re-scanned. Test gap F-7. |
| G-4 | `--max-depth` silently skips deeper files; unit test only, no end-to-end test | True: default 10 (`cli.rs:187-189`), applied at `scan.rs:38`; unit test at `scan.rs:175-192`; no e2e test. | **No-op (valid test gap).** Re-land the e2e test (F-4); no code change. |
| G-5 | `Diff::total()`’s doc is inaccurate because it excludes `case_mismatch` | The doc is accurate: it says it ignores case mismatches, which are still a difference (`diff/mod.rs:65-66`); `is_empty()` explains the separate predicate (`82-91`). Predicate divergence is tested (`diff/mod.rs:503-514`, `tests/why/identical.rs:63-82`). Only the exit-code consequence lacks a binary test. | **No-op (doc already correct).** Keep the doc; add the exit-code test (F-5). |
| G-6 | `update --hash-any-of` stores all algorithms; only the warning, no pinning test | The behavior is intentional and documented in code (`update.rs:73-94`); it is already tested at `tests/hash_mode/end_to_end.rs:98-115` (both digests stored). | **No-op (already covered).** Do not add a duplicate test. |
| G-7 | Non-dry-run `compare` backs up both caches; documented only obliquely | `open_side` passes `backup_first = true` (`side.rs:90`); `README.md:435-440` says compare writes the caches; `README.md:1093` says backups accumulate. | **Side effect, already documented.** Optional one-line clarification only. |
| G-8 | `compare-self --ignore-cache` bails; the inert flags warn; only the bail is directly tested | The bail is tested at `tests/compare_self.rs:439-463`; the no-trust redundancy is tested at `134-151`; `--dry-run` is tested at `tests/compare.rs:534-551`. The warning *text* is not asserted, but the behavior is. | **No-op (already covered).** Optional stderr assertion, not a finding. |
| G-9 | `--hash-all-of none` + distrusted stat is refused; covered only at unit/config level, not through the binary | The refusal is at `config/common.rs:257-279`; `tests/stat_trust/refusal.rs:19-57` runs the real binary with all three flag spellings, and `:123` checks uniform subcommands. | **No-op (wrong coverage claim).** Already covered; do not duplicate. |
| G-10 | `copy_one` aborts if mtime restore fails after the bytes are correct | True (`copy.rs:147-151`). The reverted commit added an accurate comment; it was not a bug fix. | **No-op (doc note only).** Re-add the comment/doc sentence (F-6); no behavior change. |
| G-11 | Path traversal via `..`/absolute `rel` is not a girsync exposure | True: rels are internal (`scan.rs:53-59`); roots are validated (`sync/mod.rs:385-395`). | **No-op (negative confirmed).** No action. |
| — | (reverted README note) “removals remove the link itself, not the target” | False for paths *under* a followed directory link: `remove_file(dst/link/x)` follows the intermediate link (F-1). It is true only for the top-level link path. | **Side effect / do not re-add.** It would codify the F-1 data-loss path. |

---

## 5. Open questions and unverified claims

1. **F-1/F-2 were not executed.** No binary was run and no link was created. The
   control flow is source-verified; the OS error codes for `File::create` on a
   directory link and for `remove_file` through an intermediate link are standard but
   not observed here.
2. **Windows junctions.** On rustc 1.98.1, `symlink_metadata` on a junction reports
   `is_symlink = true`, `is_dir = false`, `is_file = false`, `attrs = 0x410`, while
   `path_is_symlink()` is true and the followed `metadata()` is a directory. Older
   toolchains have been reported to differ, so the `FileType` bits are not a
   portable test on their own; `FILE_ATTRIBUTE_REPARSE_POINT` is necessary but not
   sufficient — cloud placeholders, dedup and AppExecLink set it too. The walk
   follows junctions (reparse points resolved by `fs::metadata`), so F-1’s deletion
   shape applies to them too, and `src/link.rs` starts from std’s name-surrogate
   `FileType::is_symlink` and falls back to `read_link` (§7.3, T-4). Whether
   `remove_dir_all` on a junction removes only the junction or recurses into the
   target is still **not verified** here, and the fix avoids depending on it.
3. **`remove_dir_all` on a directory symlink.** Rust’s documentation says it removes
   the link rather than following it, but I did not run it. F-1 does not depend on
   this: the data loss is the `remove_file` of a descendant.
4. **Is F-1 intentional?** The README says links are followed and targets outside the
   root are materialized. If the maintainers read that as “the target is part of the
   tree, so extras inside it are deleted”, then F-1 is a documentation defect rather
   than a code defect. Either way, the current README does not warn that `DELETE` can
   remove files outside `dst`, and the safe default is a guard.
5. **F-6 testability.** Making `set_modified` fail while the write succeeds requires
   a read-only/ACL/filesystem condition that is not portable. I did not find a
   test-only hook for it. The doc note is the practical fix; the test is optional.
6. **F-7 transaction drop.** The claim that dropping an uncommitted redb
   `WriteTransaction` discards its rows is taken from the API and from the code
   comment at `phases.rs:167-169`; it was not observed. The same caveat applies to the
   test-only `GIRSYNC_TEST_DURABILITY=none` path.
7. **The `compare-self` warning text.** `warn!` is emitted at
   `compare_self.rs:102-120`, but no test captures stderr; whether the default console
   level always shows it is not verified. The behavior (flag accepted, no writes) is
   tested.
8. **Test counts.** The reverted commit message claims 326/335 passing tests and a
   clean clippy/fmt. I did not reproduce any of that; no build or test was run. Those
   numbers should not be carried into a new change description without a fresh run.

---

## 6. Recommended landing order

### Safe to re-land now (test-only, no behavior change)

| Item | Why |
|------|-----|
| F-3’s three `walk_live` symlink tests (file link, directory link + subtree, dangling abort) | They pin behavior the current tree already has; the reverted `b68ca17` tests would pass unchanged. Do not re-land the `ft.is_symlink()` branch with them. |
| F-4’s `--max-depth` end-to-end test | Test-only; asserts the documented silent skip. The reverted `run_sync_max_depth_skips_deeper_files` is a ready model. |
| F-5’s case-only exit-code test | Test-only; pins `report_diff`’s use of `is_empty()` rather than `total()`. |
| F-6’s comment/doc sentence | Accurate and behavior-neutral; re-add the mtime-restore note (not the false symlink note). |

### Needs tests first (behavior fixes)

| Item | Order |
|------|-------|
| F-1 (deletion through dst directory links) | Write the failing test first (inside-dst target, external target, `--keep-extra`), then the symlink-aware plan/delete guard. This is the highest-priority fix. |
| F-2 (dst directory link vs src file) | Write the failing test, then the `d.is_dir()` / `d.is_symlink()` branch in `copy_one`. |
| F-7 (uncommitted tail) | Lower `GIRSYNC_TEST_COMMIT_OPS`, inject a mid-copy failure, assert the committed prefix and the discarded tail; add the README “No resume” clarification. |

### Should be dropped

| Item | Why |
|------|-----|
| **G-1’s `ft.is_symlink()` branch** | Dead code: with `follow_links(true)`, `file_type()` is the target type (walkdir `dent.rs:230-273`). Re-adding it changes nothing at runtime and obscures the real defects. |
| The reverted README sentence “completed copies … a rerun skips them” | False for the uncommitted tail (`phases.rs:167-169`). |
| The reverted README sentence “removals remove the link itself, not the target” | False for descendants of a followed directory link (F-1). |
| The reverted G-6/G-8/G-9 test additions | Already covered by `tests/hash_mode/end_to_end.rs:98`, `tests/compare_self.rs:134-151,439-463`, `tests/compare.rs:534-551`, and `tests/stat_trust/refusal.rs:19-57`. Duplicates add maintenance cost without coverage. |
| The old G-5 “doc error” edit | The doc at `diff/mod.rs:65-66,82-91` is already accurate; only the exit-code test is missing. |

---

## 7. Pass log

Implementation plus the eight review passes after it, one subsection each. Evidence,
failure scenarios and per-pass verification are in the git history:
`git log --oneline main..HEAD` and `git show <commit>`; the hashes below are the
index. §1–§6 remain the source-only original audit.

### 7.1 First pass — F-1…F-7

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| F-1 | `DELETE` reached through a dst directory link, removing the link target (possibly outside dst, possibly a file src also has); extras under a link now plan the topmost link | fixed | `1c827b2`, `42b9766`, `cca7218`, `9beede2`, `00f3e39`, `dae1a78`, `943e2d5` |
| F-2 | A dst directory link at a src file path was skipped by the clear guard and aborted the run in `File::create`; it is now unlinked with a platform-correct call | fixed | `2bec6b3`, `cca7218` |
| F-3 | No symlink or junction tests at all; 24 link cases now live in `tests/symlink.rs`, directory links falling back to a junction | tests | `1c827b2`, `2bec6b3`, `42b9766`, `f4ffb59`, `943e2d5` |
| F-4 | `--max-depth` had no end-to-end test; a depth-11 file under the default cap now stays unmirrored with exit 0 | tests | `d077a78` |
| F-5 | The case-only exit code was unpinned at command/binary level; a case-only pair now pins exit 4 with `case_mismatch=1`, `total_diff=0` | tests | `ddadcc9` |
| F-6 | A failed mtime restore aborts an otherwise-good copy, undocumented; the copy step and `copy_one` now say so | documented | `d077a78`, `1d723cb` |
| F-7 | The uncommitted cache tail on a mid-copy failure was untested; `GIRSYNC_TEST_FAIL_COPY` plus a low `GIRSYNC_TEST_COMMIT_OPS` pins it | tests | `d077a78` |

**Decisions still in force:**
- A removal never traverses a dst link: an extra under one resolves to the topmost link, whose descendants leave `delete_files`/`rmdir`.
- A directory link at a file path is unlinked; a dst link to a *file* is still followed and written through.
- A failed mtime restore aborts the run although the bytes are durable, and a rerun recopies.
- Cache rows are durable only to the last commit threshold; a mid-copy failure drops the tail and a rerun re-stats.

### 7.2 Second pass — N-1…N-8

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| N-1 | A shared-link type conflict deleted external data through `fix_dirs`/`copy_one`; `ensure_link_safe` now refuses any action with a strict link ancestor | fixed | `00f3e39`, `dae1a78`, `943e2d5` |
| N-2 | Cache drift after removing a link and after `fix_dirs`; rows are pruned by prefix and `fix_dirs` writes its directory row | fixed | `dae1a78`, `943e2d5` |
| N-3 | `--exclude` of a directory did not exclude its subtree; ancestor directories of include matches are retained | fixed | `5092cce`, `943e2d5` |
| N-4 | A shared link with extra descendants exited 0; a default sync now fails, `--keep-extra`/`--missing-only` warn | resolved | `00f3e39`, `943e2d5` |
| N-5 | A rename through a shared link mutated the external target; it is now refused before the rename | fixed | `00f3e39`, `943e2d5` |
| N-6 | The reparse check ran for every entry; it is gated on `ft.is_dir()`, one `symlink_metadata` per entry and one stat per component | fixed | `9beede2` |
| N-7 | Junction docs and the README symlink/filter bullets did not match the code; claims, comments and test count corrected | fixed | `9beede2`, `5092cce`, `dae1a78`, `abc413c` |
| N-8 | Link helpers swallowed non-`NotFound` errors; they now propagate, and `fix_dirs` uses `link::remove` | fixed | `9beede2`, `dae1a78` |

**Decisions still in force:**
- Shared links are unsupported: a default sync fails before the dry run or any filesystem change if it would write, delete or rename through one, or leave extras under one.
- `--keep-extra`/`--missing-only` opt out of the extras failure only, warn that the run does not converge, and still refuse writes.
- An excluded directory excludes its subtree; an include match keeps ancestor directories, so a link above an included path stays visible.
- The dst cache follows dst exactly: removing a link prunes its whole prefix, and `fix_dirs` writes its row.

### 7.3 Third pass — T-1…T-6

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| T-1 | Nested and overlapping roots were accepted and destroyed data; they are refused before any cache or file change, component-wise, case-insensitively on Windows | fixed | `71c473c`, `2f536de`, `e0ecfff` |
| T-2 | A failed `sync_all` was ignored and the copy cached; a failed flush now aborts with exit 3 and no row | fixed | `924e475`, `2f536de`, `e0ecfff` |
| T-3 | Parent-first case renames rewrote children through paths the parent move had already changed; children are rewritten or covered and map/cache descendants re-keyed | fixed | `8279581`, `2f536de` |
| T-4 | Every reparse point was treated as a link; only redirecting ones are, starting from `FileType::is_symlink` with `read_link` as fallback | fixed | `2477e0c`, `e0ecfff` |
| T-5 | Dead public `link::is_link` and a stale test-side comment; both removed | fixed | `2477e0c` |
| T-6 | Case-only root spellings made two caches and a misleading `--ignore-cache` hint; the root is canonicalized and a lock is distinguished from corruption | fixed | `08c19ae`, `2f536de`, `e0ecfff` |

**Decisions still in force:**
- A mirror inside its own source is refused; there is no “exclude the destination automatically” mode.
- A failed flush is fatal and not retried in-process; the rerun recopies because no row was written.
- One directory rename is one `RENAME`; already-covered descendants stay silent, while the map and cache follow them.
- On Windows a link is a redirecting reparse point; non-link reparse points are ordinary data.
- A lock is not corruption: `--ignore-cache` is never suggested for `DatabaseAlreadyOpen`.
- Two case-only spellings of one root are one cache, even before the cache file exists.

### 7.4 Fourth pass — F-1 (reused ID)

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| F-1 | A link inside the effective src that reached dst (target inside dst, or containing it) hid the root overlap and let the delete pass eat dst; it is refused after the src walk and before planning | fixed | `8057f9a`, `d3c8f00`, `174ba52`, `49e7d49` |

The fourth pass reused the ID `F-1` for a different finding; the original high finding is §2’s F-1, whose status is in §7.1.

**Decisions still in force:**
- A src link whose effective target is dst, or contains dst, is refused after src phase A; a link elsewhere is ordinary content, even back inside src.
- A `girpr-cache*` component is reserved at any depth, so a followed link cannot mirror a nested root’s live cache or backups.
- A removal whose parent resolves inside src is refused on disk, so the plan is never the last word on what may be deleted.

### 7.5 Fifth pass — P5-1, P5-2

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| P5-1 | `mkdirs` ran before `fix_dirs`, so a valid plan failed on `create_dir_all`; the order is swapped and a plan-time preflight refuses a path under an unfixed blocking file | fixed | `2e165e6`, `59f1127` |
| P5-2 | A dst file link into src was written through and destroyed the source; dst links into src are refused after dst phase A and `copy_one` re-checks the final component | fixed | `82e5617`, `93ab01c` |

**Decisions still in force:**
- `fix_dirs` runs before `mkdirs`.
- A dst link resolving into src, or containing src, is refused after dst phase A even when the plan would only unlink it; write-through to a dst-internal target stays sanctioned.
- Hard links are invisible to the scanner, so these rules do not see them; P6-2 refuses them by link count instead.

### 7.6 Sixth pass — P6-1, P6-2, P6-3

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| P6-1 | A filtered dst blocker at an ancestor was invisible to the plan guard; `ensure_mkdir_targets_free` stats the disk before `rename_to_src_casing` and refuses it | fixed | `af5116b`, `08897e4` |
| P6-2 | A dst hard link was written through, truncating the shared inode; `hardlink::count` refuses any destination with more than one link before `File::create` | fixed | `b2b1445`, `2a99fa3` |
| P6-3 | The README test count was stale; it now states 384 test functions, 383 run by default | fixed | `8d52de6` |

**Decisions still in force:**
- Any destination with more than one hard link is refused, blanket rather than limited to links into src; P7-4 credits a same-inode name the plan removes before the copy (§7.7). P8-1 narrows the blanket rule to regular-file destinations (§7.8).
- The mkdir preflight and the real run read the same disk state, so a filtered blocker is refused alike in `--dry-run` and before any record.
- `--max-depth` still skips silently; an excluded dst directory at an ancestor still reports `MKDIR` although `create_dir_all` is a no-op.

### 7.7 Seventh pass — P7-1…P7-4

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| P7-1 | The case-only rename used a fixed sibling temporary, so a user file at that name was replaced (exit 0, silent loss) and a directory aborted the pass; the temporary became a unique `<full name>.girsync_case_tmp.<n>`, claimed before use and stepped past an occupied candidate (the name was later bounded to a fixed short sibling, P8-4) | fixed | `7e8809b` |
| P7-2 | `ensure_link_safe` refused a descendant of a dst link that the plan's own `fix_dirs` removes first; a link ancestor that is a planned fix is now skipped, while a link above it is still refused | fixed | `f4dd3c0` |
| P7-3 | `RENAME` and `renamed` were published before the two renames, so a failed pass left a record for an action that did not happen; a real run now emits after both succeed, and a dry run still reports the step it would take | fixed | `7e8809b`, `f60e151` |
| P7-4 | The sixth pass's hard-link refusal lived only in `copy_one`, so a dry run reported `COPY` and exit 0 for a tree the real run refused; `hardlink::info` adds an inode identity and `ensure_hard_linked_targets_safe` decides at plan time in both modes | fixed | `53d113f` |

**Decisions still in force:**
- The case-fix temporary is a claimed, unique sibling; an existing path at a candidate is never replaced. A failed step 2 puts the path back, and a failed step 1 removes its own file placeholder.
- A `RENAME` record and the summary's `renamed` describe a completed rename. A real run publishes neither on failure; a dry run publishes the step it would take, and the README's dry-run list names `RENAME`.
- A link ancestor that is itself a planned `fix_dirs` entry does not block descendants: `fix_dirs` unlinks it before `mkdir`/`copy`. A link above the fixed one still blocks.
- The hard-link decision is plan-time and identical in both modes. A same-inode name removed by `delete_files` or `fix_dirs` before the copy is credited; unlinking a link itself is not credited. `copy_one` keeps its runtime check.

### Verification (seventh pass)

- `cargo test -p girsync` — **395 passed, 0 failed, 1 ignored** (the pre-existing `tests/sync.rs` case); suite 396 functions, 395 run. Breakdown: lib 149; cli_dispatch 1, compare 13, compare_self 12, convert 4, hash_mode 18, helpers 12, lazy 27, migration_lock 11, output 16, rw_threads 15, stat_trust 19, symlink 24, sync 27 (+1 ignored), sync_plan 19, update 3, why 25; doctests 0. `--test symlink -- --nocapture`: 24 passed, file-link cases self-skip (`os error 1314`), every junction case runs.
- `cargo build -p girsync`, `cargo clippy -p girsync --all-targets`, `cargo fmt -p girsync --check` — clean, exit 0, no warnings.

### 7.8 Eighth pass — P8-1…P8-6

All six findings were confirmed against `374453d` and fixed at the root cause;
none was rejected. Two were Medium, four Low.

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| P8-1 | The hard-link preflight stat'ed every copy destination, so on POSIX a directory's `nlink` (always ≥ 2) refused every file-over-directory type conflict with a bogus message; `LinkInfo` now says whether the destination is a regular file, and only those are judged | fixed | `4dfb046` |
| P8-2 | A kept dst file link's dst-internal target looked `EXTRA`, so the delete phase removed the content the write-through needed: changed content exited 3 after the delete, equal content exited 0 with a dangling link, and a target src also mirrors was overwritten through the link; `spare_link_storage` resolves the chain at plan time and spares or refuses | fixed | `0385eee` |
| P8-3 | Both plan-time guards stat'ed the post-rename spelling, so on a case-sensitive filesystem a case-only dst link or hard link was invisible to a dry run (exit 0) while the real run renamed and then refused; a dry run now stats the spelling actually on disk | fixed | `1543725` |
| P8-4 | The case-fix temporary appended a suffix to the full source name, so a name within ~18 bytes of `NAME_MAX` failed `ENAMETOOLONG` and a legal case-only rename exited 3; the temporary is now a fixed short `.girsync_case_tmp.<n>` sibling | fixed | `e5df3d4` |
| P8-5 | A failed step-1 placeholder cleanup was discarded with `let _ =`, leaving tool residue in dst with no mention of the failure; the cleanup error is now attached to the primary error with the temporary's path | fixed | `0459a68` |
| P8-6 | The failure hook fired before the temporary was claimed, so neither the placeholder-cleanup nor the step-2 rollback path had a test; `..._STEP1`/`..._STEP2` hooks now exercise both and assert exit 3, no `RENAME`, no residue | tests | `0459a68` |

P8-2's plan order was reproduced pre-fix with a hard-link stand-in, because this
host refuses file symlinks: `sync --dry-run --output json` printed `delete
target.bin` then `copy a.bin`, with `"copied":1,"deleted":1`. P8-4 was reproduced
with a 244-byte source name: the old suffix built a 266-byte candidate, and the
run exited 3 on `stat ...girsync_case_tmp.0` (`os error 123`); the same fixture
passes now.

**Decisions still in force:**
- The hard-link preflight judges **regular-file** destinations only. A directory or directory link at a copy path is a type conflict, removed by `fix_dirs`/`copy_one` before the write, not a hard-link hazard.
- A kept dst file link's dst-internal target is that link's **content**, not an extra: it is spared from `delete_files`, from `diff.extra` (so the printed record and the action name the same set) and from `rmdir` together with every directory above it. Intermediate links on the chain are spared too.
- A target `src` also mirrors is one inode carrying two mirrored paths. A plan that would copy either end, or replace the target with a directory, is refused before any change: one file cannot be given two contents.
- The plan-time link and hard-link guards stat the spelling still on disk in a dry run and the post-rename path in a real run. `DiskSpelling` is that map, in both directions.
- The case-fix temporary is a fixed short sibling, claimed with `create_new`/`create_dir`; no part of its name derives from the source, so no source name can push it past the component limit.
- A failed step 1 removes its file placeholder and reports a cleanup failure beside the primary error; a failed step 2 rolls back and reports a rollback failure the same way.

### Latest verification (eighth pass)

- `cargo test -p girsync` — **409 passed, 0 failed, 1 ignored** (the pre-existing `tests/sync.rs` case); suite 410 functions, 409 run. Breakdown: lib 155; cli_dispatch 1, compare 13, compare_self 12, convert 4, hash_mode 18, helpers 12, lazy 27, migration_lock 11, output 16, rw_threads 15, stat_trust 19, symlink 28, sync 31 (+1 ignored), sync_plan 19, update 3, why 25; doctests 0.
- `cargo test -p girsync --test symlink -- --nocapture` — 28 passed, 0 failed. Nine file-link bodies print `skip: ... (os error 1314)` and return; one case-only body self-skips because this filesystem pairs `CaseProbe` with `caseprobe`.
- `cargo build -p girsync`, `cargo clippy -p girsync --all-targets`, `cargo fmt -p girsync --check` — clean, exit 0, no warnings.

### Open for maintainers

- **Hard links.** P6-2 closed the dst-hard-link write-through by refusing any destination with more than one link; P7-4 makes that decision at plan time, with a `(dev, ino)` / (volume serial, file index) identity, and credits names the plan removes first; P8-1 narrows it to regular-file destinations. Still open: no POSIX `nlink`/identity run (the pre-P8-1 over-refusal of directories is invisible on NTFS, where a directory reports one link), and no fixture for a hard link to a file outside both roots.
- **P7-2 runtime and P8-2/P8-3 end-to-end.** The dst-file-link-at-a-src-directory test and the new link-target spare/refuse cases self-skip here (the host refuses file symlinks, `os error 1314`), as do the case-only parity cases (this filesystem pairs `CaseProbe` with `caseprobe`). Their unit-level halves run and pass. Needs developer-mode Windows or POSIX CI to execute the end-to-end cases.
- **File links.** The file-link cases self-skip here because the host refuses file symlinks; they need a Windows host with developer mode or a POSIX CI. Native POSIX symlinks and Unix link behaviour are unexercised; junctions stand in for directory links only.
- **P7-1/P8-4 on POSIX and a crash between the renames.** The fixed short temporary was exercised on Windows only (including a 244-byte source name); a process killed between step 1 and step 2 was not produced (the next run steps over such a leftover by construction), and POSIX `NAME_MAX` was reasoned about rather than exercised. Whether a direct case-only `rename` is reliable enough to drop the two-step was not tested.
- **The failure injectors.** The cleanup and rollback tests use the test-only `GIRSYNC_TEST_FAIL_RENAME{,_STEP1,_STEP2}` hooks, because a real `fs::rename` failure cannot be arranged portably. P8-5's cleanup-error wording is not forced: a failing `remove_file` on the tool's own placeholder could not be arranged here, so only the cleanup path (not its error text) runs.
- **T-3** has no failing-first regression on a case-insensitive host (`fsutil file setCaseSensitiveInfo` is denied here); watch it on Linux CI. **T-4**’s non-link reparse classification is source-verified only: no OneDrive install, and tagging needs elevation.
- **F-6**’s mtime-restore failure still has no injected test; the doc note is the practical fix. **P6-1**’s `--keep-extra`/`--missing-only` variants are unit-covered, not all run end-to-end.
- **Other gaps.** `remove_dir_all` on a genuine directory containing a junction is not in the suite (an eighth-pass probe outside the repo removed such a directory and the junction's target outside the tree survived, closing the Windows half); T-2’s flush injector is test-only; T-6’s binary test is Windows-only.

---

*End of review. §1–§6 are the source-only original audit: no project file changed,
nothing built, tested or run. §7 records the implementation and the second through
eighth passes, which did build, test and run the binary; per-pass detail is in the
commits it cites.*
