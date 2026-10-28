# Fifth adversarial pass — `girsync` (`tools/sync`)

- **Branch:** `fix-gaps/girsync`
- **HEAD:** `1fd075c` (`docs(review): fold the fourth adversarial pass`)
- **Date:** 2026-10-28
- **Scope:** `tools/sync` only — `src/**`, `tests/**`, `README.md`,
  `DESIGN-NOTES.md`, `Cargo.toml`; baseline `git diff main..HEAD`.
- **Method:** source audit of the current tree plus live runs of
  `target\debug\girsync.exe` on Windows 11. No source, test, README, design or
  Cargo file was changed by this pass.
- **Result:** two actionable findings, both open. Build, tests, clippy and fmt
  are green; neither finding is caught by the current suite.

## Verdict

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| P5-1 | medium | ordering / dry-run divergence | `src/commands/sync/apply/mod.rs:83-84`, `src/commands/sync/plan.rs:235-241` | `mkdirs` runs before `fix_dirs`, so a src directory nested under a `dst` type conflict aborts the real run after the dry run printed a plan that claimed success. |
| P5-2 | high | missing validation / unsafe write-through | `src/commands/sync/apply/copy.rs:210-213`, `src/commands/sync/mod.rs:451-503`, `src/commands/sync/plan.rs:329-336` | A `dst` **file** link whose target is inside `src` is written through; the copy overwrites a source file and the run can exit 0. |

P5-1 is present on `main` with the same apply order and the same `mkdir` loop.
P5-2's write-through is also unchanged from `main` (`File::create` follows a
final-component link there too); what this branch adds is the `src`→`dst`
link-overlap check and the ancestor guards, so the asymmetry — and the fourth
pass's claim that the rule is symmetric — is new even though the underlying
write-through is old. Both are reported because the review standard is the
current tree.

---

## P5-1 — `mkdirs` before `fix_dirs` makes a valid plan unexecutable

### Scenario and reproduction

`src` has a directory that itself contains a directory, and `dst` has a **file**
where the outer directory belongs:

```text
src/a/b/x.txt
dst/a            (a regular file)
```

PowerShell:

```powershell
$b = "E:\Symlinks\Temp\opencode\rev5-nestedfix2"
New-Item -ItemType Directory -Path "$b\src\a\b" -Force | Out-Null
New-Item -ItemType Directory -Path "$b\dst" -Force | Out-Null
Set-Content "$b\src\a\b\x.txt" "x" -NoNewline
Set-Content "$b\dst\a" "blocking file" -NoNewline

target\debug\girsync.exe sync --src "$b\src" --dst "$b\dst" --dry-run --output json
target\debug\girsync.exe sync --src "$b\src" --dst "$b\dst" --output json
```

Observed dry run (`--dry-run`): **exit 0**, and the plan says it would create
both the directory and the file:

```json
{"event":"missing","path":"a/b"}
{"event":"missing","path":"a/b/x.txt"}
{"event":"type-conflict","path":"a"}
{"event":"mkdir","path":"a/b"}
{"event":"fix-dir","path":"a"}
{"because":"missing","event":"copy","path":"a/b/x.txt"}
{"copied":1,"deleted":0,"dry_run":true,"event":"summary","keep_extra":false,"missing_only":false,"mkdir":1,"renamed":0,"rmdir":0}
```

Observed real run: **exit 3**, before any planned change lands:

```text
ERROR girsync: fatal error="mkdir E:\Symlinks\Temp\opencode\rev5-nestedfix2\dst\a/b: Cannot create a file when that file already exists. (os error 183)"
```

`dst\a` is still the blocking file and `dst\a\b` was not created. The plan is
valid — `fix_dirs` would remove `dst\a` and create the directory, after which
`a/b` could be created — but the phases run in the wrong order.

### Why it happens

1. `build_plan` puts `a` in `fix_dirs` because src has a directory and dst has a
   file at that path (`src/commands/sync/plan.rs:228-233`).
2. The same planner puts `a/b` in `mkdir` because the dst effective map has no
   entry for `a/b` (it has only the file `a`), and the rule is "src dir not in
   `dm`" (`plan.rs:235-241`).
3. `Applier::apply` runs `self.mkdirs(plan)?` **before**
   `self.fix_dirs(plan)?` (`src/commands/sync/apply/mod.rs:83-84`).
4. `mkdirs` calls `create_dir_all(dst.join("a/b"))`
   (`apply/mod.rs:103-118`); `dst/a` is still a file, so the OS returns
   `AlreadyExists` (183) and the run aborts.
5. The dry-run branch never executes either phase; it prints the plan records and
   returns `Ok(0)` (`src/commands/sync/mod.rs:344-359`), so it cannot expose the
   ordering failure.

### Standard violated

- The README's step 1 says a type conflict is resolved toward src
  (`README.md:723-732`), and the module doc calls the phase order a
  **correctness invariant** (`apply/mod.rs:3-16`). A plan that `fix_dirs` can
  satisfy is not executed because `mkdirs` gets there first.
