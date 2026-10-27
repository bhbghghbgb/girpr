# Third adversarial pass — `girsync` (`tools/sync`)

Tree reviewed: `e8585fa` (`fix-gaps/girsync`, clean). Date: 2026-10-28.
Method: re-read the code the first two passes cite, then exercise the built binary against
scratch trees under `E:\Symlinks\Temp\rev3`. No source, test, Cargo, README or review file
was modified; this note is the only file created.

Severity scale: as `ADVERSARIAL_REVIEW.md` §1 — `high` = data loss or writes/deletes outside
the mirror root; `medium` = a run fails or produces the wrong result; `low` = documentation
or test gap with no incorrect result on the covered paths.

## 1. Verdict

| ID | Severity | Class | Location | Summary |
|----|----------|-------|----------|---------|
| T-1 | **high** | bug | `src/commands/sync/mod.rs:389-399`, `src/effective/side.rs:216-231` | Nested `src`/`dst` roots are accepted; `dst` inside `src` overwrites a source file and exits 0, and `src` inside `dst` deletes both source files |
| T-2 | **medium** | bug (contradicts a documented guarantee) | `src/commands/sync/apply/copy.rs:187-189`, `README.md:321-325` | `sync_all()`'s error is dropped, so a failed flush still verifies from the page cache, caches a digest, and exits 0 |
| T-3 | **medium** | bug | `src/commands/sync/rename.rs:45-54,96-124` | On a case-sensitive host the pass renames a parent before its children, so every child rename then fails with `ENOENT` and the run exits 3 after printing renames it did not perform |
| T-4 | **low** | bug (latent on this host) | `src/link.rs:62-72,80-93,162-177` | Every Windows reparse point is classified as a link, so non-link reparse points (cloud placeholders, dedup, AppExecLink) are refused as link ancestors and handed to link removal |
| T-5 | **low** | dead code / doc error | `src/link.rs:39-45`, `src/lib.rs:47`, `tests/common/mod.rs:166-167` | `link::is_link` has no caller but is exported; the test comment says it is "not exported from the public surface" |
| T-6 | **low** | bug / doc error | `src/effective/side.rs:225-231`, `README.md:913-914` | `cache_identity` falls back to the raw path when the cache file is absent, so case-only spellings of one root escape the distinct-sides check and fail with "Database already open … use --ignore-cache" |

## 2. Findings

### T-1 — Nested roots are accepted; `sync` overwrites and deletes its own source (high, bug)

**Code.** `validate_inputs` runs the only input disjointness check there is, and it compares
cache paths, not roots:

```rust
/// Reject the inputs that make a mirror meaningless, before anything is touched.
fn validate_inputs(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    if is_record_path(src) || is_record_path(dst) {
        bail!("sync needs folder vs folder (record inputs are compare-only)");
    }
    if !src.is_dir() || !dst.is_dir() {
        bail!("src and dst must both be directories");
    }
    // One cache per run: two spellings of one folder would otherwise make the
    // mirror delete src out from under itself, on top of the handle clash.
    ensure_distinct_sides(&classify(src), &classify(dst))?;
    Ok(())
}
```

