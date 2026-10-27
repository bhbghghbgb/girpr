# Reviewer's notes — girsync F-1…F-7 implementation

**Project:** `girsync` (`tools/sync`)  
**Branch:** `fix-gaps/girsync`  
**Reviewed tip:** `6d0e118`  
**Baseline:** `64dc69d`  
**Status:** Reviewer's notes (read-only audit; no tracked source files changed; committed later with the branch-reference correction)

---

# Adversarial review — girsync F-1…F-7 implementation

**Branch/reviewed tip:** `fix-gaps/girsync` @ `6d0e118`  
**Implementation range:** `64dc69d..HEAD`, scoped to `tools/sync`  
**Committed:** with the branch-reference correction on this branch (the reviewed tip above predates this commit).  
**Method:** read the review, README, tests and post-fix source; inspect the full diff; run the suite and focused tests; reproduce link scenarios with `target\debug\girsync.exe` in `E:\Symlinks\Temp\audit2`; probe junction semantics with `rustc 1.98.1`. No tracked files were modified (`git status --porcelain` empty).

**Short answer:** F-2, F-4, F-5 and F-7 are genuinely fixed; F-6 is documented as scoped; F-3 has tests but four of ten are vacuous on this host. **F-1 is only partially fixed.** The literal “delete an extra file under a dst-only link” shape is closed, but the same reach-through remains in `fix_dirs` and `copy_one` when the link is **shared** by src and dst. I reproduced external data loss with exit 0. The branch should not land as “F-1 fixed, covered”.

---

## 1. Verdict table

| ID | Verdict | Evidence |
|---|---|---|
| **F-1** | **Partially fixed** | `delete_extras`/`rmdir` guards work for dst-only links, but `fix_dirs` and `copy_one` still delete through a shared dst link. External file/subtree loss confirmed, exit 0. |
| **F-2** | **Fixed** | `copy_one` unlinks a directory link at a file path via `link::remove`; the regression test passes. Pre-fix failure mode is consistent with the old guard and a junction `File::create` probe (`os error 5`). |
| **F-3** | **Fixed, host-limited** | 10 symlink tests added; 4 file-link tests return early with `skip: ... os error 1314` and still count as passes. Directory-link/junction cases run. |
| **F-4** | **Fixed** | End-to-end depth-11 test passes: exit 0, deep file absent, shallow file copied. |
| **F-5** | **Fixed** | Binary-level test passes: exit 4, `case_mismatch=1`, `total_diff=0`. |
| **F-6** | **Fixed (documented)** | README “No resume” and `copy_one` comment document the mtime-abort behavior; no test, which the review allowed. |
| **F-7** | **Fixed** | `GIRSYNC_TEST_FAIL_COPY` + `GIRSYNC_TEST_COMMIT_OPS=2` test passes; committed prefix retained, tail absent, rerun converges. |

---

## 2. Per-finding analysis

### F-1 — Partially fixed

**What the fix does correctly.** `build_plan` maps an extra path under a dst link to the topmost link (`tools/sync/src/commands/sync/plan.rs:189-193`). `delete_extras` re-checks the disk and removes the link instead of the descendant (`tools/sync/src/commands/sync/apply/phases.rs:29-49`). `remove_unknown_dirs` does the same for `rmdir` (`phases.rs:219-241`). The two dst-only-link tests and the `--keep-extra` test pass. The original F-1 shape is closed.

**What it does not close.** The guard is only in the delete paths. `fix_dirs` and `copy_one` still resolve intermediate link components:

- `fix_dirs` joins the relative path and calls `p.is_file()` / `p.is_dir()` before `remove_file` / `remove_dir_all` (`tools/sync/src/commands/sync/apply/mod.rs:131-138`). `Path::is_file` follows a junction in an **intermediate** component, so `dst/link/x` is the target’s `x`.
- `copy_one` checks `d.is_dir()` and only tests `is_link(&d)` on the **final** component (`tools/sync/src/commands/sync/apply/copy.rs:145-150`). A real directory inside the link target passes `is_link` as false and is removed recursively.

This matters when the link is **shared**: src also has `link`, so the planner deliberately leaves the link in place (`plan.rs:189-193`) and resolves type conflicts under it toward src. The planner will not delete the extra descendant under the shared link, but `copy_one` will delete it indirectly.

