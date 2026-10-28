# ADVERSARIAL REVIEW — `girsync` (`tools/sync`)

**Scope and method.** Source-only audit of `tools/sync` (`src/`, `tests/`, `README.md`, `Cargo.lock`), read with `git show`/`git grep`/`git log` and the walkdir 2.5.0 sources; the first pass built and ran nothing.

**Baseline.** `HEAD = 64dc69d` (“revert: drop the adversarial review series (tree back to 7f2d69)”); `64dc69d^{tree}` and `7f2d69c^{tree}` are both `66985753edf618610a3df629bacdf2b6b70609fa`. The uncompacted audit — per-finding essays, the verified-correct section, corrections to the reverted series and the landing order — is in git history: `git show 94f1c3a:tools/sync/ADVERSARIAL_REVIEW.md`.

Open items, trade-offs and unverified areas: [OPEN_FOR_MAINTAINERS.md](OPEN_FOR_MAINTAINERS.md).

## Pass log

Implementation plus the nine review passes after it, one subsection each. Evidence,
failure scenarios and per-pass verification are in the git history:
`git log --oneline main..HEAD` and `git show <commit>`; the hashes below are the
index. The first pass's original text is at
`git show 94f1c3a:tools/sync/ADVERSARIAL_REVIEW.md`.

### First pass — F-1…F-7

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| F-1 | `DELETE` reached through a dst directory link, removing the link target (possibly outside dst, possibly a file src also has); extras under a link now plan the topmost link | fixed | `1c827b2`, `42b9766`, `cca7218`, `9beede2`, `00f3e39`, `dae1a78`, `943e2d5` |
| F-2 | A dst directory link at a src file path was skipped by the clear guard and aborted the run in `File::create`; it is now unlinked with a platform-correct call | fixed | `2bec6b3`, `cca7218` |
| F-3 | No symlink or junction tests at all; 24 link cases now live in `tests/symlink.rs`, directory links falling back to a junction | tests | `1c827b2`, `2bec6b3`, `42b9766`, `f4ffb59`, `943e2d5` |
| F-4 | `--max-depth` had no end-to-end test; a depth-11 file under the default cap now stays unmirrored with exit 0 | tests | `d077a78` |
| F-5 | The case-only exit code was unpinned at command/binary level; a case-only pair now pins exit 4 with `case_mismatch=1`, `total_diff=0` | tests | `ddadcc9` |
| F-6 | A failed mtime restore aborts an otherwise-good copy, undocumented; the copy step and `copy_one` now say so | documented | `d077a78`, `1d723cb` |
| F-7 | The uncommitted cache tail on a mid-copy failure was untested; `GIRSYNC_TEST_FAIL_COPY` plus a low `GIRSYNC_TEST_COMMIT_OPS` pins it | tests | `d077a78` |

