# Seventh adversarial pass — `girsync` (`tools/sync`)

- **Branch:** `fix-gaps/girsync`
- **Reviewed tree:** `81779c5` (`docs(review): compact the intermediate pass history`); the sixth-pass code under review is `af5116b`, `08897e4`, `b2b1445`, `2a99fa3`, documented by `8d52de6`.
- **Mode:** read-only source review plus runtime probes. No source, test, README, `DESIGN-NOTES.md`, Cargo or `ADVERSARIAL_REVIEW.md` file was changed in this pass; the only addition is this note.
- **Result:** **not sound.** One silent data-loss defect in the case-only rename pass (**P7-1**), plus three low-severity gaps: an over-broad link-ancestor refusal (**P7-2**), a `RENAME` record emitted before the rename (**P7-3**), and a dry-run/real-run divergence left by the sixth pass's hard-link refusal (**P7-4**).

## Verdict

| ID | Severity | Class | Where | Finding |
| --- | --- | --- | --- | --- |
| P7-1 | Medium (high impact, low likelihood) | bug — silent data loss | `src/commands/sync/rename.rs:204-208` | The case-only rename uses a fixed sibling path, `<stem>.girsync_case_tmp`, as its intermediate. Any existing dst entry at that path is replaced (file) or makes the pass abort (directory) — even when the entry is excluded or `--keep-extra` was passed. Reproduced with exit 0 and the file gone, with no delete and no error. |
| P7-2 | Low | over-refusal / inconsistent guards | `src/commands/sync/plan.rs:331-337` vs `plan.rs:434-438,455-456`, `apply/mod.rs:84-90,150-161` | `ensure_link_safe` refuses a copy under a dst **file link** that the plan's own `fix_dirs` removes and replaces with a directory before the copy runs. `ensure_mkdir_targets_free` and `ensure_dirs_applicable` explicitly allow exactly this shape for a plain file, so the two preflights disagree. Source-only; not runtime-verified on this host. |
| P7-3 | Low | reporting contract | `src/commands/sync/rename.rs:188-193` | `RENAME` (and `renamed += 1`) run before `if dry_run` and before the two `fs::rename` calls. A failed rename therefore leaves a record for an action that did not happen; a dry run also emits `RENAME` although nothing is renamed (and the README's dry-run list omits `RENAME`). Reproduced. |
| P7-4 | Low | dry-run parity | `apply/copy.rs:252-269` vs `plan.rs:541-547,565-601` | The sixth pass's hard-link refusal exists only inside `copy_one`, so a dry run reports `COPY` and exit 0 for a tree the real run refuses with exit 3. Reproduced with `--keep-extra` and an in-dst hard link. |

Severity key: **High** = destroys data outside the requested change or corrupts the source; **Medium** = destroys dst data or blocks the run under a specific, non-default shape; **Low** = wrong report, over-refusal, or preview/outcome divergence without data damage. No finding here is High.

## P7-1 — the case-fix temporary name is taken from the user's namespace and overwrites what is there

**Where.** `rename_to_src_casing`, `src/commands/sync/rename.rs:151-248`, specifically:

```rust
// two-step for Windows case-only rename
let tmp = from.with_extension("girsync_case_tmp");
std::fs::rename(&from, &tmp)
    .with_context(|| format!("rename {} (case fix step1)", from.display()))?;
std::fs::rename(&tmp, &to)
    .with_context(|| format!("rename to {} (case fix step2)", to.display()))?;
```

Lines 204-208. The pass validates every rename's link ancestors up front (lines 168-175) precisely so a failure cannot leave dst half-renamed, but the temporary path is not validated, reserved, or checked for existence.

**Mechanism.** `tmp` is always a sibling of `from` inside `dst`, derived only from the source file name. `Path::with_extension` replaces the last extension, so:

| `from` | `tmp` |
| --- | --- |
| `data.txt` | `data.girsync_case_tmp` |
| `report.tar.gz` | `report.tar.girsync_case_tmp` |
| `README` | `README.girsync_case_tmp` |

`std::fs::rename` replaces an existing **file** destination on Windows (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`) and on POSIX (`rename(2)`); if the destination is a directory the call fails instead (`Access is denied` on Windows, `EISDIR` on POSIX). The temporary is therefore a destructive target that the dst scan and the filters never treat as a write, and `--keep-extra` cannot protect it.

**Evidence (runtime, debug binary built from this tree).** Paths abbreviated with `...`; fixtures under `E:\Symlinks\Temp\girsync_review7\verify7`.

1. **A dst file at the temporary name is replaced, and the run still exits 0.**

```
...\a\src\Data.txt               = "SRC!!"    (5 B)
...\a\dst\data.txt               = "dstdst"   (6 B)
...\a\dst\data.girsync_case_tmp  = "PRECIOUS" (8 B)

PS> girsync.exe sync --src ...\a\src --dst ...\a\dst --output json
{"event":"rename","from":"data.txt","to":"Data.txt"}
{"event":"extra","path":"data.girsync_case_tmp"}
{"event":"changed","path":"Data.txt"}
{"because":"changed","event":"copy","path":"Data.txt"}
{"copied":1,"deleted":0,"dry_run":false,"event":"summary",...}
exit=0

after: ...\a\dst\Data.txt, ...\a\dst\girpr-cache
       ...\a\dst\data.girsync_case_tmp is gone
```

The stream reports the path as `extra`, then reports `deleted:0`: the file was destroyed by the rename, not by the delete phase, and no count or error records it.

2. **`--keep-extra` does not protect it.** Same tree with `--keep-extra`:

```
{"event":"rename","from":"data.txt","to":"Data.txt"}
{"event":"extra","path":"data.girsync_case_tmp"}
{"event":"changed","path":"Data.txt"}
{"because":"changed","event":"copy","path":"Data.txt"}
{"copied":1,"deleted":0,"dry_run":false,"event":"summary","keep_extra":true,...}
exit=0
after: only Data.txt and girpr-cache
```

The flag's documented job is to spare dst-only files (`README.md:742`), and the run's own `keep_extra:true` plus `deleted:0` says it did; the file is gone.

3. **An excluded path is destroyed, with no `extra` record at all.** Same tree, `--exclude '*.girsync_case_tmp'`:

```
{"event":"rename","from":"data.txt","to":"Data.txt"}
{"event":"changed","path":"Data.txt"}
{"because":"changed","event":"copy","path":"Data.txt"}
{"copied":1,"deleted":0,"dry_run":false,"event":"summary",...}
exit=0
after: only Data.txt and girpr-cache
```

`README.md:1034-1037` says excluded paths are "treated as nonexistent on **both** sides ... kept untouched in dst during sync". This is the same class as F-1/P6-1: a destructive path chosen without consulting the effective view, except this one is not even a plan action.

4. **A directory at the temporary name aborts after a published rename (and can leave a partial pass).**

```
...\b\dst\data.girsync_case_tmp\keep.txt
PS> girsync.exe sync --src ...\b\src --dst ...\b\dst --output json
{"event":"rename","from":"data.txt","to":"Data.txt"}
ERROR girsync: fatal error="rename ...\dst\data.txt (case fix step1): Access is denied. (os error 5)"
exit=3
after: data.txt unchanged, data.girsync_case_tmp\keep.txt still there
```

With two case pairs, the first rename succeeds and the second aborts, leaving dst half-renamed:

```
{"event":"rename","from":"a.txt","to":"A.txt"}
{"event":"rename","from":"b.txt","to":"B.txt"}
ERROR fatal error="rename ...\dst\b.txt (case fix step1): Access is denied. (os error 5)"
exit=3
after: A.txt exists, b.txt still lower-case
```

The cache-row moves and `dm` re-keying run only after the loop (lines 210-246), so a mid-pass failure also leaves the dst cache keyed on names the disk no longer has. Phase A self-heals the rows on the next run, but the function's own stated goal — "Validate the whole pass before the first filesystem call ... A partial pass would leave dst half-renamed" (lines 164-167) — is not met for this failure mode.

**Failure scenario.** A user has a dst file named like the tool's temporary (`data.girsync_case_tmp`, `README.girsync_case_tmp`, ...), or excludes such a pattern because it looks like tool residue, and syncs the same directory with a case-only difference. The file is silently destroyed with exit 0; `--keep-extra` and `--exclude` do not help. If the entry is a directory, the run aborts instead, possibly after other case renames in the same pass have already landed, and leaves a false `RENAME` record (P7-3). The name is also produced by this tool itself: a run killed between the two renames leaves the temporary behind, so the collision can arise from the tool's own interrupted run.

**Why it violates the standard.**

- `README.md:698` defines `RENAME` as "dst's casing aligned to src's". A rename may change the spelling of `from`; it may not remove a different path.
- `README.md:742`: `--keep-extra` spares extras, directories as well as files. Fixture 2 destroys one while reporting it spared.
- `README.md:1034-1037`: excluded paths are "kept untouched in dst during sync". Fixture 3 destroys one with no record.
- A record must describe what happened. A file that disappears must appear as `DELETE`, as an error, or not at all — not as an unmentioned side effect of `RENAME`, with `deleted:0`.
- The temp name is an implementation detail written into the user's namespace without an existence check. The pass already has `dm` and the disk; it simply does not ask.

**Suggested fix.** Never rename onto a path that exists. Choose a unique sibling and verify it immediately before use, for example:

```rust
// Sketch: the full source name is the base, and the candidate is verified free
// immediately before the rename. OsString keeps non-UTF-8 names intact.
fn free_case_tmp(from: &Path) -> Result<PathBuf> {
    let parent = from.parent().unwrap_or(Path::new(""));
    let name = from.file_name().context("case-fix path has no file name")?;
    for n in 0..1000 {
        let mut cand_name = name.to_os_string();
        cand_name.push(format!(".girsync_case_tmp.{n}"));
        let cand = parent.join(cand_name);
        match std::fs::symlink_metadata(&cand) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(cand),
            Ok(_) => continue,
            Err(e) => return Err(e).with_context(|| format!("stat {}", cand.display())),
        }
    }
    bail!("no free case-fix temporary beside {}", from.display())
}
```

(or a random nonce, or a `tempfile` on the same volume). Because the rename pass runs before the plan, a freshly created temporary is not scanned or planned. A cheaper fail-fast alternative is to preflight every planned temporary against `dm` and the disk and refuse the pass with a specific message, but that blocks synchronization for a file the user may want to keep; the unique-name fix is preferable. Add tests for: (a) `--keep-extra` preserves a file at the old temporary name while the case rename still happens; (b) an excluded file at that name is untouched; (c) a directory at that name produces no record and no earlier rename in the same pass (or a pre-loop refusal); (d) after a simulated step1/step2 interruption, the next run reports the leftover as an extra rather than overwriting it.

If a direct case-only `rename` is reliable on the supported filesystems, the temporary can be dropped entirely; the two-step exists because it was not reliable, so that needs a test first.

## P7-2 — a planned `fix_dirs` link is refused by the link guard it is supposed to satisfy

**Where.** `ensure_link_safe` (`src/commands/sync/plan.rs:317-337`) checks every `mkdir`, `copy`, `fix_dirs`, `delete_files` and `rmdir` path:

```rust
for rel in actions {
    if let Some(link) = crate::link::link_ancestor_above(dst, rel)? {
        bail!("refusing to change `{rel}` through dst link `{link}`: ...");
    }
}
```

The same file's `ensure_dirs_applicable` (`plan.rs:356-363,374-395`) and `ensure_mkdir_targets_free` (`plan.rs:418-471`) implement the opposite rule for a **plain file** at a strict ancestor: the action is allowed exactly when that ancestor is a planned `fix_dirs` entry, because `Applier::fix_dirs` (`apply/mod.rs:134-173`, ordered before `mkdirs` at `apply/mod.rs:84-91`) removes the blocker and creates the directory first.

A dst **file link** is also kind `file` — `scan.rs:110-119` records it as a file with `is_link` set separately — so when src has a directory at that path it lands in `fix_dirs` (the type-conflict bucket, `plan.rs:229-234`), and `ensure_mkdir_targets_free`'s `fixed` set accepts it (`plan.rs:434-438,455-456`). `ensure_link_safe` then refuses the nested action.

**Source trace.**

- src: directory `a`, directory `a/b`, file `a/b/c.txt`.
- dst: `a` is a file symlink to an external file; no `a/b`, no `a/b/c.txt`.
- After the rename, `dm` has `a` as kind `file`; the diff puts `a` in `type_conflict`; `build_plan` puts `a` in `fix_dirs`, `a/b` in `mkdir` and `a/b/c.txt` in `copy`.
- `ensure_mkdir_targets_free`: for src dir `a`, `paired("a")` is the dst file, so the candidate is skipped (it becomes a fix, not a mkdir). For src dir `a/b`, `paired("a/b")` is absent, so its prefix `a` is statted: `disk_is_dir(a) == Some(false)` (the link resolves to a file), but `fixed` contains `a` because `a` is a src dir paired with a dst file, so it passes. `a/b` itself is missing (`None`), which is fine.
- `ensure_dirs_applicable`: the same shape is accepted through `dm` and `plan.fix_dirs` (`plan.rs:374-395`).
- `ensure_link_safe`: `plan.fix_dirs` contains `a`, and a top-level path has no strict ancestor, so that action passes; `plan.mkdir` contains `a/b`, and `link_ancestor_above(dst, "a/b")` returns `a` (a link is a link whether it points at a file or a directory, `link.rs:175-186`), so the run bails before it reaches the also-refused `plan.copy` entry.
- Apply order, had it been allowed: `fix_dirs("a")` sees `try_is_link(a)` and calls `link::remove(a)` (target untouched), then `create_dir_all(a)` (`apply/mod.rs:150-161`); `mkdir("a/b")` then creates the child, and the copy has a real directory and never traverses the link.

Nothing in the refusal protects a link target: the only link between `dst` and the copy is removed by the earlier phase. The refusal is a false positive produced by two guards describing different policies for the same plan.

**Failure scenario.** A dst has `a` as a symlink to an external file (a redirect the user set up), and src now has a directory `a` with files — a normal type conflict the planner resolves. The run exits 3 with "refusing to change `a/b` through dst link `a`: remove the link, or exclude it", and the user must remove a link the plan was already going to remove. The same shape with a plain file at `a` converges; only the link spelling of the same file is refused. The dry run refuses identically (the guard precedes the dry-run branch, `mod.rs:339-360`).

**Why it violates the standard.**

- `plan.rs:356-363` states the rule in the code's own words: "a strict ancestor that dst holds as a **file** is fine exactly when that ancestor is itself a planned `fix_dirs` entry: the fix removes the file and puts a directory there".
- N-1's blanket decision ("`ensure_link_safe` now refuses any action with a strict link ancestor", `ADVERSARIAL_REVIEW.md:604`) was justified by writes that resolve a *shared* link into its target. Here the link is the type-conflict entry itself and is unlinked first; the write resolves an ordinary directory.
- The planner's two preflights should agree on the same plan. Today one accepts a shape the other refuses, and the refusal is reported to the user as a link-safety problem that does not exist.

**Not runtime-verified.** This host refuses file symlinks (`New-Item -ItemType SymbolicLink` fails with `os error 1314`, "A required privilege is not held by the client"), so the shape cannot be built here. Junctions are directory links: a junction at a src directory path gives dst kind `dir`, not the file-at-a-directory-path conflict, and is correctly refused as a shared/write-through link. The trace above is source-only; this needs a developer-mode Windows host or POSIX CI.

**Suggested fix.** Narrow `ensure_link_safe` to skip a strict link ancestor that is itself a planned `fix_dirs` entry (the returned link is the shortest ancestor, and any higher link would have been returned first and would still be refused). That matches `ensure_dirs_applicable`'s `fixed` carve-out and the apply order. Alternatively, if the blanket rule is deliberate, make `ensure_mkdir_targets_free` refuse the shape too and document that a file link at a type-conflict path also blocks every descendant action; the two guards must tell the same story. Add a test (developer mode or POSIX) that a dst file link at a src directory is unlinked and replaced, and its subtree created.

## P7-3 — `RENAME` is published before the rename, and under `--dry-run`

**Where.** `src/commands/sync/rename.rs:188-208`:

```rust
report.emit(Record::pair(
    "RENAME", "->", "from", &step.from, "to", &step.to,
));
renamed += 1;

if dry_run {
    continue;
}

let from = dst_root.join(&step.from);
let to = dst_root.join(&step.to);
...
let tmp = from.with_extension("girsync_case_tmp");
std::fs::rename(&from, &tmp)...?;
std::fs::rename(&tmp, &to)...?;
```

Every apply phase does the opposite: `DELETE`/`RMDIR` are emitted after the removal (`apply/phases.rs:55-71,264-272`), `COPY` after the verified copy and cache row (`phases.rs:182-200`), `MKDIR`/`FIX-DIR` after creation (`apply/mod.rs:118-121,161-162`).

**Evidence.** Fixture 4 of P7-1: the stream contains

```
{"event":"rename","from":"data.txt","to":"Data.txt"}
ERROR girsync: fatal error="rename ...\dst\data.txt (case fix step1): Access is denied. (os error 5)"
exit=3
```

and `dst\data.txt` still has its old name and content: the record claims an action that did not happen. The two-pair fixture shows two `RENAME` records for one completed rename.

A dry run emits the same record although no rename occurs. Fixture with src `Data.txt`, dst `data.txt`:

```
PS> girsync.exe sync --src ...\c\src --dst ...\c\dst --output json --dry-run
{"event":"rename","from":"data.txt","to":"Data.txt"}
{"event":"changed","path":"Data.txt"}
{"because":"changed","event":"copy","path":"Data.txt"}
{"copied":1,"deleted":0,"dry_run":true,"event":"summary",...,"renamed":1,...}
exit=0
after: dst still holds data.txt
```

`README.md:805-808` lists the dry-run records as `FIX-DIR`/`MKDIR`/`DELETE`/`COPY`/`RMDIR` + `SUMMARY`; `RENAME` is not among them, and `plan::dry_run_records` (`plan.rs:565-601`) has no `RENAME`. So the dry-run stream contains a segment its documented definition does not.

**Failure scenario.** A script consumes the JSON stream and treats each `rename` as completed (to update an index, or to assume the later plan paths are exact). On exit 3 it sees renames that did not happen; in a dry run it sees a rename that the recorded definition says the dry run does not report, and that certainly did not happen. The summary's `renamed` similarly counts a real-run step before it succeeds, although a failure aborts before the summary is emitted, so only the record is observable today.

**Why it violates the standard.** The README says sync prints "the actions, as it takes them" (`README.md:674-675`) and the action segment is "what the apply phases do ... as they happen" (`README.md:700`). The dry run's records are documented as the same ones a real run reports (`README.md:805-808`, `plan.rs:541-547`). A record that precedes its operation is neither.

**Suggested fix.** Emit and count only after success in a real run, while keeping the dry-run counts intact:

```rust
if dry_run {
    report.emit(...);
    renamed += 1;
    continue;
}
// ... both renames ...
report.emit(...);
renamed += 1;
```

Then a failed rename leaves no record and no count. If the dry-run stream is meant to omit `RENAME` (`README.md:805-808`), drop the dry-run emit and make the summary's `renamed` reflect that choice; either way, pin it with a test. Fixture 4 of P7-1 is the failing-first test: assert exit 3, no `rename` record, and `data.txt` still present.

## P7-4 — P6-2's hard-link refusal is absent from the dry run

**Where.** The refusal lives only in `copy_one` (`apply/copy.rs:252-269`):

```rust
match crate::hardlink::count(&d) {
    Ok(1) => {}
    Ok(links) => bail!("refusing to copy `{rel}`: dst `{}` has {links} hard links, ..."),
    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
    Err(e) => return Err(e)...
}
```

`Applier::copy_files` runs only in a real run. `--dry-run` reports `plan::dry_run_records` (`mod.rs:360-374`; `plan.rs:565-601`), which emits `COPY` for every `plan.copy` item without touching the destination. `plan.rs:541-547` states that the dry-run record list is "the single definition of what a dry run reports", that the real run "cannot drift from it", and that `a_dry_run_reports_what_a_real_run_reports` pins the two lists field for field. `README.md:823` states: "A dry run must answer the same question, not a smaller one."

**Evidence.** Fixture 5 (`E:\Symlinks\Temp\girsync_review7\verify7\g`):

```
src\a.txt       = "NEW"
dst\a.txt       = "OLD"
dst\shared.txt  = hard link to dst\a.txt   (same inode; LinkType = HardLink)

PS> girsync.exe sync --src ...\g\src --dst ...\g\dst --output json --keep-extra --dry-run
{"event":"extra","path":"shared.txt"}
{"event":"changed","path":"a.txt"}
{"because":"changed","event":"copy","path":"a.txt"}
{"copied":1,"deleted":0,"dry_run":true,"event":"summary","keep_extra":true,...}
exit=0

PS> girsync.exe sync --src ...\g\src --dst ...\g\dst --output json --keep-extra
{"event":"extra","path":"shared.txt"}
{"event":"changed","path":"a.txt"}
ERROR girsync: fatal error="copy a.txt: refusing to copy `a.txt`: dst `...\g\dst\a.txt` has 2 hard links, ..."
exit=3
after: dst\a.txt still "OLD"
```

The dry run reports a copy and a successful outcome; the real run refuses before truncating. Without `--keep-extra`, the default delete of `shared.txt` removes the second name first and the copy succeeds (verified on the same fixture layout), so the divergence is observable exactly when the other name is kept (`--keep-extra`, `--missing-only`) or lies outside `dst`, where no run deletes it.

**Failure scenario.** A user previews with `--dry-run`, sees `COPY` and exit 0, and schedules the real run (or a wrapper concludes "already converged" from the dry run). The real run exits 3. Nothing is damaged — the refusal is safe — but the preview answered a different question, which is the one thing the dry-run contract forbids.

**Why it violates the standard.**

- `README.md:823` and `plan.rs:541-547`: the dry run takes the same decisions and reports the same actions; only writes are absent.
- The sixth-pass decision recorded in `ADVERSARIAL_REVIEW.md:673` applies the same rule to the neighbouring preflight: "The mkdir preflight and the real run read the same disk state, so a filtered blocker is refused alike in `--dry-run` and before any record." P6-2 added a disk-state refusal after that decision without a plan-time counterpart.
- `a_dry_run_reports_what_a_real_run_reports` only compares successful runs, so the regression is invisible to the existing test.

**Suggested fix.** Move the hard-link decision into a preflight both modes execute, and make the dry run refuse before its records. Do not simply check every copy target: a default run deletes a same-inode extra first (the fixture without `--keep-extra`) and would be falsely refused. Either (a) add an inode identity (`(volume, file index)` on Windows, `(dev, ino)` on Unix) and allow a destination with `n > 1` only when the other names are accounted for by planned `delete_files`/`rmdir` paths; or (b) if the refusal stays a runtime-only check, document the divergence in the dry-run section and add a test that pins it, so it is a known limitation rather than a silent contradiction of the "same document" claim. A third option is to stop truncating a hard-linked destination: write a temporary file beside it and rename over the destination (breaking the link, preserving the other name), which removes the refusal — but it changes write-through semantics for a dst *symlink*, so it must be limited to regular files with `nlink > 1` and tested.

## Re-checked and held

These were exercised again in this pass and showed no new defect:

- **P6-1** — `ensure_mkdir_targets_free` still runs before the rename and before any record (`mod.rs:288-299`); a filtered dst file at a mkdir ancestor is refused identically for `--dry-run` and a real run, and its tests pass.
- **P6-2** — the hard-link guard is present before `File::create` and fires when the shared name survives (`--keep-extra`, or a name outside `dst`); the Windows `GetFileInformationByHandle` path has no handle leak on the success or failure branch (`hardlink.rs:37-77`). The only gap is the dry-run parity above.
- **P6-3** — the README's test count claim matches the suite: 384 functions, 383 run by default, 1 ignored.
- **Rename link preflight** — N-5 still refuses every planned rename through a dst link before the first rename (`rename.rs:168-175`).
- **`fix_dirs` order and cache rows** — `fix_dirs` remains before `mkdirs`, removes a link at the path (not its target), and writes its directory row (`apply/mod.rs:134-172`).

## Verification

All commands were run in `E:\hoccode\hocrust\girpr` on branch `fix-gaps/girsync` at `81779c5`, before this note was written. The tree was clean and remained unchanged apart from this note.

| Command | Result |
| --- | --- |
| `cargo build -p girsync` | success, 48.96 s |
| `cargo test -p girsync` | exit 0; **383 passed, 0 failed, 1 ignored**; 384 test functions. Breakdown: lib 141; `main` 0; `sled2redb` 0; `cli_dispatch` 1; `compare` 13; `compare_self` 12; `convert` 4; `hash_mode` 18; `helpers` 12; `lazy` 27; `migration_lock` 11; `output` 16; `rw_threads` 15; `stat_trust` 19; `symlink` 24; `sync` 23 (+1 ignored); `sync_plan` 19; `update` 3; `why` 25; doctests 0 |
| `cargo test -p girsync --test symlink -- --nocapture` | exit 0; 24 passed; six file-link cases self-skip with `skip: this platform refused a file link: A required privilege is not held by the client. (os error 1314)`; every junction case ran |
| `cargo clippy -p girsync --all-targets` | exit 0; no warnings |
| `cargo fmt -p girsync --check` | exit 0 |

Runtime probes used `target\debug\girsync.exe` (the debug binary built from this tree) with `--output json`; each fixture was created fresh under `E:\Symlinks\Temp\girsync_review7\verify7` and no repository file was touched. The commands were of the form

```
girsync.exe sync --src <fixture>\src --dst <fixture>\dst --output json [--keep-extra] [--dry-run] [--exclude '*.girsync_case_tmp']
```

with the fixtures and observations quoted under P7-1, P7-3 and P7-4. The `--exclude` run in P7-1 used the glob string exactly as shown; the two hard-link runs in P7-4 used `New-Item -ItemType HardLink -Path ...\dst\shared.txt -Target ...\dst\a.txt` to create the second name (PowerShell reported `LinkType = HardLink`).

## Not verified

- **P7-2 runtime.** File symlinks cannot be created on this host (`os error 1314`), so the dst-file-link-at-a-src-directory shape is source-traced only. Needed: a developer-mode Windows or POSIX run with `dst/a` a symlink to a file outside `dst` and `src/a/b/c.txt`, checking that the current exit 3 becomes a successful `FIX-DIR` + `MKDIR` + `COPY` once the guard is narrowed (or a documented refusal in both preflights).
- **P7-1 on POSIX.** The file-replacement half is std/POSIX `rename(2)` semantics and was not executed on Unix; the directory half would fail differently (`EISDIR` rather than Windows `Access is denied`) and needs a POSIX CI run. No native symlinks were exercised here either.
- **Direct case-only rename.** Whether a single `std::fs::rename(from, to)` is reliable for case-only changes on the supported Windows filesystems (which would remove the temporary entirely) was not tested; the two-step was presumably added for a reason, so this needs a test matrix before changing.
- **Crash between the two renames.** The leftover-temporary scenario is inferred from the code and the mid-pass failure above; it was not produced by killing the process between step1 and step2, and the next run's treatment of such a leftover was not run.
- **Hard-link identity in a preflight.** The proposed inode-identity fix for P7-4 was not prototyped; whether the Windows `BY_HANDLE_FILE_INFORMATION` volume serial + file index can be obtained for every planned delete without opening a path that is already gone was not tested.
- **POSIX `nlink` and external hard links.** The hard-link fixture here is Windows-only and the second name is inside `dst`; a hard link to a file outside both roots (the open item in `ADVERSARIAL_REVIEW.md:683`) remains unexercised, and the POSIX `nlink` path was not run.
- **Whole-tree dry-run/apply equality beyond hard links.** The existing equality test covers successful runs; P7-4 is the one runtime-only guard found by inspection. No exhaustive audit of every apply-phase failure for a dry-run counterpart was attempted.
- **No new tests were added in this pass.** Every suggested regression above is a recommendation, not a pinned failing case.

---

*Seventh pass. Source review and runtime probes only; the only file this pass adds is this note. P7-1 is the reason the branch is not sound; P7-2–P7-4 are small, and P7-2 shares its shape with the already-fixed P6-1/P5-1 family.*
