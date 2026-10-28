# TENTH ADVERSARIAL PASS — the link-storage spare, in the filesystem's spelling

**Baseline.** `fix-gaps/girsync` at `fa008e8` (clean tree). Scope and method as the
main review: `tools/sync` only, source reading plus the commands below. The subject
is the ninth pass's R9-1/R9-3 fixes (`e93b17f`) and the P8-2 spare they extend,
judged against the binding decisions at `ADVERSARIAL_REVIEW.md:160` and `:173-175`.

**Verdict.** One Medium bug and one Low documentation gap. `target_rel` leaves the
final component in the spelling the link text uses, so on a case-insensitive
filesystem a kept dst file link whose text differs in case from the on-disk entry —
or whose target this run's own rename pass moved to src's casing — is not recognized
as naming that entry. The target then misses `sm`, `dm` and `plan.delete_files`: the
run deletes the link's content, or skips the shared-inode refusal and writes through
it to a mirrored path, and exits 0. The same spelling also defeats the R9-1
component spare and the R9-3 grouping key.

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| R10-1 | Medium | bug | `src/commands/sync/plan.rs:292-312` (impact `:470-527`, `:574-607`) | The terminal-target key is the link text's final-component spelling, not the walk's; on a case-insensitive filesystem a case difference makes the spare and the mirrored-target refusal miss, so the run deletes the link's content or writes through it to a mirrored path and exits 0 |
| R10-2 | Low | doc | `DESIGN-NOTES.md:235-268`; `README.md:1056-1068` | The ninth pass added component/alias sparing and the two-kept-links-one-storage refusal, but neither document that enumerates the link-storage rules names them |

---

## R10-1 — a case-differing link text misses every map key

**Behavior (source-verified).**

`target_rel` resolves the target's **parent** and keeps the final component
verbatim:

```rust
// plan.rs:303-311
let Some(name) = target.file_name() else {
    return Ok(None);
};
let resolved = parent.join(name);
let Ok(rel) = resolved.strip_prefix(canonical_dst) else {
    return Ok(None);
};
let rel = rel.to_string_lossy().replace('\\', "/");
Ok((!rel.is_empty()).then_some(rel))
```

The parent is canonicalized, so its components come back in the on-disk case; the
final component is whatever `read_link` returned. `plan_rel` maps only the renames
this pass planned, and a real run has `on_disk = None` (`rename.rs:286-297`,
`mod.rs:298-302`), so it is the identity. The result is then used as the key into
both maps and as the path to spare:

```rust
// plan.rs:470-471, 495-527 (abridged)
let canonical_rel =
    target_rel(&target, &canonical_dst)?.map(|disk| plan_rel(on_disk, disk));
...
match sm.get(tr) {
    Some(trec) if trec.kind == "file" => {
        if plan.copy.iter().any(|item| item.rel == *rel || item.rel == tr) {
            bail!("refusing to sync `{rel}`: dst `{rel}` is a link to `{tr}`, ...");
        }
    }
    ...
    None => {
        storage.keep_path(tr);
    }
}
terminals.push((rel.clone(), tr.to_string(), tr.to_string()));
```

`LinkStorage::apply` retains `delete_files` by exact string (`plan.rs:196`), and
`ensure_shared_storage_safe` groups by the same string (`plan.rs:575-578`).

**Probe (outside the repo).** `E:\Symlinks\Temp\opencode\r10probe\probe.rs` copies
the current `target_rel` verbatim and calls it against a real case-insensitive tree:

```text
canonical_dst = \\?\E:\Symlinks\Temp\opencode\r10probe\tree\dst
target_rel(real/X.BIN) = Some("real/X.BIN")
dm contains real/x.bin = true
dm.get(target_rel) = None
control target_rel(real/x.bin) = Some("real/x.bin"), dm.get = Some("file")
canonicalize(real) = \\?\...\tree\dst\real
```

So only the final component carries the link text's spelling, and a map keyed by the
walk's spelling (`real/x.bin`, what `read_dir` reports) misses it.