- The project's dry-run contract is that a dry run takes the same decisions and
  that `sync`'s two outputs are "the same document" (`README.md:808-812`,
  `README.md:841-850`); here the dry run reports a successful plan that the real
  run cannot execute. This is exactly the dry-run/apply divergence the plan
  printer is meant to make impossible.
- Fail-fast is satisfied only in the narrow sense that the run exits 3; the
  message names the wrong operation (a `mkdir` that should never have been
  attempted) and does not name the type conflict that makes the plan valid.

### Why the suite misses it

`a_type_conflict_fix_leaves_compare_self_clean`
(`tests/symlink.rs:633-651`) uses `src/dir/x.txt` plus a `dst/dir` file. There
is no directory nested under `dir`, so `plan.mkdir` is empty and the ordering
never matters. The test passes for the cache-row reason it was written for, not
for the ordering.

### Fix

Run `fix_dirs` before `mkdirs` in `Applier::apply` (and update the module doc,
`plan::dry_run_records`, and the documented `MKDIR`/`FIX-DIR` order in
`README.md:841-846` so the dry run mirrors apply). `fix_dirs` already
uses `create_dir_all` for the replacement directory, so it creates any missing
parents itself; two `fix_dirs` entries cannot be nested because dst cannot hold
a file at both `a` and `a/b`. A regression test should use the fixture above and
assert exit 0, `dst/a/b/x.txt` exists, `dst/a` is a directory, and the dry run
also exits 0 with the same records.

Alternative: filter descendants of `fix_dirs` out of `plan.mkdir`, but then
`fix_dirs`/`copy_one` must also write the cache rows for those directories, or
`compare-self` will report them as extra (the N-2b failure mode). Reordering is
smaller and keeps the existing row-writing path.

---

## P5-2 — a `dst` file link into `src` is written through, mutating the source

### Scenario (source-verified)

```text
src/a.txt   = "AAAA"          (4 bytes)
src/b.txt   = "BBBBBBBBBB"    (10 bytes)
dst/a.txt   -> src/b.txt      (a file symlink, final component)
```

`dst` contains only that link. The expected safe outcome is a refusal before any
change, in the `disjoint` family. What the code does instead: the write-through
itself is deterministic — whenever the copy of `a.txt` runs,
`File::create(dst/a.txt)` truncates `src/b.txt`. What happens to the exit code
depends on whether `b.txt` is also in the copy set:

- With `--exclude b.txt` (or any filter that keeps `b.txt` out of the src map),
  `b.txt` is not copied, the copy of `a.txt` verifies against the 4-byte target,
  and the run exits 0. `src/b.txt` is destroyed silently. This is the cleanest
  deterministic shape.
- Without the exclude, `b.txt` is also missing from dst and is copied too.
  `copy_files` runs copies in the rayon pool, so the `b.txt` copy may read the
  original bytes, a truncated/partial target, or the completed 4 bytes. It fails
  the size check (exit 3) only if it observes a partial target; otherwise it
  exits 0. The source mutation has already happened either way, so the exit code
  is a race rather than a guard.

The numbered steps apply to the copy of `a.txt`:

1. The dst walk follows the link. `dst/a.txt` is reported as a **file** with the
   target's size and mtime, with `is_link = true`
   (`src/scan.rs:92-119`, `src/effective/phase_c.rs:203-212`).
2. `diff_maps` sees `src/a.txt` and `dst/a.txt` both as files; the sizes differ,
   so the pair is `changed` (`src/diff/mod.rs:249-259`) and the planner emits a
   copy item (`src/commands/sync/plan.rs:183-184`).
3. `ensure_link_safe` checks `link_ancestor_above` for each action
   (`plan.rs:329-330`). That helper deliberately excludes the final component
   (`src/link.rs:175-186`), so the link **at** `a.txt` is not refused.
4. `copy_one` checks only ancestors (`src/commands/sync/apply/copy.rs:172-184`).
   `d.is_dir()` is false for a file link, so the directory-link unlink branch is
   skipped, and `File::create(&d)` follows the link and truncates
   `src/b.txt` (`copy.rs:210-213`).
5. The copy writes `"AAAA"` into the target, then `set_modified` stamps the
   target with `src/a.txt`'s mtime (`copy.rs:236-240`). Verification stats
   `dst/a.txt` through the link (now 4 bytes) and hashes the same bytes, so size
   and digest both pass (`copy.rs:242-265`).
6. If `b.txt` is in the copy set, it is copied too, reading whatever
   `src/b.txt` now holds; that copy is where the exit code becomes a race (see
   above). If it is excluded, there is no second copy and the run exits 0.

Net result: `src/b.txt` loses its original 10 bytes. No record names the source
write. The source cache row for `b.txt` still records the old size/hash (if the
row exists at all), so the next run re-hashes the mutated file and accepts it —
the corruption becomes self-consistent. With `--exclude b.txt`, `dst/b.txt` is
never created; without it, `dst/b.txt` gets either the original or the mutated
bytes depending on the race, and a later run converges on the mutated source.

A self-link (`dst/a.txt -> src/a.txt`) is **not** silent: `File::create`
truncates the file after `fin` is opened, the copy reads zero bytes, and the size
check fails with exit 3. The silent case needs a distinct target inside `src`,
which is the scenario above.