`ensure_distinct_sides` (`src/effective/side.rs:216-223`) compares `cache_identity`, i.e.
`<root>/girpr-cache` canonicalized (`:228-231`); its doc (`:200-215`) says this "covers every
shape of the collision in one predicate". Nested roots have two different cache files, so
nothing runs. There is no containment check anywhere in `src/` (`grep -n "starts_with|strip_prefix"`
finds only the walk's relativization, cache-name and filter uses).

**Failure.** `root/y.txt`, `root/sub/y.txt` (both source files), `--src root --dst root/sub`,
memory cache, default flags:

```
MISSING sub
MISSING sub/y.txt
CHANGED y.txt
MKDIR sub
COPY sub/y.txt because=missing
COPY y.txt because=changed
SUMMARY renamed=0 mkdir=1 copied=2 deleted=0 rmdir=0 ... dry_run=false
exit=0
```

Afterwards `root/sub/y.txt` contains `SRC-Y` (it was `SRC-SUB-Y`) and `root/sub/sub/y.txt`
contains `SRC-SUB-Y`: a source file was overwritten by its parent's namesake, a nested copy
was created, and the run reported success. Adding one dst-only file (`sub/deep/z.txt`) makes
the deletes run first: `DELETE deep/z.txt` removes the source file, then
`copy sub/deep/z.txt: stat ...\root\sub/deep/z.txt: The system cannot find the file specified.
(os error 2)` aborts with exit 3. The reverse nesting (`--src root/sub --dst root`) is worse:

```
DELETE sub/x.txt
DELETE sub/y.txt
exit=3
```

Both source files are gone and the target `y.txt` was never updated. A `--dry-run` exits 0 and
prints the same destructive plan, so it gives no warning.

**Standard violated.** `dst` is a mirror of `src`; a mirror run must not modify `src`. The
refusal that exists is justified by exactly this ("a self-collision makes the run
meaningless", `side.rs:209-215`), and `validate_inputs`'s own doc claims to reject the inputs
that make a mirror meaningless "before anything is touched". README:720 and :909-914 name only
cache collisions as refused pairs.

**Fix.** After the two `is_dir` checks, canonicalize both roots and refuse when either
`starts_with` the other (Rust's `starts_with` is component-wise, so `C:\A` does not match
`C:\AB`); compare case-insensitively on Windows. Reuse that helper for T-6. Message should
name the direction and exit 3. Add a binary test for both directions asserting exit 3 **and**
that every source byte is unchanged; this is the missing test that would have caught it.

### T-2 — A failed flush is swallowed, and the verify then hashes the page cache (medium, bug)

**Code.** `copy_one` writes, flushes, and drops the result of the flush:

```rust
if !test_no_fsync() {
    fout.sync_all().ok();
}
drop(fout);
```

The next steps (`copy.rs:200-208`, `verify_digests` at `:59-64`) restore the mtime and re-read
the file. README:312-325 sells that re-read as a durability check:

> A copied file is read from src once (the stream), written, `sync_all`'d, and then **re-read
> from dst and hashed again** … because the second one happens after `sync_all()`, it hashes
> the bytes that actually reached the disk — the only way a copy catches silent corruption on
> the way down.

`ADVERSARIAL_REVIEW.md:317-322` (F-6) goes further and claims the opposite of the code: "A
failure in either call is propagated with `?`" and "The bytes are already correct and durable
at that point". Neither holds for `sync_all`.

**Failure.** A write can be accepted into the OS cache and fail only at the flush (disk full,
quota, `ERROR_IO_DEVICE`, network volume gone). `.ok()` discards that error; `set_modified`
succeeds; the verify re-read is served from the same cache, so the digests match; `copy_one`
returns the record and the run exits 0 with the digest cached. After a crash or remount the
file can carry the right size and mtime with zero/partial data; phase A then trusts
size+mtime+cached digest, so the corruption is not re-copied. The README guarantee is false
in exactly the case it exists for.

**Standard violated.** A zero exit must not assert durability the code did not obtain; the
verification is documented as reading "the bytes that actually reached the disk".

**Fix.** `fout.sync_all().with_context(|| format!("flush {}", d.display()))?;` before the drop,
so the copy — and the cache insert that follows — never happens on a failed flush. If
best-effort flush is intended instead, remove the README sentence and state that verify reads
through the OS cache. The mtime failure immediately below is already fail-fast
(`copy.rs:191-204`); the durability barrier should not be weaker than the timestamp.

### T-3 — Parent-before-child renames fail on case-sensitive hosts (medium, bug)

**Code.** Renames are ordered by lowercase path and applied unconditionally:

```rust
let mut by_lower: Vec<(String, &String)> = slow.into_iter().collect();
by_lower.sort();
...
for (from, to, drel, srel) in renames {
    report.emit(Record::pair("RENAME", "->", "from", &drel, "to", &srel));
    ...
    let tmp = from.with_extension("girsync_case_tmp");
    std::fs::rename(&from, &tmp)
        .with_context(|| format!("rename {} (case fix step1)", from.display()))?;
    std::fs::rename(&tmp, &to)
        .with_context(|| format!("rename to {} (case fix step2)", to.display()))?;
```

Both maps contain directory rows (`src/effective/phase_a.rs:155`), so a directory and every
path below it are queued. The sort is lexicographic on the lowercase key: `dir` precedes
`dir/a.txt`.

**Failure.** Default (insensitive) mode is the default on every platform (`src/cli.rs:184-186`);
the pass runs whenever `!case_sensitive` (`src/commands/sync/mod.rs:273-277`). On a
case-sensitive filesystem with src `Dir/A.txt` and dst `dir/A.txt`, the directory rename
`dir`→`Dir` runs first. The child entry still says `from = dir/A.txt`; that path no longer
exists (the file moved with its parent), so step 1 fails with `ENOENT`. The run exits 3 after
`report.emit` has already printed both `RENAME` records (`:100-101`), and after `dm` was
re-keyed for the child (`:107-109`) although its cache row was never moved (`:126-131`). If
only the directory component differed, the tree is already correct at that point and a rerun
is a no-op; if a child's leaf casing also differed, the first run leaves it unchanged and a
rerun completes it. Either way the first run fails a legitimate case-only directory rename and
reports renames it did not perform. The pass's own comment (`:83-86`)
says a partial pass is unacceptable and that it validates the whole pass up front; the fixed
order makes that validated plan stale before the first rename.

**Standard violated.** A validated plan may not be invalidated by its own first action; a
`RENAME` record must describe a rename that happened.

**Fix.** Keep the queue but rewrite `from` through the renamed-directory prefixes as the loop
proceeds, and skip entries whose `from` then equals `to`; entries whose leaf name still
differs will succeed because their parent is now at the new casing. Add a test with a
case-only directory rename containing a file, run in default mode on a case-sensitive
filesystem.

### T-4 — Reparse point is not the same predicate as link (low, latent on this host)

**Code.** Link detection is one attribute bit:

```rust
fn is_link_metadata(md: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    ...
}
```

and `is_reparse_point` (`:80-93`) uses the same test. The doc claims "True when `path` itself
is a link or a junction, whichever the platform calls it" (`:36-37`). `FILE_ATTRIBUTE_REPARSE_POINT`
is set by every reparse tag, including tags that do not redirect (cloud placeholders, dedup,
AppExecLink), not only `IO_REPARSE_TAG_SYMLINK`/`IO_REPARSE_TAG_MOUNT_POINT`.

**Failure.** A directory carrying such a tag is walked with `is_link = true`
(`src/scan.rs:90-91`); then `plan::link_ancestor` (`plan.rs:112-126`) and
`ensure_link_safe`/`link_ancestor_above` (`plan.rs:316-353`, `link.rs:162-177`) treat it as a
link ancestor. Every `mkdir`/`copy`/`fix_dirs`/delete/rename below it fails with "refusing to
… through dst link … remove the link, or exclude it", which is the wrong diagnosis and a
refusal with no way to proceed except excluding real data. In the delete path the same tag
routes the entry to `link::remove` (`link.rs:110-139`), which on a directory calls
`remove_dir` instead of `remove_dir_all`, so a non-empty tagged directory fails with `ERROR_DIR_NOT_EMPTY`
(or an empty one is removed as if it were a link).

**Standard violated.** A classification named "is a link" must match the platform's link
kinds; an over-match in a destructive-guard predicate converts ordinary paths into refusals
and into the wrong removal routine.

**Fix.** Read the reparse tag (`FSCTL_GET_REPARSE_POINT`) and return true only for
`IO_REPARSE_TAG_SYMLINK` and `IO_REPARSE_TAG_MOUNT_POINT`; everything else is an ordinary
entry. Use that predicate in both `is_link_metadata` and the walk's directory test
(`scan.rs:90-91`). Add a unit test over the tag decision, factored out of the syscall.

### T-5 — Dead, unsafe-by-default public `link::is_link`, and a stale comment about it (low)

**Code.**

```rust
/// This is the infallible convenience form. Destructive guards use
/// [`try_is_link`] instead, because a metadata error treated as "not a link" is
/// the unsafe direction: the guard would then reach through the path it could
/// not read.
pub fn is_link(path: &Path) -> bool {
    try_is_link(path).unwrap_or(false)
}
```

`grep -n "is_link\("` over `src/` finds only the definition; every guard uses `try_is_link`
(`apply/phases.rs:48,239`, `apply/mod.rs:138`, `apply/copy.rs:159`, `link.rs:169`). It is
still exported: `src/lib.rs:47` has `pub mod link`. The test mirror's comment contradicts that:

```rust
/// True when `path` is itself a link or junction (the test-side mirror of
/// `girsync::link::is_link`, which is not exported from the public surface).
```

**Failure.** A reader of the tests is told the function is unreachable when it is public; a
library consumer can call the one form whose own doc says never to use it in a destructive
guard, and gets `false` on EACCES/EPERM. Neither is a runtime bug today; both are maintenance
hazards in the file that owns every destructive guard.

**Standard violated.** Public surface should be either used or removed; comments must match
the tree.

**Fix.** Delete `is_link` (no callers) and keep `try_is_link`; or make it `pub(crate)` if a
non-destructive caller appears later. Correct `tests/common/mod.rs:166-167` to name
`try_is_link` or drop the parenthetical.

### T-6 — Case-only spellings of one root escape the distinct-sides check (low)

**Code.** The identity is the canonical cache path, with a raw fallback:

```rust
/// A path that does not exist yet cannot collide with anything but an
/// identically-spelled one, so the raw path is a good enough identity.
fn cache_identity(side: &Side) -> PathBuf {
    let raw = side.cache_path();
    raw.canonicalize().unwrap_or(raw)
}
```

The fallback premise is false on a case-insensitive filesystem: `root` and `ROOT` are the same
root, and on a fresh tree neither `<root>/girpr-cache` nor `<ROOT>/girpr-cache` exists, so both
fall back to raw strings that differ. README:913-914 says "Resolution is by *canonical* path,
so two spellings of one target (`F/sub/..` vs `F`) collide too" — true for `..`, false for
case-only spellings when the cache is absent.

**Failure.** Fresh `root` vs `ROOT`, no flag: the check passes, the src side creates
`root/girpr-cache`, the dst open hits the same file and the run dies with

```
open redb cache E:\...\C\ROOT\girpr-cache (corrupt? use --ignore-cache):
Database already open. Cannot acquire lock.
```

exit 3. The appended advice is wrong (nothing is corrupt) and harmful: with a fresh pair and
`--ignore-cache`, `open_db` (`src/cache/open.rs:68-73`) removes the cache path the src side
just created, so both sides run against different file objects for one directory and the run
exits 0 with `copy=0 delete_files=0` — the meaningless self-pair the guard exists to refuse.
Once any cache file exists, a retry reports the correct "src and dst resolve to the same
cache" message, so on a fresh tree only the first attempt produces the misleading one.

**Standard violated.** `ensure_distinct_sides`'s documented purpose ("the same folder on both
sides … collide") and README:913-914; an error's remediation advice must not remove the
guard's own precondition.

**Fix.** Canonicalize the *root* (which exists) and join the cache file name, rather than
canonicalizing a file that may not exist; compare case-insensitively on Windows (the same
helper as T-1). Do not append "corrupt? use --ignore-cache" to an `AlreadyOpen`/lock error
(`src/cache/db/cachedb.rs:117-122` is too broad). Add a binary test: `root` vs `ROOT` on a
fresh tree exits 3 with the same-cache message.

## 3. Re-checked in the current tree, no finding

- F-1/F-2 fixes are present: `copy_one` unlinks a dst directory link at a file path before
  `File::create` (`copy.rs:158-164`), the planner remaps extras under a dst link to the
  topmost link (`plan.rs:112-126`), and `mkdirs`, `fix_dirs`, `copy_one`, the delete pass and
  `rename_to_src_casing` re-check ancestors on disk (`apply/mod.rs:225-233`,
  `apply/phases.rs:22-84`, `plan.rs:316-353`). No counterexample found in this pass.
- N-1…N-8: the guards the §8 table cites are still at their call sites (shared-link refusal
  before apply, subtree-aware exclusion at `filter.rs:29-67` with ancestor retention in
  `phase_a.rs:86-111`, cache prune in step with dst, reparse check gated on `is_dir()` at
  `scan.rs:90-91`). This is a presence check of the cited code, not a fresh audit of every
  wording in §8.
- Probe: `std::fs::remove_dir_all` on a genuine directory containing a junction removed only
  the junction; the external target file survived and the run exited 0.
- Probe: `root` vs `root\.` is still rejected with `src and dst resolve to the same cache
  …/girpr-cache` (exit 3).

## 4. Verification

Run against `e8585fa`; no source file changed during the pass (this note is the only addition):

```
$ cargo build -p girsync
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.31s

$ cargo test -p girsync
    351 passed; 0 failed; 1 ignored
    (the ignore is pre-existing in tests/sync.rs: 16 passed, 1 ignored there)

$ cargo test -p girsync --test symlink -- --nocapture
    ok. 17 passed; 0 failed; 0 ignored
    4 skip lines: "this platform refused a file link: A required privilege is not held
    by the client. (os error 1314)" (walk_follows_a_file_link,
    walk_aborts_on_a_dangling_link, sync_materializes_a_src_file_link,
    sync_writes_through_a_dst_file_link; junction cases ran)

$ cargo clippy -p girsync --all-targets
    Finished `dev` profile ... (no warnings)

$ cargo fmt -p girsync --check
    exit 0
```

Binary probes (debug binary), scratch root `E:\Symlinks\Temp\rev3`,
`GIRSYNC_TEST_CACHE=memory`, binary `target\debug\girsync.exe`:

```
# T-1, dst inside src
sync --src ...\A\root --dst ...\A\root\sub
    COPY sub/y.txt because=missing
    COPY y.txt because=changed
    SUMMARY ... copied=2 ... dry_run=false
    exit=0
    after: root/sub/y.txt = SRC-Y (was SRC-SUB-Y); root/sub/sub/y.txt = SRC-SUB-Y

# T-1, src inside dst
sync --src ...\B\root\sub --dst ...\B\root
    DELETE sub/x.txt
    DELETE sub/y.txt
    exit=3
    after: root/sub contains no source files

# T-6, fresh case-only pair, then the advised flag
sync --src ...\C\root --dst ...\C\ROOT
    fatal error="open redb cache ...\C\ROOT\girpr-cache (corrupt? use
    --ignore-cache): Database already open. Cannot acquire lock."
    exit=3
sync --ignore-cache --src ...\D2\root --dst ...\D2\ROOT
    SUMMARY ... copied=0 deleted=0 rmdir=0 ... dry_run=false
    exit=0
```

## 5. Not verified

- T-2: no flush-failure injection (a full disk, quota, or filter driver would be needed); the
  finding is the direct consequence of `sync_all().ok()` plus the documented verify contract.
- T-3: this host's filesystem is case-insensitive and `fsutil file setCaseSensitiveInfo
  E:\Symlinks\Temp\rev3\cs enable` returned `Access is denied`; the ordering failure is
  derived from the sort and the two `rename` calls, not observed.
- T-4: no non-link reparse point was available (no OneDrive install; setting a tag needs
  elevation), so only the tag semantics are cited, not a failing run.
- Windows file symlinks: privilege `os error 1314`; 4 of the 17 `symlink` tests skip here.
  The `#[cfg(unix)]` branches of `link.rs`/`copy.rs` are not compiled on this host at all, so
  Unix link behavior is entirely unreviewed at runtime.
- mtime-restore failure during a copy (`copy.rs:200-204`) and the discarded cache tail on a
  mid-copy abort (review F-6/F-7): no injector; these remain unexercised.
- No concurrency or large-tree stress was run beyond the case-only pair; no Unix execution.