**Scenario (a) — the link's content is deleted, exit 0.** dst: `real/x.bin`
(content "old") and the file link `a -> real\X.BIN`; src: `a` with content "old" and
no `real/`. `dm` holds `a`, `real`, `real/x.bin`; the diff makes `real/x.bin` an
extra and `real` an rmdir (`plan.rs:669-707`, `742-766`). The spare computes
`tr = real/X.BIN`, `sm.get` misses, and the `None` arm calls
`keep_path("real/X.BIN")`. That spares the string and its ancestors, so
`plan.rmdir` loses `real` (`plan.rs:197-198`), but `plan.delete_files` keeps
`real/x.bin` (`:196`) — the file and its directory disagree about whether the target
is storage. Deletes run before copies (`apply/mod.rs:90-97`), the delete phase
removes `real/x.bin`, `a` dangles, and the run exits 0.

**Scenario (b) — a mirrored path ends wrong, exit 0.** dst: `real/x.bin` ("old")
and `a -> real\X.BIN`; src: `a` ("new") and `real/x.bin` ("old"). The diff copies
`a` only. The spare again computes `tr = real/X.BIN`, misses `sm`, and records a
terminal under that key, so the `Some(trec)` refusal at `plan.rs:495-508` never
fires. `plan.copy = [a]`; the delete list is empty; `copy_one` opens `dst/a` and
writes through the link (`apply/copy.rs:276-279`). `real/x.bin` now holds "new"
while src's `real/x.bin` holds "old"; the run exits 0. The next run copies
`real/x.bin` back, which breaks `a` instead, and the runs alternate without
converging. This is exactly the P8-2 shape the ninth pass set out to refuse.

**Scenario (c) — a case-only rename of the target, no exotic link needed.** dst:
`real/x.bin` and `a -> real\x.bin`; src: `real/X.BIN` and `a` ("new"). The rename
pass moves the target to `real/X.BIN` on disk and re-keys `dm` to src's casing
(`rename.rs:374-466`), but a rename does not rewrite the link text. In the real run
`on_disk = None`, so `target_rel` answers `real/x.bin` and the maps (keyed
`real/X.BIN`) miss. In a **dry** run the disk still has `real/x.bin`, so the same
spelling matches `DiskSpelling::plan_rel`'s old side and is mapped to `real/X.BIN`
(`rename.rs:347-363`) — the dry run spares or refuses while the real run deletes or
writes through. A case-only rename of a link target is a routine mirror event, which
makes this the most plausible trigger.

**Scenario (d) — R9-1 and R9-3 are weakened by the same spelling.**

- R9-1: for a target `LINK/x` where `LINK` is a junction on disk named `link`,
  `spare_link_components` checks the prefix `dst/LINK` (`plan.rs:338-344`) and
  `target_rel` returns the link text's `LINK`; `keep_path("LINK")` spares a string
  the delete plan does not contain, the walk's `link` stays in `delete_files`, and
  the run unlinks the chain component the kept link resolves through.
- R9-3: two kept links `a -> real\X.BIN` and `b -> real\x.bin` produce terminal
  keys `real/X.BIN` and `real/x.bin`, i.e. two groups of one, so
  `ensure_shared_storage_safe` never fires and either copy writes the one inode.

**Why it fails the standard.** `ADVERSARIAL_REVIEW.md:160` binds the target as "that
link's **content**, not an extra" and `:173-175` record R9-1/R9-3 as fixed;
`DESIGN-NOTES.md:250-255` says a target src also has is "one inode carrying two
mirrored paths; if either end is planned for copy, the plan is refused"; the code's
own comment at `plan.rs:155-159` says the target "must not be in `delete_files`".
All three are false when the link text's final component differs in case from the
entry the walk reported, on a filesystem where both spellings are the same file.

**Fix.** Normalize every path derived from link text to the walk's spelling before
using it as a map key. In `target_rel`, when `try_is_link(target)` is false there is
no chain hop to preserve, so `target.canonicalize()` (strip `canonical_dst`) is
safe and yields the on-disk case; when the final component is a link, canonicalize
the parent and resolve the name against the parent's `read_dir` (case-insensitively
on Windows) or against a lowercase index of `dm` built once. Apply the same
normalization to the prefix result in `spare_link_components` and to
`alias_target`/`absolute_alias` before `dm.get`/`spare_alias_chain`, and pass
`case_sensitive` (or that index) into `spare_link_storage`; keep the `plan_rel`
step for the dry-run spelling. Add a unit test that calls `target_rel` with a final
component whose case differs from the on-disk entry (it runs on this
case-insensitive host without a symlink), an end-to-end case on a host with file
symlinks, and a dry-run/real-run parity case for scenario (c).

