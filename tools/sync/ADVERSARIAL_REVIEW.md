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
For a Windows junction, `is_symlink()` is false; see the open question in §5.

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
2. **Windows junctions — corrected in the second pass.** This section originally
   said `FileType::is_symlink()` is false for a junction and `is_dir()` is true.
   That is wrong on rustc 1.98.1: `symlink_metadata` on a junction reports
   `is_symlink = true`, `is_dir = false`, `is_file = false`, `attrs = 0x410`,
   while `path_is_symlink()` is true and followed `metadata()` is a directory.
   Older toolchains have been reported to differ, so the `FileType` bits are not
   a portable test on their own. The walk follows junctions (they are reparse points resolved
   by `fs::metadata`), so F-1’s deletion shape applies to them too. **The third
   pass corrected the predicate again:** `FILE_ATTRIBUTE_REPARSE_POINT` is
   necessary but not sufficient — cloud placeholders, dedup and AppExecLink set
   it too — so `src/link.rs` now starts from std’s name-surrogate
   `FileType::is_symlink` and falls back to `read_link`; see §9 T-4. Whether
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

## 7. Implementation progress

Landed on `fix-gaps/girsync`, scoped to `tools/sync`. The review text above is
unchanged except where this section marks a correction. This section records what
happened to each finding; §8 records the second adversarial pass that re-opened
F-1 and the fixes for it. The closing line still describes the original audit,
not the implementation.

| ID | Status | Change | Commits |
|----|--------|--------|---------|
| F-1 | **fixed**, covered after the second pass | Link-ness is carried from the walk (`link::try_is_link`, redirecting-reparse-tag based, so junctions are included; the predicate was narrowed in §9 T-4) into `LiveEnt`/`SideEntry`/`EffRec`. `build_plan` replaces any extra path under a dst link with the topmost link and drops its descendants from `delete_files`/`rmdir`; both apply passes re-check disk before removing. The second pass found that a **shared** dst link (src also has the path) still let writes and type-conflict fixes traverse it; `plan::ensure_link_safe` now refuses such a plan before the dry run or any filesystem change, and every destructive apply path re-checks its ancestors. Tests: inside-dst target, external target, `--keep-extra`, shared-link fix/copy/rename refusal, shared-link extras. | `1c827b2`, `42b9766`, `cca7218`; `9beede2`, `00f3e39`, `dae1a78`, `943e2d5` |
| F-2 | **fixed**, covered | `copy_one` unlinks a directory link at a file path with a platform-correct call (`link::remove`: `remove_dir` on Windows for a directory target, `remove_file` on Unix) instead of skipping it and failing in `File::create`. A file link is still followed and written through, and the README now says so. Test uses a directory link to an external target. | `2bec6b3`, `cca7218` |
| F-3 | **tests** | 17 link cases in `tests/symlink.rs`: three walk cases (file link stats as target, directory link subtree, dangling link aborts) and fourteen `sync` cases (src file link, src dir link, dst file-link write-through, F-2, F-1 inside/external, `--keep-extra`, shared-link fix/copy/rename refusal, shared-link extras fail vs `--keep-extra`, `compare-self` clean after link removal and after a type-conflict fix, `--exclude link`). Directory links fall back to a junction where `symlink_dir` needs a privilege. | `1c827b2`, `2bec6b3`, `42b9766`, `f4ffb59`, `943e2d5` |
| F-4 | **tests** | Depth-11 file with the default cap: `sync` exits 0, the deep file stays unmirrored, the shallow file is copied. | `d077a78` |
| F-5 | **tests** | Case-only pair (`src/a.txt`, `dst/A.txt`, equal bytes): exit 4, JSON `SUMMARY` has `case_mismatch=1`, `total_diff=0`. | `ddadcc9` |
| F-6 | **documented**, no test | The copy step now says a failed mtime restore aborts the run although the bytes are correct (a rerun recopies), and `copy_one` carries the same point at the call site. No behavior change. The review made a test optional; none was added. | `d077a78`, `1d723cb` |
| F-7 | **tests** + README | Test-only `GIRSYNC_TEST_FAIL_COPY` (fails one path before opening dst) plus `GIRSYNC_TEST_COMMIT_OPS=2`: four copies, failure on `f2.bin`, exactly two committed rows, the failing path uncached, rerun converges. The “No resume” note now states that the uncommitted cache tail is lost. | `d077a78` |