**Reproduction A — `fix_dirs` destroys an external file.**

Fixture: `src/link/x/y.txt`; `dst/link` is a junction to `outside`; `outside/x` is a file containing `EXTERNAL-DATA`.

```
{"event":"type-conflict","path":"link/x"}
{"event":"fix-dir","path":"link/x"}
{"because":"missing","event":"copy","path":"link/x/y.txt"}
{"copied":1,"deleted":0,...,"event":"summary",...}
exitA=0
outside_x_is_dir=True
outside_x_content=y.txt
external_marker=SRC
```

`outside/x` was an external file; it is now a directory containing src’s `y.txt`. The original bytes are gone. The summary says `deleted=0`; the loss is reported only as `fix-dir`.

**Reproduction B — `copy_one` recursively deletes an external subtree.**

Fixture: `src/link/x` is a file; `dst/link` is a junction to `outside`; `outside/x/y.txt` is an external file.

```
{"event":"extra","path":"link/x/y.txt"}
{"event":"type-conflict","path":"link/x"}
{"because":"type-conflict","event":"copy","path":"link/x"}
{"copied":1,"deleted":0,...,"event":"summary",...}
exitB=0
outside_x_is_dir=False
outside_x_content=SRC-FILE
outside_y_exists=False
```

The planner refused to delete the extra `link/x/y.txt` because it is under a shared link, but `copy_one`’s `remove_dir_all(dst/link/x)` deleted `outside/x/y.txt` anyway. Again exit 0, `deleted=0`.

**Verdict: Partially fixed.** The literal delete-extras bug is fixed; the class “a destructive operation must not traverse a link ancestor” is not. The README sentence “paths under a `dst` link are not deleted through it” (`tools/sync/README.md:1011-1014`) is therefore false for type conflicts and for the rename pass.

---

### F-2 — Fixed

The old guard `d.is_dir() && !d.is_symlink()` was false for a directory link, so `File::create` failed. The new code uses `is_link(&d)` and `link::remove(&d)` (`copy.rs:145-150`), which calls `remove_dir` on Windows for a directory target and `remove_file` on Unix (`tools/sync/src/link.rs:78-101`).

The regression test `sync_replaces_a_dst_directory_link_at_a_file_path` (`tests/symlink.rs:223`) passes with a junction. The pre-fix failure mode is consistent with a probe: `File::create` on a junction fails with `Access is denied. (os error 5)`; the old guard skipped the unlink, so the copy hit that error. The fix is correct.

**Residual:** a **file** link at the final path is still followed and written through (`copy.rs:161-164`). That is intentional, but the comment says “the README documents write-through” — the README does not contain that statement. See N-7.

**Verdict: Fixed.**

---

### F-3 — Fixed, but coverage is host-limited

Ten tests exist in `tools/sync/tests/symlink.rs`:

- walk: file link stats, directory link subtree, dangling link aborts;
- sync: src file link, src directory link, dst file-link write-through, F-2, F-1 inside-dst, F-1 external, `--keep-extra`.

The suite result is `10 passed`, but four tests return early:

```
skip: this platform refused a file link: A required privilege is not held by the client. (os error 1314)
```

Those are `walk_follows_a_file_link`, `walk_aborts_on_a_dangling_link`, `sync_materializes_a_src_file_link`, and `sync_writes_through_a_dst_file_link`. They still count as passes. On this host, file-link materialization and write-through are therefore **not actually exercised**; only the directory-link/junction cases are.

There is no test for a **shared** link (src and dst both have the path), no test for `fix_dirs`/`copy_one` through a shared link, no cache-consistency assertion after link removal, no `--exclude link` test, and no rename-through-link test. Those are exactly the gaps that hide N-1, N-2 and N-5.

**Verdict: Fixed for the original “no tests at all” gap; coverage is partial and the green result overstates it.**

---

### F-4 — Fixed

`tests/sync.rs:706` (`run_sync_max_depth_skips_deeper_files`) uses a depth-11 file with the default cap of 10, asserts exit 0, deep file absent, shallow file copied. It passes:

```
test run_sync_max_depth_skips_deeper_files ... ok
test result: ok. 1 passed; 0 failed; ...
```