---

## R10-2 — the ninth pass's two safety rules are absent from the invariant docs

**Behavior.** `DESIGN-NOTES.md:235-268` ("A kept dst file link's target is its
content, not an extra") says an intermediate link is spared and a mirrored target is
refused. It does not mention sparing every link **component** and the alias spelling
the walk reported (`spare_link_components`, `spare_alias_chain`,
`plan.rs:323-373`), nor the whole-set rule that refuses two kept links resolving to
one storage (`ensure_shared_storage_safe`, `plan.rs:574-607`). `README.md:1056-1068`
likewise describes the single-link target and refusal but neither of those. The
review log records both as fixed (`ADVERSARIAL_REVIEW.md:173-175`), and the
enforcing cases exist (`sync_spares_a_kept_link_through_a_dst_directory_link`,
`sync_refuses_two_kept_links_that_share_one_dst_file`,
`sync_refuses_a_chained_pair_of_kept_links_sharing_storage`), but the two documents
a maintainer reads before touching the spare do not name them.

**Why it fails the standard.** `DESIGN-NOTES.md` opens by saying the dangerous edits
are the ones "nothing fails loudly about" and that its list *is* the invariants. A
refusal that keeps one inode from being given two contents is exactly such an
invariant; an undocumented one is the first thing a later simplification drops.

**Fix.** Add a sentence to the kept-link section for the component/alias spare and
one for the group refusal (including the "no copy, no refusal" arm), name the three
tests above, and mirror the two clauses in the README's symlink bullet. R10-1's
spelling fix should land first, since the rule as written today is
spelling-dependent.

---

## Verification (tenth pass, this host)

- `cargo build -p girsync` — `Finished dev profile ... in 0.38s`, exit 0, no
  warnings.
- `cargo test -p girsync` — **420 passed, 0 failed, 1 ignored**; 20 result lines,
  doctests 0. Breakdown: lib 162; main 0; sled2redb 0; cli_dispatch 1, compare 13,
  compare_self 12, convert 4, hash_mode 18, helpers 12, lazy 27, migration_lock 11,
  output 16, rw_threads 15, stat_trust 19, symlink 32, sync 31 (+1 ignored),
  sync_plan 19, update 3, why 25.
- `cargo test -p girsync --test symlink -- --nocapture` — 32 passed, 0 failed,
  exit 0. Fourteen bodies print `skip:` and return: thirteen file-link cases
  (`A required privilege is not held by the client`, `os error 1314`) and one
  case-only case (this filesystem pairs `CaseProbe` with `caseprobe`).
- `cargo clippy -p girsync --all-targets` — `Finished ... in 10.80s`, exit 0, no
  warnings.
- `cargo fmt -p girsync --check` — exit 0.
- R10-1's mechanism, probed outside the repo (`r10probe`, above) with the current
  `target_rel` copied verbatim: a case-differing final component returns a rel the
  walk's `dm` key does not match, while the on-disk spelling matches. The parent's
  `canonicalize` returns the on-disk case, so the loss is confined to the final
  component.

Only this file was created; no other project file was modified.

## Not verified

- R10-1 end to end: this host refuses file symlinks (`os error 1314`), so a kept
  dst *file* link cannot be built. The resolution and map-miss are probed; the
  plan/delete/copy consequences in scenarios (a)-(d) are traced from the cited
  lines, not executed.
- Scenario (c)'s dry-run/real-run divergence is traced, not run (it needs a file
  link); the `DiskSpelling` mapping itself is source-read only here.
- R10-1 on POSIX or on a case-sensitive directory: the issue is specific to a
  case-insensitive lookup; not exercised.
- The R9 end-to-end cases (`tests/symlink.rs:927-1063`) self-skip here, so the ninth
  pass's coverage did not execute on this host. `OPEN_FOR_MAINTAINERS.md:8` already
  records that.
- R10-2 is a documentation observation; no code path was run to test a doc.