**Failing first.** The first-pass F-1 and F-2 tests were run against the pre-fix
tree (the fix files stashed). F-1 panicked reading `real/x` (`NotFound`) for both
the inside-dst and external-target fixtures; F-2 aborted with
`create ...\dst\a.bin: Access is denied. (os error 5)`. Both pass with the fixes.
The second-pass fixes were driven by the reviewer’s binary reproductions at
`6d0e118` (§8), not by a new failing-first run; the new tests pin the refusal
before any write.

**Final verification.** On Windows 11, on `fix-gaps/girsync` (source at `943e2d5`;
this section is a later docs-only commit):

- `cargo build -p girsync` — Finished, no warnings.
- `cargo test -p girsync` — 351 passed, 0 failed, 1 ignored
  (`tests/sync.rs`, a pre-existing `#[ignore]` for a known issue).
- `cargo nextest run -p girsync` — 351/351 passed, 1 skipped.
- `cargo clippy -p girsync --all-targets` — clean, no warnings.
- `cargo fmt -p girsync --check` — exit 0.

The suite has 352 test functions, 351 of them run by default.

This host refuses to create a file link without the privilege (`os error 1314`),
so the four file-link cases (`walk_follows_a_file_link`,
`walk_aborts_on_a_dangling_link`, `sync_materializes_a_src_file_link`,
`sync_writes_through_a_dst_file_link`) print
`skip: this platform refused a file link: …` and return early; they count
as passes in the 17/17 `tests/symlink.rs` result. Every directory-link case runs
here through `cmd /c mklink /J`, including F-1 and F-2.

**Not verified; decisions left.**

- True `symlink_dir` (as opposed to a junction) is not exercised on this host,
  and no Unix link path was run. Junctions are detected by a redirecting reparse
  tag; since §9 T-4 the test starts from std’s name-surrogate
  `FileType::is_symlink` (true for them on rustc 1.98.1) and falls back to
  `read_link`.
- `std::fs::remove_dir_all` on a *genuine* directory that contains a junction is
  still unexercised; only paths that are themselves links are special-cased. The
  pre-existing recursive removals keep their old behavior.
- A shared link is now a hard boundary: default `sync` fails before touching the
  filesystem if it would write, delete or rename through one, or if it would
  leave extras under one. `--keep-extra`/`--missing-only` opt out of the extras
  failure only and warn that the run does not converge; writes remain refused.
- An mtime-restore failure (F-6) still has no injected test.

---

## 8. Second adversarial pass (N-1…N-8)

A second review re-audited the first-pass tip `6d0e118` with the built binary in
`E:\Symlinks\Temp\audit2`, a standalone junction probe on rustc 1.98.1, and the
source at that tip. Its notes are folded in here; the notes file itself is
removed. All eight findings are **valid**. N-1, N-2, N-3, N-4 and N-5 were
reproduced with the binary or by `compare-self`; N-6 is a cost claim confirmed
from the call sites; N-7 and N-8 are confirmed from the source.