### Why it is not executed here

This host refuses to create file symlinks without elevation:

```text
New-Item -ItemType SymbolicLink ... -> Administrator privilege required for this operation.
cmd /c mklink ...                 -> You do not have sufficient privilege to perform this operation.
```

The symlink test suite prints the same cause five times
(`os error 1314`), including `sync_writes_through_a_dst_file_link`
(`tests/symlink.rs:193-221`), so no file-link write-through path is exercised on
this host. P5-2 is therefore **source-verified, not executed**; the code path is
the same one the existing (self-skipping) test sanctions, with the target moved
from inside `dst` to inside `src`.

### Standard violated

- `validate_inputs` refuses root containment with the explicit rule "a mirror
  cannot write into its own source" (`src/commands/sync/mod.rs:429-436`). A dst
  link into src makes the trees overlap through the link, which is the same
  invariant, just not visible to a canonical-path check on the two roots.
- The fourth pass closed the symmetric `src`→`dst` direction with
  `validate_src_links` (`mod.rs:451-503`, called at `mod.rs:185`) and states the
  rule is "symmetric through links" (`ADVERSARIAL_REVIEW.md:1030-1031`). The
  `dst`→`src` direction has no equivalent check.
- `guard_removal_outside_src` is delete-only
  (`src/commands/sync/apply/phases.rs:393-409`), and
  `ensure_link_safe`/`copy_one` only guard ancestors, so nothing covers a
  final-component link used as a write target.
- The README sanctions write-through for a dst **file** link
  (`README.md:1032-1034`) but does not carve out a target inside `src`; a user
  reading the symlink bullet has no warning that this shape rewrites the source.

Policy note: if write-through into `src` is meant to be allowed, the docs must
say so and the fourth pass's "symmetric" claim must be withdrawn. The current
state is neither: it silently rewrites the source and can report success.
Refusing the target is the smaller change and matches the root-overlap rule.

### Fix

Add a pre-flight validation over the dst side, symmetric with
`validate_src_links`: for every `is_link` entry in the dst effective map (or, at
minimum, every `plan.copy` target), canonicalize `dst/rel` and refuse when the
resolved target is inside `src` or contains `src`, with a `disjoint` message
naming the link and both resolved paths. It must run after the dst walk and
before the rename/apply phases, like the src check. A belt-and-braces re-check in
`copy_one` before `File::create` (resolve the final component only when it is a
link — one `symlink_metadata` per copy, or pass the dst map's `is_link` flag into
the copy phase) would keep the guarantee even if the plan and disk disagree.

Regression test: mirror `sync_writes_through_a_dst_file_link` with
`dst/a.bin -> src/b.bin`, assert exit 3, a message containing `disjoint` and the
link, `src/b.bin` bytes unchanged, and no dst file created. It will self-skip on
this host and should run on a host with file-symlink privilege (or POSIX CI).

---

## Verification

Commands run on this tree (`E:\hoccode\hocrust\girpr`, branch
`fix-gaps/girsync`, HEAD `1fd075c`):

| Command | Result |
|---------|--------|
| `cargo build -p girsync` | Finished, exit 0, no warnings (cached rerun 0.40 s; the full build earlier in this pass took 42.85 s) |
| `cargo test -p girsync` | 368 passed, 0 failed, 1 ignored; exit 0 |
| `cargo test -p girsync --test symlink -- --nocapture` | 22 passed, 0 failed; exit 0; five `skip: this platform refused a file link: ... (os error 1314)` lines |
| `cargo clippy -p girsync --all-targets` | clean, exit 0, no warnings |
| `cargo fmt -p girsync --check` | exit 0 |

Suite breakdown from the full run: lib 131, cli_dispatch 1, compare 13,
compare_self 12, convert 4, hash_mode 18, helpers 12, lazy 27, migration_lock
11, output 16, rw_threads 15, stat_trust 19, symlink 22, sync 20 (+1 ignored),
sync_plan 19, update 3, why 25.

P5-1 was reproduced with the exact commands and output shown above. P5-2 was
not executed; see the not-verified list.

## Not verified

- **P5-2 execution.** File symlink creation is refused on this host
  (`os error 1314`); the finding is source-verified against the cited lines.
- **Genuine `symlink_dir` / Unix links.** Only Windows junctions and plain
  directories ran; the junction fallback is not the same code path as a native
  directory symlink on POSIX.
- **Non-link reparse points** (cloud placeholders, dedup, AppExecLink): still no
  available fixture.
- **The `set_modified` failure path** (F-6): still has no injected test.
- **P5-1 on POSIX.** The ordering bug is platform-independent by inspection, but
  only Windows was run.
- **`main` binary behavior for P5-1.** The order and the `mkdir` loop are
  identical on `main` by `git show` comparison; no `main` binary was rebuilt.
- **`--missing-only` / `--keep-extra` variants** of P5-1 and P5-2: the plan
  shapes are the same, but neither was run.

## Disposition

Both findings are review-only: no source, test, README, design-note or Cargo file
was modified. P5-2 is the higher priority because it can lose source data and
report success; P5-1 is a functional/dry-run parity bug that aborts safely.