The test pins the current “silent skip” behavior rather than making it safer. That is consistent with the finding being a test gap, not a behavior bug, but the README’s depth warning remains important. Minor gap: no file exactly at depth 10 and no follow-up `compare` asserting `MISSING`.

**Verdict: Fixed.**

---

### F-5 — Fixed

`tests/compare.rs:647` (`a_case_only_pair_exits_four_with_total_diff_zero`) runs the binary and parses the JSON summary. It passes:

```
test a_case_only_pair_exits_four_with_total_diff_zero ... ok
test result: ok. 1 passed; 0 failed; ...
```

It asserts exit 4, `case_mismatch=1`, `total_diff=0`. Default `case_sensitive=false` is portable across platforms, so the test is not Windows-only.

**Verdict: Fixed.**

---

### F-6 — Fixed (documented)

The README now says a failed mtime restore aborts the run although the bytes are correct, and a rerun recopies (`tools/sync/README.md:736-740`). `copy_one` carries the same point at the call site (`copy.rs:180-186`). No behavior change, no test. The review explicitly made the test optional; documentation was the requested fix.

**Verdict: Fixed (documented).**

---

### F-7 — Fixed

The test-only hook is in `copy.rs:69-80` and fails before opening dst (`copy.rs:133`). The test at `tests/sync.rs:744` uses `GIRSYNC_TEST_FAIL_COPY=f2.bin` and `GIRSYNC_TEST_COMMIT_OPS=2`, with four copies. It asserts exactly two committed rows, the failing path uncached, and a rerun converges. It passes:

```
test a_failed_copy_discards_only_the_uncommitted_cache_tail ... ok
test result: ok. 1 passed; 0 failed; ...
```

The hook is deterministic under the subprocess-per-test harness; the `OnceLock` env read is fine because each test run is a fresh process. README’s “No resume” paragraph now states the uncommitted tail is lost.

**Verdict: Fixed.**

---

## 3. Decision review

### What I agree with

- **Carrying link-ness from the walk** is the right shape. `path_is_symlink()` plus the reparse attribute is better than trying to infer link-ness later from the followed `file_type`.
- **Mapping an extra descendant to the topmost link** is correct for deletion. It is cache-only for the plan and belt-and-braces on disk, which is the right split.
- **Platform-correct unlink** in `link::remove` is necessary; `remove_file` on a Windows directory link/junction is not equivalent.
- **Test-only failure injection** for F-7 is appropriately scoped; it does not change production behavior.
- **Documenting F-6 instead of changing behavior** is defensible: the failure is fail-fast, and a portable test would be platform-specific.
- **F-4/F-5 tests** are at the right level: the command/binary, not just a unit predicate.

### What I would change

**1. The shared-link policy is internally inconsistent.** For deletes, a shared link is treated as a boundary: extras under it are left alone. For writes and type conflicts, it is not a boundary: `fix_dirs` and `copy_one` write/delete through it. For renames, it is not a boundary either. A link is either part of the mirrored tree or it is not. The current hybrid gives the worst of both: it preserves the link and its extra descendants for the delete pass, then deletes external data through it during a type-conflict copy, and exits 0.

The review’s §7 open question — “confirm that keeping content is preferred over removing the shared link” — asks the wrong question. Keeping content is fine; **silently not converging and sometimes deleting external content is not**. The viable policies are:

- **Unlink the shared link and materialize a real directory from src.** This requires replanning the descendants as missing after the unlink; the current plan assumes the followed tree is still there. It gives true mirror semantics.
- **Fail fast** with a clear error: “shared link at `link`; refusing to mirror through it; remove it or use `--keep-extra`.” This is simpler and matches the project’s fail-fast values.
- **Leave it and document non-convergence**, but then `sync` must not exit 0 as if it succeeded. A warning plus nonzero exit, or a `SUMMARY` flag, is the minimum.

The current “leave it, exit 0, compare later exits 4” is the weakest option. The same decision also fixes N-4 (extra descendants) and N-5 (rename through the link).

**2. The runtime guard is not centralized, despite the module comment.** `link.rs:9-11` says it is “the only place that decides link-ness and the only place that unlinks one.” In fact:

- `scan.rs:89` calls `is_reparse_dir` directly;
- `fix_dirs` uses raw `p.is_symlink()` (`apply/mod.rs:133`);
- `rename.rs:107-112` uses raw `std::fs::rename`;
- `mkdirs` (`apply/mod.rs:99-113`) has no link check at all;
- `copy_one` checks only the final component.

Either centralize a `assert_no_link_ancestor`/`resolve_link_ancestor` helper and call it from every write/delete/rename path, or soften the claim. The current comment gives a false sense of coverage.

**3. `--exclude` semantics conflict with the runtime guard.** `filter.rs:3-4` says filtered paths are “left untouched in dst.” With `--exclude link`, the exact `link` entry is removed from `dm`, but its followed children (`link/x.txt`) are not, because glob `link` does not match `link/x.txt`. The planner then schedules `link/x.txt`; the runtime guard sees the on-disk link and removes `link`. The dry run says `DELETE link/x.txt`, the real run says `DELETE link`. That is both a filter-contract violation and a dry-run/real divergence. Excluding a directory should exclude its subtree, or the planner/runtime guard must refuse to delete an excluded link. See N-3.

**4. The junction documentation is wrong on this toolchain.** `link.rs:15-17` and `ADVERSARIAL_REVIEW.md:614-615` claim `FileType::is_symlink` is false for a junction. On `rustc 1.98.1`, `symlink_metadata` on a junction reports:

```
is_symlink=true is_dir=false is_file=false attrs=0x410
path_is_symlink=true follow_is_dir=true
```

So `is_reparse_dir`’s `md.file_type().is_dir() && reparse` test is **false** for a junction on this toolchain; `is_link` works only because `path.is_symlink()` is true. If a supported toolchain reports `is_symlink=false` and `is_dir=false`, both helpers miss junctions. The doc should state the actual behavior and the helper should be made robust (check the reparse attribute plus followed `metadata().is_dir()`, rather than requiring `symlink_metadata().is_dir()`). This is a portability risk, not just a comment nit.

**5. The test suite’s green result is not the same as “covered.”** Four of ten symlink tests self-skip and count as passes. That is acceptable as a local fallback, but CI should have a mode that fails when link tests cannot run (`GIRSYNC_REQUIRE_LINK_TESTS=1` or a privileged CI job), or at least report the skip count separately. Otherwise a regression in file-link materialization/write-through is invisible on this host.

**6. F-4’s test pins silent data loss rather than preventing it.** That is what the finding asked for, so I do not count it as a defect in the test. But the README should be explicit that a deep tree can produce a green, incomplete mirror; the current warning is easy to miss.

---

## 4. New / remaining findings

| ID | Severity | Finding |
|---|---|---|
| **N-1** | **High** (data loss; arguably Critical) | Shared dst link + type conflict: `fix_dirs` removes an external file; `copy_one` recursively removes an external subtree; both exit 0. |
| **N-2** | **Medium** | Cache drift after a successful sync: (a) stale descendant rows under a removed link; (b) no row for a directory created by `fix_dirs`; `compare-self` exits 4. |
| **N-3** | **Medium** | `--exclude link` deletes the excluded link; dry run says `DELETE link/x.txt`, real run says `DELETE link`. |
| **N-4** | **Medium** | Shared link + extra descendants: `sync` reports `extra link/y.txt`, deletes nothing, exits 0; `compare` immediately exits 4. |
| **N-5** | **Medium** | Case-only rename through a shared link renames an external target file; exit 0. |
| **N-6** | **Low/Medium** | Scan performance regression: a reparse-point stat per entry, two stats per `is_link`, and ~2·depth stats per guarded deletion. |
| **N-7** | **Low** | Documentation/claim inaccuracies: junction `is_symlink`, README “not deleted through it”, “README documents write-through”, “only place that decides link-ness”, stale test count, §7 “F-1 fixed, covered”. |
| **N-8** | **Low** | Swallowed metadata/removal errors in the new guards; fail-fast is not applied consistently. |

### N-1 — Shared-link type conflict deletes external data (High)

Locations: `tools/sync/src/commands/sync/apply/mod.rs:131-138`; `tools/sync/src/commands/sync/apply/copy.rs:145-150`.

The two reproductions are A and B above. The key conditions:

- src and dst both have `link`, so `plan.rs:189-193` deliberately keeps the link;
- a type conflict exists **under** the link (`link/x` is a file on one side and a dir on the other);
- `fix_dirs` or `copy_one` joins the relpath and lets the OS resolve `link` as an intermediate component.

A fix must cover every write/delete/rename path, not just `delete_extras`:

- `fix_dirs`: use `link::topmost_link_ancestor` / `link::is_link`; unlink the link or bail; never call `remove_file`/`remove_dir_all` through it.
- `copy_one`: check the **ancestor** chain, not only `d`; if a link ancestor exists, apply the shared-link policy.
- `mkdirs`: `create_dir_all(dst.join(r))` can create directories inside an external target when a missing src dir is under a shared/excluded link.
- `rename_to_src_casing`: see N-5.

Minimum acceptable behavior if the shared-link policy is not implemented: **bail before touching the filesystem** with a message naming the link. Exit 0 after external data loss is the one outcome that must not ship.

### N-2 — Cache drift after a successful sync (Medium)

**(a) Stale rows under a removed link.** `predrop_cache_entries` removes only the planned path; when the plan names the topmost link (`link`), rows for `link/x.txt` remain (`apply/mod.rs:174-187`). `prune_dst_cache` builds `present` from all of `dm`, including the descendants, so it keeps them (`phases.rs:306-325`).

Reproduction C:

```
sync:
{"event":"delete","path":"link"}
{"copied":0,"deleted":1,...,"event":"summary",...}
exitC_sync=0
link_exists_after=False

compare-self:
{"event":"missing","path":"link/x.txt"}
{"case_mismatch":0,"changed":0,"event":"summary","extra":0,"missing":1,"total_diff":1,...}
exitC_self=4
```

A successful sync leaves the dst cache claiming a path that no longer exists. The next sync’s phase A would prune it, but `compare-self` is the audit command and should be clean immediately after a successful sync.

Fix: when a link is unlinked, drop every cache row with the `link/` prefix, in both `predrop_cache_entries` and `prune_dst_cache`. Add a test: sync removes a dst-only link, then `compare-self` exits 0.

**(b) `fix_dirs` never writes the new directory row.** `fix_dirs` removes the old row (`mod.rs:126-129`), creates the directory (`mod.rs:138`), and emits `FIX-DIR` — but never calls `w.put(r, &FileRec::dir())`. The new `prune_dst_cache` comment says “`mkdir` ... `copy` ... `fix_dirs` ... All three put a cache row as they go” (`phases.rs:267-272`), but `fix_dirs` does not.

This is not link-specific. Reproduction G: `src/dir/x.txt`, `dst/dir` is a file; sync succeeds, then:

```
compare-self:
{"event":"extra","path":"dir"}
{"case_mismatch":0,"changed":0,"event":"summary","extra":1,"missing":0,"total_diff":1,...}
self_exit=4
```

Reproduction A shows the same for `link/x`: after sync, `compare-self` reports `extra link/x`. The missing row is the directory the sync just created. This bug may predate the branch, but the branch’s new comment asserts the opposite and the F-1 fixture exposes it.

Fix: after `create_dir_all`, write a `FileRec::dir()` row for `r` (batch it if possible). Add a `compare-self` assertion to the type-conflict tests.

### N-3 — `--exclude link` deletes the excluded link; dry run diverges (Medium)

Reproduction D:

```
dry run --exclude link:
{"event":"extra","path":"link/x.txt"}
{"event":"delete","path":"link/x.txt"}
... exit 0; link_exists_after_dry=True

real run --exclude link:
{"event":"extra","path":"link/x.txt"}
{"event":"delete","path":"link"}
... exit 0; link_exists_after_real=False
```

The filter removes the exact `link` entry from `dm`, but `link/x.txt` survives the filter because the glob `link` does not match `link/x.txt`. The planner therefore has no `link` ancestor to find and schedules the descendant. The runtime guard then finds the on-disk link and removes it. The excluded link is deleted, contrary to `filter.rs:3-4`, and the dry run’s record is not what the real run does.

Fix options:

- make an excluded directory exclude its subtree (prune in the walk or filter by prefix);
- or have the planner treat an on-disk link ancestor as a barrier even when the link entry was filtered out, and skip the descendant;
- or, if the semantics are “exclude the entry but still consider its target contents,” never delete the link; report the descendant as extra and accept non-convergence, but make dry-run match.

Add a test asserting both the dry-run and real record name the same path and the excluded link survives.

### N-4 — Shared link extras: green sync, red compare (Medium)

Reproduction E:

```
sync:
{"event":"extra","path":"link/y.txt"}
{"event":"changed","path":"link/x.txt"}
{"because":"changed","event":"copy","path":"link/x.txt"}
{"copied":1,"deleted":0,...,"event":"summary",...}
exitE_sync=0
outside_y_survives=True

compare:
{"event":"extra","path":"link/y.txt"}
{"case_mismatch":0,"changed":0,"event":"summary","extra":1,"missing":0,"total_diff":1,...}
exitE_compare=4
```

This is the documented §7 open decision: a link src also has is left in place, so its extra descendants remain. The problem is the contract: `sync` exits 0, then `compare` exits 4 on the tree the sync just “finished.” A mirroring tool should either converge or say it did not. If the shared-link policy is fail-fast/unlink+materialize (N-1), this disappears. If the policy is “leave it,” `sync` should warn and/or return nonzero, and the README should state that a shared link is not converged.

### N-5 — Rename through a shared link mutates external files (Medium)

Reproduction F:

```
{"event":"rename","from":"link/x.txt","to":"link/X.txt"}
{"copied":0,"deleted":0,...,"renamed":1,"event":"summary",...}
exitF_sync=0
outside_files=X.txt
```

`rename_to_src_casing` calls `std::fs::rename(dst/link/x.txt, dst/link/X.txt)` without a link-ancestor check (`rename.rs:107-112`). The target’s `x.txt` is renamed to `X.txt`. Content is preserved, but an external tree is mutated, and the run exits 0. Guard the rename path or fold it into the shared-link policy.

### N-6 — Scan/deletion performance regression (Low/Medium)

- `scan.rs:89` calls `is_reparse_dir(ent.path())` for **every** entry, including regular files. On Windows that is a `symlink_metadata` per file. The comment at `scan.rs:86-88` says only directories need the extra check, but the code does not gate on `ft.is_dir()`.
- `is_link` (`link.rs:37-38`) is `path.is_symlink() || is_reparse_dir(path)`. For a non-link, that is two `symlink_metadata` calls (one in `is_symlink`, one in `is_reparse_dir`).
- `topmost_link_ancestor` calls `is_link` per component, so a depth-D deletion pays roughly 2·D stats; `link.rs:111` claims “one `symlink_metadata` per component.”

On a large cold scan, the per-file extra stat is the significant one. Fix: gate the reparse check on `ft.is_dir()` (junctions are directories), and consider a single `symlink_metadata` that inspects `file_type` and attributes together. This is not a correctness blocker, but the review’s cost claims should be corrected.

### N-7 — Documentation and claim inaccuracies (Low)

1. **Junction `is_symlink`.** `link.rs:15-17` and `ADVERSARIAL_REVIEW.md:614-615` say it is false. On `rustc 1.98.1` it is true (`is_symlink=true is_dir=false attrs=0x410`). `is_reparse_dir` returns false for junctions on this toolchain; `is_link` works only via `path.is_symlink()`. Correct the comment and make the helper robust.
2. **README “paths under a dst link are not deleted through it”** (`README.md:1011-1014`). False for `fix_dirs`/`copy_one` type conflicts (A/B) and for rename (F). Scope it to the extra-delete path or fix the code.
3. **README “anything under a link src also has is left untouched.”** False for writes/copies/renames (E/F). It is true only for the delete pass.
4. **“README documents write-through”** (`copy.rs:161-163`). The README has no such statement. The symlink bullet says links are followed/materialized; it does not say a dst file link is overwritten through. Add the sentence or remove the claim.
5. **“The only place that decides link-ness and the only place that unlinks one”** (`link.rs:9-11`). False: `scan.rs:89` calls `is_reparse_dir` directly; `fix_dirs` uses raw `p.is_symlink()`; `rename` uses raw `std::fs::rename`; `mkdirs` has no check.
6. **Test count** (`README.md:1060`): 327 functions / 326 run is stale. Actual: 340 test functions, 339 run + 1 ignored.
7. **§7 overclaim** (`ADVERSARIAL_REVIEW.md:581`): “F-1 fixed, covered.” The delete path is covered; the shared-link type-conflict path is not. §7:619-622 acknowledges the shared-link open decision but not the data-loss consequence.
8. **`prune_dst_cache` comment** (`phases.rs:267-272`): “All three put a cache row as they go.” `fix_dirs` does not; see N-2b.
9. **`link.rs:111`**: “One `symlink_metadata` per component.” `is_link` does two per non-link component.