| ID | Severity | Verdict | Resolution | Commits |
|----|----------|---------|------------|---------|
| N-1 | high | valid, fixed | Shared-link type conflict deleted external data through `fix_dirs` and `copy_one`. `plan::ensure_link_safe` now refuses any action with a strict link ancestor before the dry run or apply, and `mkdirs`, `fix_dirs`, `copy_one`, `rename_to_src_casing` re-check. | `00f3e39`, `dae1a78`, `943e2d5` |
| N-2 | medium | valid, fixed | (a) Cache rows under a removed link are pruned by prefix; (b) `fix_dirs` writes its `FileRec::dir()` row. `compare-self` is clean after either. | `dae1a78`, `943e2d5` |
| N-3 | medium | valid, fixed | An excluded directory now excludes its subtree; include-filtered descendants keep their ancestor directories, so a link above them stays visible. Dry run and real run name the same path. | `5092cce`, `943e2d5` |
| N-4 | medium | valid, resolved | Shared link with extra descendants: default `sync` now fails instead of exiting 0; `--keep-extra`/`--missing-only` warn and accept the non-convergence. | `00f3e39`, `943e2d5` |
| N-5 | medium | valid, fixed | Rename through a shared link is refused before the rename. | `00f3e39`, `943e2d5` |
| N-6 | low/medium | valid, fixed | Reparse check gated on `ft.is_dir()`; `try_is_link` is one `symlink_metadata`; `topmost_link_ancestor` is one stat per component, and its comment says so. | `9beede2` |
| N-7 | low | valid, fixed | Junction doc corrected; README symlink/filter bullets rewritten to match the code; the `link.rs` “only place” claim softened; `prune_dst_cache` comment made true; test count and §7 overclaim corrected. | `9beede2`, `5092cce`, `dae1a78`, this commit |
| N-8 | low | valid, fixed | `try_is_link` propagates non-`NotFound` errors; `link::remove` propagates metadata errors; removal helpers match `NotFound` explicitly and propagate the rest; `fix_dirs` uses `link::remove`. | `9beede2`, `dae1a78` |

### N-1 — shared-link type conflict deleted external data

Reproduction A: `src/link/x/y.txt`; `dst/link` a junction to `outside`;
`outside/x` a file holding external bytes. `sync` reported
`type-conflict link/x` then `fix-dir link/x`, replaced the external file with a
directory containing src’s `y.txt`, and exited 0 with `deleted=0`.

Reproduction B: `src/link/x` a file; `dst/link` the same junction;
`outside/x/y.txt` external. The planner refused the extra under the shared link,
but `copy_one`’s `remove_dir_all(dst/link/x)` deleted the external subtree and
wrote src’s file; exit 0, `deleted=0`.

The class is “a destructive operation must not traverse a link ancestor”, not
just the delete-extras path. The branch chooses fail-fast: `ensure_link_safe`
iterates every `mkdir`/`copy`/`fix_dirs`/`delete_files`/`rmdir` action, asks
`link::link_ancestor_above`, and bails naming the link before the dry run or the
first filesystem change. The apply phases re-check as belt and braces. Tests:
`sync_refuses_to_fix_a_directory_through_a_shared_dst_link`,
`sync_refuses_to_copy_through_a_shared_dst_link`,
`sync_refuses_to_rename_through_a_shared_dst_link`.

### N-2 — cache drift after a successful sync

(a) Removing a dst-only link dropped only the planned row, so `compare-self`
reported the followed descendant as missing and exited 4.
`predrop_cache_entries`, `remove_unknown_dirs` and `prune_dst_cache` now drop
every row with the removed link’s prefix. Test
`a_sync_that_removes_a_link_leaves_compare_self_clean`.

(b) `fix_dirs` created the directory but never wrote a cache row, so
`compare-self` reported the new directory as extra — for a plain type conflict
as well as under a link. It now writes `FileRec::dir()`. Test
`a_type_conflict_fix_leaves_compare_self_clean`.

### N-3 — `--exclude` of a directory

`--exclude link` removed the exact entry but kept `link/x.txt`, because the glob
did not match the descendant. The plan therefore scheduled the descendant, and
the runtime guard deleted the excluded link: dry run said `DELETE link/x.txt`,
the real run said `DELETE link`. `filter.rs` now treats an excluded directory as
excluding its whole subtree; `effective/phase_a.rs` keeps ancestor directories
when an include matches only a descendant, which is what keeps a link above an
included path visible. Test `excluding_a_dst_link_leaves_it_and_its_target_alone`
asserts the excluded link and its target survive and the dry/real records agree;
unit tests cover subtree exclusion and ancestor retention.

