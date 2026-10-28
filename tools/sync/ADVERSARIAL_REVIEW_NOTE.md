# Eighth adversarial pass — `girsync` (`tools/sync`)

- **Branch:** `fix-gaps/girsync`
- **Reviewed tree:** `374453d` (clean before this note was added); every line cite below is this tree.
- **Mode:** read-only review. No source, test, README, `DESIGN-NOTES.md`, Cargo or `ADVERSARIAL_REVIEW.md` file was changed; the only addition is this note.
- **Result:** **not sound.** Two Medium defects (a POSIX-only refusal of every file-over-directory type conflict, and dst file-link targets that the plan does not see), plus four Low gaps.

## Verdict

| ID | Severity | Class | Where | Finding |
| --- | --- | --- | --- | --- |
| P8-1 | Medium | bug — over-refusal | `src/commands/sync/plan.rs:458-483` | The hard-link preflight stats every copy destination without checking `dm`'s kind. On Unix a directory's `nlink` is at least 2, so every src-file-over-dst-directory conflict (and every dst directory link at a file path) is refused with a bogus "has N hard links" message in both modes. |
| P8-2 | Medium | bug — plan/apply inconsistency, non-convergence | `src/commands/sync/plan.rs:317-350,433-485`, `src/commands/sync/apply/copy.rs:205-234,276-279`, `src/commands/sync/apply/mod.rs:90-97` | A dst file link whose target is another path inside dst is compared and written through as ordinary content, but the delete plan does not know the target. A dst-only target is removed before the write-through: changed content exits 3 after the delete (the repo's own write-through test would fail), equal content exits 0 with a dangling link. A target src also mirrors makes the copy change that path, so the two names ping-pong or race. |
| P8-3 | Low | bug — dry-run parity (POSIX) | `src/commands/sync/plan.rs:332,459`; `src/commands/sync/rename.rs:272-276` | Both plan-time guards stat the **post-rename** spelling. A dry run does not rename, so on a case-sensitive filesystem a case-only dst link or hard link is not seen and the dry run exits 0; the real run renames first and then refuses with exit 3. |
| P8-4 | Low | bug — regression (POSIX) | `src/commands/sync/rename.rs:174-181` | The case-fix temporary appends `.girsync_case_tmp.<n>` to the full name. A name within ~18 bytes of `NAME_MAX` now fails `ENAMETOOLONG` for every candidate, so a valid case-only rename exits 3 (possibly after earlier renames in the same pass). |
| P8-5 | Low | bug — swallowed error / residue | `src/commands/sync/rename.rs:297` | `let _ = std::fs::remove_file(&tmp);` drops the placeholder-cleanup failure after a failed step 1, leaving `.girsync_case_tmp.<n>` in dst with no mention of why. |
| P8-6 | Low | test gap | `src/commands/sync/rename.rs:277,292-318`; `tests/sync.rs:656-695` | The failure hook is checked before the temporary is claimed, so the only failed-rename test reaches neither the placeholder-cleanup path nor the step-2 rollback path. |

Severity key: **High** = destroys data outside the requested change or corrupts the source; **Medium** = destroys dst data, or silently reports success over a non-mirrored tree, or blocks the run under a specific shape; **Low** = wrong report, over-refusal, residue, or preview/outcome divergence without data damage.

## P8-1 — every file-over-directory type conflict is refused on Unix

**Where.** `ensure_hard_linked_targets_safe`, `src/commands/sync/plan.rs:458-483`, especially:

```rust
for item in &plan.copy {
    let d = dst.join(&item.rel);
    let info = match crate::hardlink::info(&d) { ... };
    if info.links <= 1 { continue; }
    ...
    bail!("refusing to copy `{}`: dst `{}` has {} hard links ...", ...);
}
```

The credit loop immediately above (`plan.rs:441-447`) filters records it is not sure about (`rec.kind != "file" || rec.is_link`); the copy loop has no such filter.

**Evidence.** `build_plan` puts a copy into `plan.copy` for a type conflict whenever src is a file (`plan.rs:166-174`), including when `dm` records the dst entry as a directory: the walk follows links (`src/link.rs:3-10`), so a dst directory or directory link at a file path is `kind == "dir"`. `hardlink::info` on Unix is `std::fs::metadata(path)?.nlink()` (`src/hardlink.rs:54-63`), and a directory's `nlink` is `2 + subdirectories`, never 1. The directory is only removed at apply time (`src/commands/sync/apply/copy.rs:242-248`), after this preflight has already run.

**Failure scenario.** `src/node` is a file, `dst/node/inner.txt` exists. The plan holds `node` in `copy` (`TypeConflict`); the preflight stats `dst/node`, reads `nlink == 2`, and refuses both `--dry-run` and the real run with the hard-link message. The same happens for a dst directory link at a file path (the count is the target directory's). On this Windows host NTFS reports 1 link for a directory, which is the only reason `tests/sync.rs::run_sync_resolves_type_conflicts` (line 504), `tests/sync.rs::a_copy_that_wipes_a_blocking_dir_is_not_also_planned_for_removal` (line 256) and `tests/symlink.rs::sync_replaces_a_dst_directory_link_at_a_file_path` (line 231) pass: each fixture hits exactly this path. They should fail on Linux.

**Why this fails the standard.** The guard's stated question is whether truncating the destination would change another name for the same inode. A directory at a copy destination is removed before the write, so it is not that inode. Refusing the whole shape is an omitted case, and the diagnostic names a hard-link condition that does not exist.

**Suggested fix.** Skip the stat when `dm.get(&item.rel)` is `kind == "dir"`, leaving the `NotFound` branch as is. Do **not** skip `is_link` records here: a file link's target still has a name count worth checking, and P8-2 is the separate symlink-alias question. The existing three tests become the POSIX regression coverage; additionally run the suite on Linux, since this host cannot exercise the failure.

## P8-2 — a dst file link's target is invisible to the delete plan

**Where.** `ensure_link_safe` (`plan.rs:317-350`) checks only strict **ancestors**; `ensure_hard_linked_targets_safe` (`plan.rs:433-485`) counts hard-link names; `copy_one` refuses a file link only when it resolves into or contains src (`apply/copy.rs:205-234`) and otherwise writes through it (`copy.rs:276-279`). Deletes run before copies (`apply/mod.rs:90-97`).

**Evidence.** The walk follows links, so `dst/a -> dst/b` is reported as an ordinary file at `a` with `b`'s content; the diff and plan never learn that the two names are one file. `try_is_link`/`canonicalize` run at apply time, but only against the src boundary, and the hard-link count follows the link, so the symlink itself contributes no second name. `ensure_link_safe` cannot help: the link is the final component, which a copy is explicitly allowed to write through. `dm` does carry `is_link` for `a` (`src/effective/entry.rs:120-127`), but no preflight reads it for this shape.

The repository's own write-through case, `tests/symlink.rs:197-221` (`sync_writes_through_a_dst_file_link`), is exactly this fixture: `src/a.bin`, `dst/a.bin -> dst/target.bin`, `target.bin` dst-only and differing. The documented rule (`README.md:1055-1057`) is that a dst link to a file is followed and the copy lands on the target unless the target resolves into src. `target.bin` is `diff.extra`, so `delete_files` holds it and `delete_extras` removes it before `copy_files` runs; `copy_one` then canonicalizes `a.bin` and gets `NotFound`. The test asserts exit 0 and the target's new content. It would fail where the fixture is not skipped; on this host it prints `skip` and returns.

**Probe (hard-link stand-in).** File symlinks are refused on this host, so the same path shape was built with `dst/a.bin` a hard link to `dst/target.bin` and a differing `src/a.bin`. `sync --dry-run --output json` printed `{"event":"delete","path":"target.bin"}`, then `{"event":"copy","path":"a.bin"}`, and `"copied":1,"deleted":1`. The plan therefore holds both actions for this shape, and `Applier::apply` runs `delete_extras` before `copy_files` (`apply/mod.rs:93-97`). For a symlink at `a.bin`, that delete is what turns the planned copy into a dangling-link error, or leaves the link dangling when no copy is planned.

**Failure scenarios.**

1. **The documented write-through shape.** `src/a.bin` differs from `dst/a.bin -> dst/target.bin` (dst-only). The plan holds `a.bin` in `copy` and `target.bin` in `delete_files`. `delete_extras` removes the target (`apply/phases.rs:24-72`), then `copy_one`'s `canonicalize(dst/a.bin)` (`copy.rs:213-216`) returns `NotFound`: exit 3 after the delete. With equal content, no copy is planned, so the run deletes the target and exits 0 with `a.bin` dangling and the tree no longer mirroring src; the next run aborts in the walk (`scan.rs:51`), so recovery is manual.
2. **The target is also in src.** `dst/a -> dst/b`, src has both with different content. The run copies `a` through the link, which changes `b`; `b` now differs from src, so the next run copies `b`, which changes `a`. Each run exits 0 and reports success; the pair never converges. If both are in `plan.copy` (both differ from dst), `copy_files` runs them in parallel (`apply/phases.rs:116-170`) against the same inode: interleaving either fails verify after corrupting the content (exit 3) or exits 0 with one path holding the other's bytes.

**Why this fails the standard.** The plan and the copy phase disagree about whether the link's target will exist: the delete phase treats it as an extra, the copy phase needs it. The documented write-through cannot complete, and the run either aborts after removing data or exits 0 leaving a dangling link. When the target is itself a mirrored path, the copy changes a path it did not name, which is the hard-link hazard through a different directory entry, and the two names cannot both match src.

**Suggested fix.** Resolve each file link's target at plan time (or record it during the walk). If a planned copy writes through a link whose target is in `delete_files`, either spare the target (then the write-through lands in it, as the documented case expects) or unlink and materialize the link; never delete a target the copy needs, and never leave a link whose target the plan removes. If the target is itself a mirrored path (`sm` contains it), refuse the plan or materialize the link instead of writing through, because the write changes that path and the two names can only ping-pong. Needs tests for the equal-content and mirrored-target shapes; the existing file-link cases must run on a POSIX host before this behaviour can be called covered.

## P8-3 — the plan-time guards ask about the post-rename path, which a dry run never created

**Where.** `ensure_link_safe` (`plan.rs:332` → `link::link_ancestor_above(dst, rel)`) and the copy loop of `ensure_hard_linked_targets_safe` (`plan.rs:459-460` → `hardlink::info(&dst.join(rel))`) both stat the src-cased `rel`. The rename pass re-keys `dm` in both modes but performs the filesystem rename only in a real run (`rename.rs:272-276`); the guards run after it (`mod.rs:298-302,346,356`).

**Evidence.** On a case-sensitive filesystem a case-only dst entry still has its old spelling, so the guard's stat is `NotFound`: `try_is_link` maps `NotFound` to "not a link" (`link.rs:53-58`) and `topmost_link_ancestor` walks components with it (`link.rs:158-172`), while the hard-link loop treats `NotFound` as "nothing to truncate" (`plan.rs:462`). After the real run's rename the same stat succeeds and the guards refuse. This contradicts `README.md:764-765` ("The decision is made at plan time in both modes"), `README.md:1076-1078`, `DESIGN-NOTES.md:214-216` and the P7-4 closure at `ADVERSARIAL_REVIEW.md:689`.

**Failure scenario.** Linux, default case-insensitive pairing: `dst/L` is a directory link (or a hard-linked file) and src has `l` with differing content. The dry run prints `RENAME L -> l` and `COPY l/...`, exit 0. The real run renames `L -> l`, then `ensure_link_safe` sees the link (or the hard-link preflight sees `links > 1`) and exits 3 — after a filesystem change, for a tree the dry run said was applicable. (Equal content plans no action under the link, so both modes then exit 0 and the divergence is not observable.)

**Why this fails the standard.** The documented dry-run answer is "the same question, not a smaller one". A refusal that depends on a rename the preview did not perform is a different question, and the real run's refusal follows an avoidable mutation.

**Suggested fix.** Resolve each guard stat through the pre-rename spelling: keep the `(old_rel, new_rel)` pairs from `case_renames` and stat the old path when the rename is only in memory, or run the guards against a small path map that maps post-rename `rel` to the on-disk spelling. Add a POSIX-only test for a case-only link ancestor and a case-only hard link.

## P8-4 — the case-fix temporary name can exceed `NAME_MAX`

**Where.** `claim_case_tmp`, `rename.rs:174-181`: `cand_name = file_name + ".girsync_case_tmp.{n}"` (18 bytes plus digits), then `create_new`/`create_dir`.

**Evidence.** Every candidate exceeds the filesystem's `NAME_MAX` (255 on ext4), so `symlink_metadata` at `rename.rs:185` (or the claim that follows) returns `ENAMETOOLONG` on the first `n`; there is no shorter fallback, and the pass exits 3. Before P7-1, the temporary was `from.with_extension("girsync_case_tmp")`, which **replaced** a dotted extension (so dotted names did not grow).

**Failure scenario.** A dst file or directory whose name is 238 bytes differs only in case from src; case-insensitive mode on POSIX. `claim_case_tmp` cannot create any candidate, so the pass exits 3, although the two renames themselves would have been legal. If the pass already renamed earlier entries, dst is left partly renamed with no rollback for those.

**Why this fails the standard.** A case-only alignment must work for every name the walk accepts; a suffix chosen for uniqueness must not consume headroom the user's name needs.

**Suggested fix.** Bound the candidate name: truncate the stem so the total stays under `NAME_MAX` (keeping the suffix and counter), or use a fixed-length temporary name plus the counter in a way that cannot collide with the original. Keep the claim (`create_new`/`create_dir`) semantics.

## P8-5 — a failed placeholder cleanup is discarded

**Where.** `rename.rs:293-301`:

```rust
if let Err(step1) = std::fs::rename(&from, &tmp) {
    if !step.is_dir {
        let _ = std::fs::remove_file(&tmp);
    }
    return Err(step1).with_context(...);
}
```

**Evidence.** The file placeholder was created by `claim_case_tmp` and step 1 did not replace it, so it is the tool's own empty file inside dst. If `remove_file` fails (permissions, an antivirus or indexer holding it on Windows), the failure is dropped, the run reports only the step-1 error, and `.girsync_case_tmp.<n>` remains in the user's tree. The step-2 rollback branch (`rename.rs:302-318`) already reports its secondary failure beside the primary one; this branch does not.

**Why this fails the standard.** No fallible result is discarded, and a rollback/cleanup error is part of the failure the user needs to see. The residue is also tool output left in the user's tree, where the next run's walk reads it as ordinary dst content.

**Suggested fix.** Attach the cleanup error to the primary one, as the rollback branch does:

```rust
if let Err(cleanup) = std::fs::remove_file(&tmp) {
    return Err(step1).with_context(|| format!(
        "rename ... (case fix step1); removing the placeholder {} also failed: {cleanup}",
        tmp.display()
    ));
}
```

(or `context` on the cleanup then `context` again), and test it with the hook described in P8-6.

## P8-6 — the failed-rename test never reaches the cleanup or rollback code

**Where.** `test_fail_rename` is checked at `rename.rs:277`, before `claim_case_tmp` at `292`; the only test using it is `tests/sync.rs:656-695` (`a_failed_case_rename_leaves_no_record`).

**Evidence.** `GIRSYNC_TEST_FAIL_RENAME` bails before any filesystem call, so the test asserts only the "no record, dst untouched" contract. The placeholder-cleanup branch (`293-301`) and the step-2 rollback branch (`302-318`) are never executed by any test; P8-5's dropped error is therefore invisible to the suite, and a regression in the rollback could pass CI.

**Why this fails the standard.** A failure-injection hook exists to exercise the failure handling the code claims; the interesting failure points are after the claim (during step 1) and after step 1 (step 2), not before both.

**Suggested fix.** Add a hook checked after `claim_case_tmp` but before step 1 (forces cleanup), and a step-2-only hook (forces rollback). Assert `dst` is byte-identical to its pre-run state, no `.girsync_case_tmp.*` remains, no `RENAME` record was emitted, and the exit code is 3 with the primary error named.

## Verification

Commands run in this tree on this Windows host; the test/clippy/fmt output is captured in `E:\Symlinks\Temp\opencode\girsync_verify8.txt` (the build was re-run while writing this note).

| Command | Result |
| --- | --- |
| `cargo build -p girsync` | exit 0, `Finished dev profile ... in 0.39s` |
| `cargo test -p girsync` | **396** functions: **395 passed**, 0 failed, **1 ignored** (`tests/sync.rs:962`, pre-existing) |
| `cargo test -p girsync --test symlink -- --nocapture` | **24 passed**, 0 failed; 5 file-link cases printed `skip: ... (os error 1314)` and returned early |
| `cargo clippy -p girsync --all-targets` | exit 0, no warnings |
| `cargo fmt -p girsync --check` | exit 0 |

Per-target counts for the full suite: lib 149, cli_dispatch 1, compare 13, compare_self 12, convert 4, hash_mode 18, helpers 12, lazy 27, migration_lock 11, output 16, rw_threads 15, stat_trust 19, symlink 24, sync 27+1 ignored, sync_plan 19, update 3, why 25, plus the empty unit and doc-test targets.

A scratch probe outside the repo (`E:\Symlinks\Temp\opencode\p82probe`) used a hard link as the stand-in for the file link of P8-2; `girsync sync --dry-run --output json` printed `delete target.bin`, `copy a.bin`, `"copied":1,"deleted":1`, exit 0.

A scratch probe (outside the repo) confirmed that `std::fs::remove_dir_all` on a genuine directory containing a junction does not follow the junction on this host; the target outside the tree survived. This closes that listed open item for Windows.

## Not verified

- **P8-1 on POSIX.** No Unix host was available. The failure follows from `nlink >= 2` for directories, but the refusal was not executed; three existing tests should fail on Linux.
- **P8-2's final step.** The plan order (DELETE `target.bin`, COPY `a.bin`, `deleted:1`/`copied:1`) was reproduced with a hard-link stand-in. The subsequent `canonicalize` failure for a dangling symlink is source-derived: creating a file symlink here requires a privilege the host does not grant (`os error 1314`), so `sync_writes_through_a_dst_file_link` and the four sibling file-link cases did not execute their bodies. The write-through test is exactly the delete-before-copy shape and is the strongest unexecuted check in this pass: it should fail on a host that can create the link.
- **P8-3 on POSIX.** The divergence needs a case-sensitive filesystem and the default case-insensitive pairing; not reproducible on this host.
- **P8-4.** A 238-byte name on a 255-byte `NAME_MAX` filesystem was not created; the conclusion is from the candidate construction and the platform's `NAME_MAX` rule.
- **P8-5/P8-6.** No runtime probe of a failing placeholder cleanup or a step-2 rollback; neither is reachable through the current hook.
