# NINTH ADVERSARIAL PASS — the link-storage spare, recomputed

**Baseline.** `fix-gaps/girsync` at `bf4d5ee` (clean tree). Scope and method as the
main review: `tools/sync` only, source reading plus the commands below. The subject
is the eighth pass's P8-2 (`spare_link_storage`, `0385eee`), judged against the
binding decisions at `ADVERSARIAL_REVIEW.md:719-720`.

**Verdict.** Three Medium findings. The function resolves a kept dst file link's
**terminal** target correctly, but accounts for the rest of the chain only by exact
path: a directory-link component is lost to `canonicalize` (R9-1), an intermediate
link's ancestor directories are not spared (R9-2), and two kept links that share one
storage are judged one at a time (R9-3). Each leaves a broken mirror with exit 0, or
fails after the delete phase has already run. No High: nothing outside `dst` is
written or removed.

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| R9-1 | Medium | bug | `src/commands/sync/plan.rs:204-213,285-296` | A chain through a dst **directory** link is resolved to the canonical spelling, so the link component the walk named stays in `delete_files`; the run unlinks it and leaves the kept link dangling (exit 0), or aborts a copy after the delete (exit 3) |
| R9-2 | Medium | bug | `plan.rs:291-296,326`; `apply/phases.rs:269` | A spared **intermediate** file link's ancestor directories are not spared; the rmdir pass removes them recursively, the kept link inside included, and the link above dangles (exit 0) |
| R9-3 | Medium | bug | `plan.rs:298-311`; `apply/copy.rs:279` | Storage shared by two kept links is judged per link; when src mirrors both paths, both copies write through the one inode and one mirrored path ends wrong (exit 0), then the runs alternate and never converge |

---

## R9-1 — an intermediate directory link is resolved away, then deleted

**Behavior (source-verified).**

`target_rel` resolves the target's **parent**, not the target path:

```rust
// plan.rs:204-213
fn target_rel(target: &Path, canonical_dst: &Path) -> Option<String> {
    let parent = target.parent()?.canonicalize().ok()?;
    let resolved = parent.join(target.file_name()?);
```

so for a kept link `a -> link/x` with `link` a directory link to `real`, the answer
is `real/x`, while the walk reported the same file as `link/x` too. The chain loop
then spares only that canonical spelling (`plan.rs:285-296`):

```rust
let Some(disk_target) = target_rel(&target, dst_root) else { break; };
let target_rel = plan_rel(on_disk, disk_target);
let target_is_link = crate::link::try_is_link(&target)...?;
if target_is_link {
    // An intermediate link is part of the chain the kept link
    // resolves through; removing it would break the kept link.
    storage.keep.insert(target_rel);
    ...
```