### N-4 — shared link with extra descendants

`sync` reported `extra link/y.txt`, deleted nothing, and exited 0; `compare` on
the same tree exited 4. `build_plan` records `shared_link_extras`;
`ensure_link_safe` fails by default because the run cannot converge without
deleting through the link. `--keep-extra` and `--missing-only` deliberately allow
that extra-only non-convergence and log a warning; writes are still refused.
Test `a_shared_link_with_extras_fails_unless_keep_extra`.

### N-5 — rename through a shared link

`rename_to_src_casing` renamed `dst/link/x.txt` to `dst/link/X.txt`, mutating the
external target and exiting 0. It now refuses when the path has a link ancestor,
and the plan guard catches the same case earlier. Test
`sync_refuses_to_rename_through_a_shared_dst_link`.

### N-6 — scan and deletion cost

The walk called the reparse check for every entry, including regular files;
`is_link` cost two `symlink_metadata` calls per non-link component. The reparse
check is now gated on `ft.is_dir()`, `try_is_link` is one `symlink_metadata`, and
`topmost_link_ancestor` is one stat per component. The comment in `link.rs` now
matches the cost.

### N-7 — documentation and claims

- The junction claim is corrected in §5.2 and in `link.rs`: on rustc 1.98.1 a
  junction reports `is_symlink = true`, `is_dir = false`, `attrs = 0x410`; the
  reparse attribute is the test, not the `FileType` bits.
- The README symlink bullet now says a dst file link is written through, a dst
  directory link at a file path is unlinked, a path under a dst link is never
  deleted through it, and a shared link fails before writes/renames. The filter
  bullet documents subtree exclusion and ancestor retention. The old absolutes
  (“not deleted through it”, “left untouched”) are scoped to the delete pass.
- “The README documents write-through” is now true: the sentence was added.
- `link.rs`’s “the only place that decides link-ness” became “the single
  definition of link-ness; … every destructive apply path asks them”, and those
  paths do use `try_is_link`/`link_ancestor_above`.
- `prune_dst_cache`’s “all three put a cache row” comment is true now that
  `fix_dirs` writes its row.
- The test count is 352 functions / 351 run, and §7’s “F-1 fixed, covered” is
  corrected above.

### N-8 — swallowed errors

`try_is_link` propagates every error except `NotFound`; `link::remove` propagates
a metadata error instead of guessing; the delete/rmdir/link removal helpers match
`NotFound` explicitly and propagate everything else, so a failed unlink of a
dangling link is no longer hidden; `fix_dirs` uses `link::remove` rather than raw
`remove_file`. `delete_extras` still treats an absent path as done, which is
correct after the pre-drop; the plan guard makes the link case unreachable.

### Policy decision

The reviewer’s preferred single decision was “shared links are unsupported, fail
fast.” That is what the branch now does: a default sync refuses any
write/delete/rename through a shared link and refuses to leave extras under one.
`--keep-extra`/`--missing-only` are explicit opt-outs for the extras case only,
and they warn that the run does not converge. This resolves N-1, N-4 and N-5
with one testable rule instead of the earlier hybrid.

---

## 9. Third adversarial pass (T-1…T-6)

A third review re-audited `e8585fa` with the debug binary in
`E:\Symlinks\Temp\rev3new` and the current source. Its notes are folded in here;
the notes file itself is removed. T-1, T-2 and T-6 are **valid**; T-3 and T-4 are
**valid but latent on this host**; T-5 is a valid dead-code/doc defect. T-1 and
T-6 were reproduced with the binary, T-2 is confirmed from the source (there was
no portable flush injector), T-3 is derived from the rename order and the two
`fs::rename` calls, and T-4 needs a non-link reparse point (none was available).