### N-8 — Swallowed errors in the new guards (Low)

- `link.rs:57` `is_reparse_dir` uses `.unwrap_or(false)`: a metadata error is treated as “not a link,” which is the unsafe direction for a destructive guard. Prefer propagating the error, or at least logging it.
- `link.rs:85` `remove` uses `metadata(path).map(|m| m.is_dir()).unwrap_or(false)`: a metadata error falls through to `remove_file`; the `PermissionDenied` fallback handles some cases, but the original error is lost.
- `phases.rs:222-226` and `234-238`: `if let Err(e) = ... && p.exists()` swallows the removal error when the path is gone. If a failed unlink leaves a dangling link that `p.exists()` reports false, the cache row is removed and the link remains. Prefer matching `NotFound` explicitly and propagating other errors.
- `delete_extras`: if `p` is neither link, file, nor dir and `p.exists()` is false, it silently skips; the plan row was already pre-dropped, so cache and disk can diverge. Low, but worth a log.
- `fix_dirs`: `p.is_file() || p.is_symlink()` then `remove_file` does not use `link::remove`; a directory junction at the final path would fail with `Access denied` if reachable via a race. Use the centralized helper.

---

## 5. Test adequacy

**What is genuinely covered**

- F-2’s directory-link-at-file-path regression (junction, external target).
- F-1’s dst-only-link delete cases: inside-dst target and external target; `--keep-extra` spares the link and target.
- Walk: directory link subtree and dangling link abort (dangling test is a file link, so it self-skips on this host).
- F-4 depth-11 end-to-end.
- F-5 binary-level case-only exit code and summary.
- F-7 mid-copy failure/commit-tail behavior.

**What is not covered**

- Shared link + type conflict in either direction (N-1).
- Cache consistency after link removal or `fix_dirs` (N-2).
- `--exclude` of a link and dry-run/real parity (N-3).
- Shared-link extra descendants and the sync/compare contract (N-4).
- Rename through a link (N-5).
- `mkdirs` through a link.
- File-link materialization and write-through on this host (four self-skips).
- Unix link behavior; true `symlink_dir` (junction fallback is used); `remove_dir_all` on a real directory containing a junction (I probed it manually: on rustc 1.98 it removes the junction only and leaves the target intact, but that is not in the suite).

**Green-suite caveat**

`cargo test -p girsync` is green: 339 passed, 0 failed, 1 ignored. But four symlink tests print `skip:` and return early; the harness still reports them as passed. A CI run on a host without the file-link privilege therefore cannot distinguish “10 link tests passed” from “6 link tests passed and 4 were vacuous.” Add a required-link mode or a privileged CI job.

**F-6**

No test, as allowed. The documentation is accurate; the behavior remains fail-fast. If a deterministic failure injection is ever added, it should assert exit 3 and that a rerun recopies.

---

## 6. Commands run and results

```
git rev-parse --abbrev-ref HEAD        # fix-gaps/girsync
git log -1 --oneline                   # at review time: 6d0e118 docs(review): record the implementation for F-1 through F-7
git status --porcelain                 # (empty at review time; no tracked changes)

cargo test -p girsync                  # 339 passed, 0 failed, 1 ignored
cargo clippy -p girsync --all-targets  # Finished; no warnings
cargo fmt -p girsync --check           # exit 0

cargo test -p girsync --test symlink -- --nocapture
# 10 passed; 4 "skip: ... os error 1314" lines:
# walk_follows_a_file_link, walk_aborts_on_a_dangling_link,
# sync_materializes_a_src_file_link, sync_writes_through_a_dst_file_link

cargo test -p girsync --test sync run_sync_max_depth_skips_deeper_files
# 1 passed
cargo test -p girsync --test sync a_failed_copy_discards_only_the_uncommitted_cache_tail
# 1 passed
cargo test -p girsync --test compare a_case_only_pair_exits_four_with_total_diff_zero
# 1 passed

rustc --version                        # rustc 1.98.1 (48a229cea 2026-09-01)
```

