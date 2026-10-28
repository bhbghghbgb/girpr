# Fourth adversarial pass — girsync

Tree: `fix-gaps/girsync` at `2315990 docs(review): record the third adversarial pass` (clean).
Scope: `tools/sync` only. Standard: strict validation, fail fast, never delete data, never
report success for a lossy run.

## Verdict

| ID | Severity | Class | Location | Summary |
|----|----------|-------|----------|---------|
| F-1 | High | bug (data loss) + test gap | `src/commands/sync/mod.rs:406-423`, `src/scan.rs:44-47,92-99`, `src/commands/sync/apply/phases.rs:25-71` | The disjointness guard canonicalizes only the two roots. A directory link inside `src` that reaches `dst` (or a target containing `dst`) is not seen; the walk follows it, `dst` files become "extra", the delete pass removes them, and the copy that needed them then fails — exit 3 after the only copy is gone. No test covers a link-shaped overlap. |

T-1…T-6 are otherwise present and correct at their primary sites; see "Re-check of the
third-pass fixes".

## F-1 — a link inside `src` hides the root overlap and the delete pass eats `dst`

`src/commands/sync/mod.rs:403-407` claims the check is link-proof:

```rust
// ... Canonical paths, so a link, a `..` or a
// case-only spelling cannot hide the overlap. Component-wise and
// case-insensitive on Windows, via `path_within`.
let src_canonical = canonical_or_raw(src);
let dst_canonical = canonical_or_raw(dst);
```

but only the roots are canonicalized. `walk_live` then follows every link inside the tree
(`src/scan.rs:44-47`, `follow_links(true)`) and marks it (`:92-99`), so the *effective*
source tree can contain `dst` even when `src_canonical` and `dst_canonical` are siblings.
`build_plan` turns the resulting `dst`-only paths into `delete_files`
(`src/commands/sync/plan.rs:186-226`), and `delete_extras` removes them by `dst` path with
no reference to `src` (`src/commands/sync/apply/phases.rs:25-71`, `remove_file` at `:57`).
`ensure_link_safe` (`plan.rs:316-336`) only inspects *dst* link ancestors, so a *src* link is
invisible to it.

Reproduction (Windows junction, no elevation needed; debug binary from this tree):

```powershell
$b = "E:\Symlinks\Temp\rev4-linkcontain2"
New-Item -ItemType Directory -Path "$b\root","$b\outside" | Out-Null
Set-Content "$b\outside\x.txt" "PRECIOUS"
cmd /c mklink /J "$b\root\link" "$b\outside"
target\debug\girsync.exe sync --src "$b\root" --dst "$b\outside" --output json
```

Observed:

```
{"event":"missing","path":"link"}
{"event":"missing","path":"link/girpr-cache"}
{"event":"missing","path":"link/x.txt"}
{"event":"extra","path":"x.txt"}
{"event":"mkdir","path":"link"}
{"event":"delete","path":"x.txt"}
ERROR girsync: fatal error="copy link/girpr-cache: ... The process cannot access the file
because another process has locked a portion of the file. (os error 33)"
exit=3
after: outside\x.txt exists = False
after: outside\link\x.txt exists = False
```

`root\link` is a junction to `outside`, so `root\link\x.txt` and `outside\x.txt` are the
same physical file. The run plans `x.txt` as extra, deletes it in the delete phase, then
cannot copy `link/x.txt` — the only copy of `PRECIOUS` is gone and the run never says so.
The same shape works with `root\link -> outside\sub` (delete `sub/...`, then copy
`link/...`): every variant ends in exit 3 after the deletion, never a clean refusal.

Second symptom in the same run: `is_cache_rel` prunes only top-level cache entries
(`src/util.rs:60-70`), so the dst cache reached through the link is planned as src content
(`link/girpr-cache` above). The copy currently dies on the live cache lock; with a cold or
copied cache it would be mirrored into `dst/link/girpr-cache`.

Why this fails the standard:

- **Fail fast:** the run mutates and deletes before it discovers the overlap. The guard is
  documented to reject this "before anything is touched" (`mod.rs:388`).
- **No silent loss / no omitted error:** exit 3 names only the cache-copy failure. Nothing
  reports that `x.txt` was deleted, still less that it was also the copy's source.