| ID | Severity | Verdict | Resolution | Commits |
|----|----------|---------|------------|---------|
| T-1 | high | valid, fixed | Nested/overlapping roots are refused before any cache or file change. Canonical roots, component-wise containment (`A`/`AB` is not nesting; case-insensitive on Windows). | `71c473c`, `2f536de`, `e0ecfff` |
| T-2 | medium | valid, fixed | A failed `sync_all` aborts the copy with exit 3; no cache row is written. Test-only `GIRSYNC_TEST_FAIL_FLUSH` injects it. | `924e475`, `2f536de`, `e0ecfff` |
| T-3 | medium | valid (latent), fixed | Parent-first renames rewrite each child through the directory moves already performed; a child the parent move covered is a no-op and emits no `RENAME`. Map/cache descendants are re-keyed. | `8279581`, `2f536de` |
| T-4 | low | valid (latent), fixed | Only redirecting reparse points are links. std’s name-surrogate test first, `read_link` as the old-toolchain fallback; the reparse-only helper and the dead public `is_link` are gone. | `2477e0c`, `e0ecfff` |
| T-5 | low | valid, fixed | Dead public `link::is_link` deleted; the test-side mirror no longer claims the predicate is unexported and mirrors the narrowed test. | `2477e0c` |
| T-6 | low | valid, fixed | `cache_identity` canonicalizes the **root** and joins the cache name, so `root` and `ROOT` collide on a fresh tree; a lock error no longer advises `--ignore-cache`. | `08c19ae`, `2f536de`, `e0ecfff` |

### T-1 — nested and overlapping roots

Reproduction A: `src=root`, `dst=root/sub` exited 0, overwrote `root/sub/y.txt`
with src’s bytes and created `root/sub/sub/y.txt`. Reproduction B:
`src=root/sub`, `dst=root` deleted the source’s `sub/x.txt` and `sub/y.txt`, then
exited 3. `validate_inputs` now canonicalizes both roots and refuses containment
in either direction, before the backup/cache step and in `--dry-run` too. The
comparison is component-wise, so `root`/`root2` is allowed and `C:\AB` is not
inside `C:\A`; on Windows it ignores case, which is what makes the same check
cover a case-only spelling. Test
`run_sync_refuses_nested_roots_before_touching_anything` asserts exit 3, the
`disjoint` message, unchanged bytes, no nested copy and no cache in either
direction, plus the allowed sibling. It was run against the pre-fix tree (the
`mod.rs` change stashed) and failed: the first direction exited 0.

### T-2 — a failed flush was ignored

`copy_one` called `fout.sync_all().ok()`. A real flush failure (disk full,
quota, a network volume that went away) was therefore followed by the verify
re-reading the same page cache the flush never wrote through, caching the
digest, and exiting 0. `flush_file` now propagates the error with the dst path
as context, so the copy fails before its record is returned to the cache and a
rerun recopies. `GIRSYNC_TEST_FAIL_FLUSH` names one relative path; the test
overrides the workspace’s nextest `GIRSYNC_TEST_NO_FSYNC=1`, since a skipped
flush cannot fail. `a_failed_flush_aborts_before_the_copy_is_cached` was run
with `copy.rs` stashed and failed on exit 0 versus 3; it also asserts no row for
the file and a clean rerun.

### T-3 — parent-before-child renames

`case_renames` sorts by lowercase src key, so a directory precedes everything
beneath it. The old loop renamed the parent and then called `fs::rename` on the
child’s **old** path; on a case-sensitive host that path is gone, so the run
exited 3 after printing `RENAME` records it did not perform. `plan_steps`
rewrites each child’s `from` through the directory moves already planned
(longest component-boundary prefix), marks a child covered by its parent as a
no-op, and `final_key` re-keys the dst map and cache for exact renames and for
dst-only descendants of a renamed directory. Three unit tests pin the ordering,
the longest-prefix rewrite and the descendant re-key; the integration case
`run_sync_renames_a_case_only_directory_before_its_children` runs here but
**cannot fail first on this host**: the filesystem is case-insensitive, so the
old child rename resolved through the new parent casing and succeeded, and
`fsutil file setCaseSensitiveInfo` is denied. On a case-sensitive host it is the
failing-first regression.