**Verified correct (do not re-fix):**
- **Symlink walk.** `walk_live` sets `.follow_links(true)` (`src/scan.rs:36-39`); walkdir 2.5.0 replaces each link with its target (`lib.rs:840-847,961-963`; `dent.rs:230-273`) and `file_type()` returns the target type (`dent.rs:150-160`), so a file link is a file, a directory link a directory, and a dangling link an error (`src/scan.rs:74-95,43`). `ft.is_symlink()` is unreachable (`b68ca17`'s branch was dead code); `path_is_symlink()` is the correct link test. The reverted G-1 premise was false.
- **`--dry-run` writes nothing.** `open_folder_cache` is the only cache open (`src/effective/side.rs:143-187`); dry-run opens read-only (`:185-186`) or in memory (`:181-183`), `--ignore-cache` is treated as absent (`:125-127`), `cmd_sync` returns before apply (`src/commands/sync/mod.rs:330-345`), and `compare-self` never writes (`src/commands/compare_self.rs:102-120`).
- **Exit codes.** A `run` error is exit 3 (`src/main.rs:34-39`), clap parse errors exit 2, and `report_diff` returns 0 or 4 from `is_empty()` (`src/commands/mod.rs:39-44`); `sync` returns `Ok(0)` and never 4 (`src/commands/sync/mod.rs:361-381`), matching `README.md:1007-1012`.
- **Pre-drop commit.** `Applier::apply` runs `mkdirs`, `fix_dirs`, `predrop_cache_entries`, then deletes/copies/`remove_unknown_dirs` (`src/commands/sync/apply/mod.rs:78-90`); `predrop_cache_entries` commits before the first filesystem change (`:174-187`).
- **Coverage check.** `plan_pairs` raises `Coverage` at `src/planner/pairs.rs:367` (message in `src/planner/coverage.rs`) before phase C hashes, so a record that cannot answer never falls back to size+mtime (`pairs.rs:165-226`).
- **Case-only rename before diff.** `cmd_sync` plans (`src/commands/sync/mod.rs:197`), renames dst to src casing (`:276`), then diffs (`:291`), matching `README.md:737-746`.
- **`--keep-extra` spares directories.** `delete_files` (`src/commands/sync/plan.rs:145`) and `rmdir` (`:189-205`) are both gated on `!keep_extra`; `tests/sync.rs:188` covers the directory half.
- **`compare` cache backup.** `open_side` passes `backup_first = true` (`src/effective/side.rs:86-100`); `README.md:435-440,1093` document that a real `compare` writes both caches and that backups accumulate — no defect.
- **`..`/absolute `rel` is not an exposure.** Rels come only from the walk (`src/scan.rs:53-59`, `strip_prefix(root)`) or a cache key; roots are validated (`src/commands/sync/mod.rs:385-395`) and `is_record_path` rejects record inputs (the reverted G-11 negative is confirmed).

**Decisions still in force:**
- A removal never traverses a dst link: an extra under one resolves to the topmost link, whose descendants leave `delete_files`/`rmdir`.
- A directory link at a file path is unlinked; a dst link to a *file* is still followed and written through.
- A failed mtime restore aborts the run although the bytes are durable, and a rerun recopies.
- Cache rows are durable only to the last commit threshold; a mid-copy failure drops the tail and a rerun re-stats.

### Second pass — N-1…N-8

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

### Third pass — T-1…T-6

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

### Fourth pass — F-1 (reused ID)

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| F-1 | A link inside the effective src that reached dst (target inside dst, or containing it) hid the root overlap and let the delete pass eat dst; it is refused after the src walk and before planning | fixed | `8057f9a`, `d3c8f00`, `174ba52`, `49e7d49` |

The fourth pass reused the ID `F-1` for a different finding; the original high finding is the first pass's F-1, still fixed there.

**Decisions still in force:**
- A src link whose effective target is dst, or contains dst, is refused after src phase A; a link elsewhere is ordinary content, even back inside src.
- A `girpr-cache*` component is reserved at any depth, so a followed link cannot mirror a nested root’s live cache or backups.
- A removal whose parent resolves inside src is refused on disk, so the plan is never the last word on what may be deleted.

### Fifth pass — P5-1, P5-2

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| P5-1 | `mkdirs` ran before `fix_dirs`, so a valid plan failed on `create_dir_all`; the order is swapped and a plan-time preflight refuses a path under an unfixed blocking file | fixed | `2e165e6`, `59f1127` |
| P5-2 | A dst file link into src was written through and destroyed the source; dst links into src are refused after dst phase A and `copy_one` re-checks the final component | fixed | `82e5617`, `93ab01c` |

**Decisions still in force:**
- `fix_dirs` runs before `mkdirs`.
- A dst link resolving into src, or containing src, is refused after dst phase A even when the plan would only unlink it; write-through to a dst-internal target stays sanctioned.
- Hard links are invisible to the scanner, so these rules do not see them; P6-2 refuses them by link count instead.

### Sixth pass — P6-1, P6-2, P6-3

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| P6-1 | A filtered dst blocker at an ancestor was invisible to the plan guard; `ensure_mkdir_targets_free` stats the disk before `rename_to_src_casing` and refuses it | fixed | `af5116b`, `08897e4` |
| P6-2 | A dst hard link was written through, truncating the shared inode; `hardlink::count` refuses any destination with more than one link before `File::create` | fixed | `b2b1445`, `2a99fa3` |
| P6-3 | The README test count was stale; it now states 384 test functions, 383 run by default | fixed | `8d52de6` |

**Decisions still in force:**
- Any destination with more than one hard link is refused, blanket rather than limited to links into src; P7-4 credits a same-inode name the plan removes before the copy (seventh pass). P8-1 narrows the blanket rule to regular-file destinations (eighth pass).
- The mkdir preflight and the real run read the same disk state, so a filtered blocker is refused alike in `--dry-run` and before any record.
- `--max-depth` still skips silently; an excluded dst directory at an ancestor still reports `MKDIR` although `create_dir_all` is a no-op.

### Seventh pass — P7-1…P7-4

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

### Eighth pass — P8-1…P8-6

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

### Ninth pass — R9-1…R9-3

All three findings were confirmed against `65c1ce8` and fixed at the root cause;
none was rejected. All three were Medium.

| ID | Finding | Status | Commits |
|----|---------|--------|---------|
| R9-1 | A kept link whose target passed through a dst **directory** link was resolved to the canonical spelling, so the link component the walk named stayed in `delete_files`; every link component, alias spelling and ancestor on the chain is now spared, and resolution errors other than `NotFound` propagate | fixed | `e93b17f` |
| R9-2 | A spared **intermediate** link did not keep its ancestor directories, so the later `rmdir` could remove the directory recursively with the link inside; every kept chain entry now goes through `keep_path`, which spares its ancestors | fixed | `e93b17f` |
| R9-3 | Storage shared by two kept links was judged one link at a time, so copying both wrote two contents through one inode and one mirrored path ended wrong at exit 0; the whole terminal-storage set is grouped and a plan that copies any member is refused before mutation | fixed | `e93b17f`, `d9aa88e` |

R9-1's canonicalization premise was probed outside the repo: with `dst/link` a
junction to `dst/real`, `target_rel` named `real/x` for both `link/x` and
`real/x`, which is why the walk's alias had to be carried separately. The
end-to-end cases need file symlinks and self-skip on this host (`os error 1314`);
the resolver and the full spare pass run against directory junctions, which take
the same `read_link`/`try_is_link` path.

**Decisions still in force:**
- A kept dst file link keeps every load-bearing path on its chain: each link component, both the walk's alias and the canonical spelling, and every ancestor directory. All inserts go through `LinkStorage::keep_path`, so no later `delete_files`/`rmdir` entry can remove them.
- Two or more kept links may resolve to one terminal only when none of them is copied. If any member is in `plan.copy`, the run is refused before the plan print and before any mutation; the chained `a -> b -> c` shape and external terminals are judged the same way.
- A resolution error other than `NotFound` is returned rather than read as “outside dst”: a failed spare must not silently leave the link's storage in the delete plan.

### Latest verification

- `cargo test -p girsync` — **420 passed, 0 failed, 1 ignored** (the pre-existing `tests/sync.rs` case); suite 421 functions, 420 run. Breakdown: lib 162; cli_dispatch 1, compare 13, compare_self 12, convert 4, hash_mode 18, helpers 12, lazy 27, migration_lock 11, output 16, rw_threads 15, stat_trust 19, symlink 32, sync 31 (+1 ignored), sync_plan 19, update 3, why 25; doctests 0.
- `cargo test -p girsync --test symlink -- --nocapture` — 32 passed, 0 failed. Thirteen file-link bodies print `skip: ... (os error 1314)` and return; one case-only body self-skips because this filesystem pairs `CaseProbe` with `caseprobe`.
- `cargo build -p girsync`, `cargo clippy -p girsync --all-targets`, `cargo fmt -p girsync --check` — clean, exit 0, no warnings.

---

*End of review. The first pass was a source-only audit: no project file changed, nothing built, tested or run; its uncompacted text is at `git show 94f1c3a:tools/sync/ADVERSARIAL_REVIEW.md`. The pass log records the implementation and the second through ninth passes, which did build, test and run the binary; per-pass detail is in the commits it cites. Open items are in [OPEN_FOR_MAINTAINERS.md](OPEN_FOR_MAINTAINERS.md).*