`dst/link/x` is a regular file reached through the link, so `target_is_link` is
false; nothing ever adds `link` (the walk's spelling of the alias) to `keep`.

**Scenario.** `dst/real/x` and the file link `dst/a -> link/x`; `dst/link` is a
junction to `dst/real`; `src/a` has the same size, mtime and content as `real/x`
(the state a previous write-through leaves). `build_plan` maps the extras `link` and
`link/x` to `delete_files = ["link"]` via `link_ancestor` (`plan.rs:411-425`).
`spare_link_storage` keeps `real/x` and the directory `real`, so those are dropped,
but `link` stays. The run executes `DELETE link` (unlinking the junction, target
intact), leaves `a` dangling, and exits 0. The next run aborts in the walk on the
dangling link (`scan.rs:51`). If `src/a` instead differs, `a` is in `plan.copy`;
deletes run before copies, so the run unlinks `link` and then fails resolving `a` —
exit 3 with the tree half changed, the exact P8-2 shape this pass was to close.

`read_link` is asked at `cur`, so an intermediate **file** link is kept; the
spelling loss is specific to a link component that is also a directory.

**Why it fails the standard.** §7.8 says “Intermediate links on the chain are
spared too”, and the code's own comment says removing one “would break the kept
link”. Both are false for a directory-link component, and `target_rel`'s
`.ok()?` additionally reads any `canonicalize` error as “outside dst”, silently
disabling the spare for a target that is inside dst.

**Fix.** Resolve the target under `dst` component by component and never collapse a
link component: for each prefix of the walked path, `try_is_link`/`dm` decides
whether it is storage, and every such link prefix (plus the final target and every
ancestor) goes into `keep`/`keep_dirs`. Keep both spellings the walk reported
(`link` and `real`, `link/x` and `real/x`). Propagate resolution errors instead of
returning `None`.

---

## R9-2 — a kept intermediate link's directories are not kept

**Behavior (source-verified).** The `target_is_link` arm keeps the link path and
moves on (`plan.rs:291-296`); only the **terminal** target arm calls
`keep_ancestors` (`plan.rs:322-327`):

```rust
None => {
    keep_ancestors(&target_rel, &mut storage.keep_dirs);
    storage.keep.insert(target_rel);
}
```

`LinkStorage.keep_dirs` is documented as “Their ancestor directories, which must not
be `rmdir`'d” (`plan.rs:169-170`), but no chain link gets that treatment.

**Scenario.** `dst/extra/b` is a file link to a target outside `dst`; `dst/a` is a
file link to `extra/b`; `src/a` equals the outside target. For `a`, the loop keeps
`extra/b`, then `target_rel` returns `None` for the outside target and breaks, so
`keep = {"extra/b"}` and `keep_dirs` is empty. `extra` is a dst-only directory in
`plan.rmdir`; `spare` drops only the exact kept path. `remove_unknown_dirs` takes
the non-link branch and calls `remove_dir_all(dst/extra)`
(`apply/phases.rs:266-273`), which removes `b` itself (std does not follow a link
child); `a` dangles and the run exits 0. The terminal target inside `dst` changes
nothing: `keep_dirs` then names that target's directories, never `extra`.

**Why it fails the standard.** A path the plan decided to keep is destroyed by a
later phase, with success reported, and the decision text says the directory above a
spared path is spared.

**Fix.** Call `keep_ancestors` for every entry inserted into `keep` (every chain
link and the terminal target), not only the terminal target.

---

## R9-3 — shared link storage is judged one link at a time

**Behavior (source-verified).** The “src also mirrors the target” refusal lives in
one arm and names only this link and its terminal target (`plan.rs:298-311`):

```rust
match sm.get(&target_rel) {
    Some(trec) if trec.kind == "file" => {
        if plan.copy.iter().any(|item| item.rel == *rel || item.rel == target_rel) {
            bail!(... one file they share ...);
        }
    }
```

Nothing records that two different kept links resolved to the same storage, and the
`target_is_link` arm above neither checks the intermediate against `sm`/`plan.copy`
nor keeps the storage. `copy_one` writes through a dst file link by design
(`apply/copy.rs:276-279`), so both copies truncate the one inode.

**Scenario.** `dst/a` and `dst/b` are file links to the same `dst/c`; `src/a` and
`src/b` are regular files with different content; `dst/c` is stale. For both links
the terminal `c` is not in `sm`, so the `None` arm keeps it and no refusal is
reached. `plan.copy = [a, b]`. `ensure_hard_linked_targets_safe` sees one link on
`c` (symlinks are not names of the inode), so it passes. The copies run (one rw
thread: `a` then `b`), `c` ends with `src/b`, `dst/a` now resolves to the wrong
content, and the run exits 0. The next run finds only `a` changed, copies it, and
breaks `b`; the runs alternate and never converge. With more than one rw thread the
two writes race the same inode as well.

The chained shape is the same omission: `dst/a -> b`, `dst/b -> c`, with `src/a` and
`src/b` both regular files; `b` is spared as an intermediate link but never compared
with `plan.copy`, so copying `a` rewrites the file `b` is meant to mirror.

**Why it fails the standard.** §7.8's rule is “one file cannot be given two
contents”, and this is one inode carrying two mirrored paths; the per-link check
cannot see the second name.

**Fix.** Record every kept link's terminal storage (canonical target / identity). If
a storage has more than one kept link and any link on it, or any chain entry that
`src` mirrors as a file, is in `plan.copy`, refuse naming both links. Also apply the
existing target-vs-`sm`/`plan.copy` test inside the `target_is_link` arm, where an
intermediate link is a mirrored name on the same inode.

---

## Verification (ninth pass, this host)

- `cargo build -p girsync` — `Finished dev profile ... in 0.27s`, exit 0, no
  warnings.
- `cargo test -p girsync` — **409 passed, 0 failed, 1 ignored**; 20 result lines,
  doctests 0. Breakdown: lib 155; main 0; sled2redb 0; cli_dispatch 1, compare 13,
  compare_self 12, convert 4, hash_mode 18, helpers 12, lazy 27, migration_lock 11,
  output 16, rw_threads 15, stat_trust 19, symlink 28, sync 31 (+1 ignored),
  sync_plan 19, update 3, why 25.
- `cargo test -p girsync --test symlink -- --nocapture` — 28 passed, 0 failed.
  Ten bodies print `skip:` and return: nine file-link cases (`A required privilege
  is not held by the client`, `os error 1314`) and one case-only case (this
  filesystem pairs `CaseProbe` with `caseprobe`).
- `cargo clippy -p girsync --all-targets` — `Finished ... in 26.74s`, exit 0, no
  warnings.
- `cargo fmt -p girsync --check` — exit 0.
- R9-1's resolution premise, probed outside the repo with `rustc` (scratch tree
  `E:\Symlinks\Temp\opencode\r9probe`): `dst/link` a junction to `dst/real`,
  `dst/real/x` a file. `parent := target.parent().canonicalize()`,
  `resolved := parent.join(file_name)`, strip `canonicalize(dst)`:
  - target `dst/link/x` → `strip_prefix = Ok("real/x")`
  - target `dst/real/x` → `strip_prefix = Ok("real/x")`

  i.e. `target_rel` returns the canonical spelling, never the walk's `link/x`.

Only this file was created; no other project file was modified.

## Not verified

- R9-1, R9-2 and R9-3 end to end: this host refuses file symlinks (`os error 1314`)
  and WSL is not usable here, so a kept dst *file* link cannot be built. The
  behaviour is traced from the cited lines and the canonicalization premise above is
  probed; the plan and apply phases are not executed for these shapes.
- R9-1's changed-content variant (exit 3 after `DELETE link`) is traced, not run;
  the equal-content variant (exit 0, dangling link) is traced only as well.
- `sync_spares_a_link_target_inside_a_dst_only_directory` — the P8-2 end-to-end test
  nearest R9-1/R9-2 — self-skips here, and the unit test at `plan.rs:1288-1314`
  calls `LinkStorage::apply` directly, so the chain loop has no executing coverage
  on this host.
- R9-2 relies on `std::fs::remove_dir_all` not following a link child (documented;
  the eighth pass probed it outside the repo). No POSIX run.
- `target_rel`'s `.ok()?` error swallowing is source-observed; a `canonicalize`
  failure other than `NotFound` was not arranged.