### T-4 — every reparse point was a link

`FILE_ATTRIBUTE_REPARSE_POINT` is set by cloud placeholders, dedup and
AppExecLink as well as by symlinks and junctions. A tagged directory was
therefore walked as `is_link = true`, refused as a link ancestor by
`ensure_link_safe`/`link_ancestor_above` (with a diagnosis that names a link
the user does not have), and routed to `remove_dir` instead of `remove_dir_all`
on deletion. The predicate now starts from `FileType::is_symlink` on
`symlink_metadata` — std’s name-surrogate test, true for symlinks, junctions and
WSL links, false for the non-redirecting tags — and falls back to `read_link`
for a toolchain whose `FileType` does not know a junction. The fallback matters
for the error contract: std returns `Uncategorized` (“Unsupported reparse point
type”), not `InvalidInput`, for an unknown tag, so the code treats a failed
`read_link` after the reparse bit as “ordinary entry” rather than propagating an
error that would refuse to walk the directory. `scan.rs` propagates a real
`try_is_link` error for a directory. A non-link reparse point could not be
created on this host (no OneDrive install; setting a tag needs elevation), so
only regular entries and the existing link suite are exercised at runtime.

### T-5 — dead public `is_link` and a stale comment

`link::is_link` had no caller, was exported through `pub mod link`, and its own
doc said not to use it in a destructive guard; the test-side mirror’s comment
claimed it was not exported. It is deleted. `tests/common/mod.rs` now says it
mirrors `try_is_link`, and the mirror implements the narrowed predicate so it
cannot call a non-link reparse point a link.

### T-6 — case-only root spellings

`cache_identity` canonicalized the cache **file**, which does not exist on a
fresh tree, and fell back to the raw path. `root` and `ROOT` then compared
unequal, the src side created `root/girpr-cache`, and the dst open failed with
`Database already open. Cannot acquire lock.` plus the harmful
`(corrupt? use --ignore-cache)` advice: `--ignore-cache` deletes the cache the
src side just made and turns the run into the meaningless self-pair the guard
exists to refuse. It now canonicalizes the root and joins `CACHE_PREFIX`,
compares with `same_path` (case-insensitive on Windows), and `cachedb::open_file`
distinguishes `DatabaseAlreadyOpen` from corruption, naming the likely cause
without suggesting `--ignore-cache`.
`run_sync_refuses_case_only_spellings_of_one_root` (`#[cfg(windows)]`) asserts
exit 3, the `resolve to the same cache` message, no `--ignore-cache` advice and
no cache file; it was run with `side.rs` stashed and failed on the lock message.
With T-1’s containment check also present, the distinct-sides check now runs
first and gives the accurate diagnosis; the T-1 check would otherwise have
called the pair “inside” rather than “the same cache”.

### Re-checked, no finding

The F-1/F-2 guards are still at their call sites (the planner remap, both apply
re-checks, the shared-link refusal before apply), and the N-1…N-8 guards the §8
table cites are still present: subtree-aware exclusion, ancestor retention,
cache pruning in step with dst, and the reparse check gated on `is_dir()`. Two
probes agreed with the earlier passes: `remove_dir_all` on a genuine directory
containing a junction removed only the junction (the external target survived,
exit 0), and `root` vs `root\.` is still refused with the same-cache message
(exit 3). This was a presence check, not a fresh audit of every §8 wording.

### Surviving decisions

- A mirror inside its own source is refused, not supported. There is no
  “exclude the destination automatically” mode.
- A failed flush is fatal. The copy is not retried in-process; a rerun copies
  the file again because no row was written.
- One directory rename is one `RENAME`; descendants that the move already put
  on src’s casing are silent. The cache and map still follow them.
- A Windows link is a redirecting reparse point. Non-link reparse points are
  ordinary data, copied and removed as such.
- A lock is not corruption: `--ignore-cache` is never suggested for
  `DatabaseAlreadyOpen`.