**Junction probe (standalone rustc):**

```
is_symlink=true is_dir=false is_file=false attrs=0x410
path_is_symlink=true follow_is_dir=true
```

**`remove_dir_all` probe on a parent containing a junction:**

```
remove_dir_all=ok
root_exists=false target_exists=true keep_exists=true
```

That probe reduces the risk of the `p.is_dir() → remove_dir_all` branches on this toolchain, but it does **not** cover the direct `dst/link/x` paths in A/B, where the OS resolves the intermediate junction before the removal call.

**Binary reproductions** (all under `E:\Symlinks\Temp\audit2`, `girsync sync ... --output json --no-log-file --log-level error`):

- A: `fix_dirs` external file loss; exit 0; `outside/x` becomes a dir with src’s `y.txt`; original `EXTERNAL-DATA` gone.
- B: `copy_one` external subtree loss; exit 0; `outside/x` becomes src’s file; `outside/x/y.txt` gone.
- C: link removed; `compare-self` exits 4 with `missing link/x.txt`.
- D: dry run `DELETE link/x.txt`; real run `DELETE link`; excluded link removed; external target survives.
- E: sync `extra link/y.txt`, `deleted=0`, exit 0; compare `extra link/y.txt`, exit 4.
- F: rename `link/x.txt → link/X.txt`; external `outside/X.txt`; exit 0.
- G: non-link type conflict `src/dir/x.txt` vs `dst/dir` file; sync exit 0; `compare-self` exits 4 with `extra dir`.

---

## 7. Overall verdict

# **Needs rework**

The branch fixes the literal F-1 delete path, F-2, F-4, F-5 and F-7, and documents F-6. It is not shippable as “F-1 fixed, covered” because a shared dst link plus a type conflict still deletes external data and exits 0, a successful sync can leave `compare-self` dirty, and `--exclude` can delete the excluded link. These are not theoretical: each was reproduced with the built binary.

**Minimum blocking items**

1. **Close the shared-link write/delete path (N-1).** Guard `fix_dirs` (`apply/mod.rs:131-138`), `copy_one` (`copy.rs:145-150`), `mkdirs` (`apply/mod.rs:99-113`) and `rename` (`rename.rs:107-112`) against link ancestors. Either unlink the shared link and materialize a real directory (with replanning), or fail fast before touching the filesystem. Add tests for shared link + type conflict in both directions (A/B) and rename (F).
2. **Make the cache consistent after a successful sync (N-2).** Write the `FileRec::dir()` row for `fix_dirs`, and prune cache rows under every removed link. Add `compare-self` assertions to the link/type-conflict tests: a successful sync must be clean.
3. **Fix the `--exclude`/dry-run divergence (N-3).** An excluded link must not be deleted, and the dry-run record must match the real record. Add a test.
4. **Correct the claims before declaring the finding fixed (N-7).** README “paths under a dst link are not deleted through it” and §7 “F-1 fixed, covered” are false as written; the junction comment is wrong on the supported toolchain; the test count is stale. Documentation is low severity, but this branch is a review artifact, and a false “fixed, covered” claim is itself a defect.

**Follow-ups after the blockers**

- Decide the shared-link policy for extras (N-4): converge or fail/warn; do not leave `sync` exit 0 with `compare` exit 4.
- Add a required-link CI mode so the four file-link self-skips cannot masquerade as coverage.
- Fix the scan/deletion stat regression (N-6).
- Propagate or explicitly classify the swallowed errors in the new guards (N-8).

If the maintainers deliberately choose “shared links are unsupported, fail fast,” that single decision collapses N-1, N-4 and N-5 into one small, testable change and is the shortest path to a defensible release. If they instead choose “shared links are part of the mirror,” then writes, deletes, renames and cache pruning must all use that same policy — the current hybrid is the one option that cannot be defended.