- **Docs:** `README.md:725-728` promises a mirror may not contain its own source "compared
  by canonical path", and the comment at `mod.rs:403-405` says a link cannot hide the
  overlap. Both are false for links inside `src`.
- **Test gap:** `tests/sync.rs:866-909` covers only textual nesting (`root` vs `root/sub`,
  `root` vs `sub`, sibling `root2`). `tests/symlink.rs` covers links on the *dst* side and
  materializing a src link, never a src link whose target intersects `dst`.

Suggested fix:

1. In `validate_inputs` (or immediately after phase A, while the run is still read-only),
   for every `src` live entry with `is_link && is_dir`, canonicalize `src.join(rel)` and
   refuse when the target intersects `dst_canonical` in either direction:
   `path_within(&target, &dst_canonical) || path_within(&dst_canonical, &target)`. Name the
   link and both resolved paths in the error, and add it to the "disjoint" family so the
   existing fail-fast wording and exit 3 apply. A src link pointing outside `dst` stays
   legal — that is ordinary mirrored content.
2. Defense in depth in `delete_extras`: canonicalize the parent of each delete target and
   refuse when it lies inside the canonical `src`. A delete must never be able to remove a
   file the run is mirroring, whatever the plan says.
3. Regression tests in `tests/symlink.rs`, gated like the existing junction cases:
   src link → `dst`, and src link → `dst/sub`; assert exit 3, the "disjoint"/link message,
   and that the target file still exists and still holds its bytes.

## Re-check of the third-pass fixes

| Fix | Site | Result |
|-----|------|--------|
| T-1 root containment | `mod.rs:406-423`, `tests/sync.rs:866-909` | Correct for roots; F-1 is the link gap above. |
| T-2 flush propagation | `apply/copy.rs` `flush_file` + `a_failed_flush_aborts_before_the_copy_is_cached` | Present, passes. Injector is a hook, not a real `sync_all` error (pre-existing limitation). |
| T-3 parent-first case rename | `rename.rs` `plan_steps` / no-op steps + `run_sync_renames_a_case_only_directory_before_its_children` | Present, passes. |
| T-4/T-5 redirecting-reparse predicate | `link.rs::try_is_link` + symlink cases | Present; junction cases run and pass. File-link cases self-skip here. |
| T-6 cache identity | `effective/side.rs::cache_identity` (canonical root + `CACHE_PREFIX`, `same_path`) | Present, passes via `resolve to the same cache` case. |

## Verification

All run in `E:\hoccode\hocrust\girpr` on the tree above.

| Command | Result |
|---------|--------|
| `cargo build -p girsync` | Finished, 0 warnings |
| `cargo test -p girsync` | 361 passed, 0 failed, 1 ignored (the known same-run created/removed dir case in `tests/sync.rs`) |
| `cargo test -p girsync --test symlink -- --nocapture` | 17 passed, 0 failed; 4 in-test skips: `this platform refused a file link: ... (os error 1314)` |
| `cargo clippy -p girsync --all-targets` | Finished, no warnings |
| `cargo fmt -p girsync --check` | exit 0, no diff |

Suite total breakdown: lib 129, cli_dispatch 1, compare 13, compare_self 12, convert 4,
hash_mode 18, helpers 12, lazy 27, migration_lock 11, output 16, rw_threads 15,
stat_trust 19, symlink 17, sync 20 (+1 ignored), sync_plan 19, update 3, why 25.

## Not verified

- Linux/macOS link behavior. This run is Windows-only; file symlinks are refused by the
  platform (os error 1314), so only directory junctions and ordinary dirs were exercised.
- A real `sync_all`/`fsync` failure. T-2 is exercised through the test hook
  (`GIRSYNC_TEST_NO_FSYNC`), not an injected OS error.
- Whether a *file* link inside `src` pointing at a `dst` file has the same effect. It should
  (the walk follows file links and `delete_files` still names the `dst` path), but the
  platform would not create the link, so it was not demonstrated.
- Other reparse points (cloud placeholders): `try_is_link` is documented to exclude them;
  no such volume was available.
- Crash consistency mid-run beyond the two injected failure tests.