- Two case-only spellings of one root are one cache on a case-insensitive
  filesystem, whether or not the cache file exists yet.

**Failing first.** T-1, T-2 and T-6 were each run against the pre-fix tree with
only that fix file stashed, and each failed (T-1: exit 0; T-2: exit 0 instead of
3; T-6: the lock message instead of the same-cache message). T-3’s integration
case and T-4’s non-link reparse case cannot be made to fail on this host for the
reasons above; their unit tests and the link suite pin the new decision.

**Final verification.** On Windows 11, on `fix-gaps/girsync` (source at
`2f536de`; the README/DESIGN-NOTES and this section are later docs commits):

- `cargo build -p girsync` — Finished, no warnings.
- `cargo test -p girsync` — 361 passed, 0 failed, 1 ignored
  (`tests/sync.rs`, the same pre-existing `#[ignore]`).
- `cargo clippy -p girsync --all-targets` — clean, no warnings.
- `cargo fmt -p girsync --check` — exit 0.
- `cargo test -p girsync --test symlink -- --nocapture` — 17 passed, 0 failed;
  the same four file-link cases print the `os error 1314` skip line, and every
  junction case runs.

No file outside `tools/sync` was changed.

**Open for the maintainer.** T-3’s only failing-first regression is the
case-sensitive-host integration test, which this Windows host cannot enable; it
should be watched on the Linux CI. T-4’s non-link reparse classification is
source-verified only (no OneDrive/elevation here). T-2’s injector is test-only;
production takes the same `sync_all` call. T-6’s binary test is Windows-only.

---

## 10. Fourth adversarial pass (F-1)

A fourth review re-audited `2315990` with the debug binary in
`E:\Symlinks\Temp\rev4-linkcontain` and the current source. Its finding is
**valid** and fixed; the note is folded in here and deleted. No other finding
survived: the third-pass fixes were re-checked at their sites and all still
apply.

| ID | Severity | Verdict | Resolution | Commits |
|----|----------|---------|------------|---------|
| F-1 | high | valid, fixed | A link inside the effective src that reaches dst (target inside dst, or containing it) is refused after the src walk and before planning; no removal may reach a path whose directory resolves inside src. | `8057f9a`, `d3c8f00`, `174ba52`, `49e7d49` |

### F-1 — a src link hid the root overlap and the delete pass ate dst

`validate_inputs` canonicalized the two **roots** and refused containment in
either direction, but the walk follows links inside a root. A directory link
`src/link -> dst` left the two roots looking like siblings while the walk
reported every dst file twice: at its real path, which the diff called `EXTRA`,
and under `link/`, which it called `MISSING`. The delete pass removed the real
path first, then the copy of `link/...` failed — data loss reported only as a
copy error. A link to a directory **containing** dst had the same shape from
above, and a file link into dst is the same hazard for one file. The comment in
`mod.rs` claiming canonical paths made a link unable to hide the overlap, and
the README’s matching claim, were false for links inside src.

Reproduction (Windows junction; no elevation needed):

```powershell
$b = "E:\Symlinks\Temp\rev4-linkcontain"
New-Item -ItemType Directory -Path "$b\root","$b\outside" | Out-Null
Set-Content "$b\outside\x.txt" "PRECIOUS"
cmd /c mklink /J "$b\root\link" "$b\outside"
target\debug\girsync.exe sync --src "$b\root" --dst "$b\outside" --output json
```

Observed: `{"event":"delete","path":"x.txt"}`, then exit 3 naming only a cache
copy failure; `outside\x.txt` no longer existed. The pre-fix tree failed the
three new directory-link cases and the nested-cache case when the source fix
was stashed.

**Resolution.** `validate_inputs` now returns both canonical roots.
`validate_src_links` runs immediately after src phase A — so filters and
`--max-depth` have already decided what this run’s source is — and canonicalizes
every link the effective map kept, refusing a target inside dst or a target
that contains dst. It checks file links too, since one file is enough. A link to
anywhere else remains ordinary mirrored content, including a target back inside
src. The error joins the `disjoint` family, names the link and both resolved
paths, and exits 3 before dst is scanned, planned, renamed or applied.

Defence in depth: `guard_removal_outside_src` in `apply/phases.rs` canonicalizes
each delete target’s **parent** in both `delete_extras` and
`remove_unknown_dirs` and refuses one inside src. The final component is not
resolved, so a link at the target is still removed as a link and its target
survives. A missing parent is left to the caller’s own `NotFound` handling; any
other resolution error aborts rather than reading as “outside src”.

**Cache pruning.** The same run exposed `is_cache_rel`’s first-component-only
test: the dst cache reached through the link was planned as `link/girpr-cache`.
A cache belongs to the root it sits in, so a nested root carries
`<root>/sub/girpr-cache` and a followed link exposes its target’s. The predicate
now reserves a `girpr-cache*` component at **any** depth, matching README’s
“always excluded” and `util.rs`’s “never part of a mirror”; ordinary names that
merely contain the prefix are untouched. This also keeps a linked nested root’s
live database (and its backups) out of the mirror.

**Tests.** `sync_refuses_a_src_directory_link_to_dst`,
`..._inside_dst`, `..._containing_dst` and
`sync_refuses_a_src_file_link_into_dst` assert exit 3, a message containing
`disjoint` and `link`, unchanged destination bytes, and that nothing was created
in dst; `a_src_link_does_not_mirror_a_nested_cache` pins the pruning rule.
`cache_paths_are_reserved_at_any_depth` and the extended
`cache_paths_are_pruned_along_with_their_contents` pin the predicate at unit
level, and `a_removal_whose_parent_resolves_inside_src_is_refused` pins the
removal guard’s allowed and refused shapes. The junction fallback now passes
native separators to `mklink` (a forward slash is a switch) and reports the
fallback error instead of the misleading `symlink_dir` one.

**Final verification.** On Windows 11, on `fix-gaps/girsync` at the docs commit
above (this section is a later docs commit):

- `cargo build -p girsync` — Finished, no warnings.
- `cargo test -p girsync` — 368 passed, 0 failed, 1 ignored (the pre-existing
  `tests/sync.rs` created/removed-in-one-run case).
- `cargo test -p girsync --test symlink -- --nocapture` — 22 passed, 0 failed;
  every directory-link case ran through a junction. Five file-link cases print
  the `os error 1314` skip line (developer mode is off), including the new
  file-link regression, which therefore compiles and asserts but does not
  execute here.
- `cargo clippy -p girsync --all-targets` — clean, no warnings.
- `cargo fmt -p girsync --check` — exit 0.

Suite breakdown: lib 131, cli_dispatch 1, compare 13, compare_self 12, convert 4,
hash_mode 18, helpers 12, lazy 27, migration_lock 11, output 16, rw_threads 15,
stat_trust 19, symlink 22, sync 20 (+1 ignored), sync_plan 19, update 3, why 25.

**Surviving decisions.** The mirror-src overlap rule is symmetric through
links: a link’s *effective* target may not touch dst, but a link pointing
elsewhere is content. Cache prefixes are reserved at every depth. A removal
guard repeats the disjointness test at the filesystem, so a plan is never the
last word on what may be deleted.

**Not verified.** Linux/macOS link behavior; this host refuses file symlinks
(os error 1314), so only directory junctions and plain directories ran. The
file-link refusal is source-verified: the check iterates `is_link` entries
without filtering by kind, and the test is present for a host that can create
one. Non-link reparse points still have no available fixture.

**Open for the maintainer.** The file-link regression needs a Windows host with
developer mode (or a POSIX CI) to run; it is the only new case that self-skips
here.

---

*End of review. Sections 1–6 describe the original source-only audit: no source,
test, Cargo, README, or other project file was changed and no build, test, or
binary run was performed for them. Sections 7–10 record the implementation and
the second, third and fourth passes, which did build, test and run the binary.*
