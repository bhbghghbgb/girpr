# ELEVENTH ADVERSARIAL PASS — the cache prefix is reserved only in its exact spelling

**Baseline.** `fix-gaps/girsync` at `400f8cb` (clean tree). Scope and method as the
main review: `tools/sync` only, source reading plus the commands below. The subject
is the tenth pass's R10-1/R10-2 fixes (`7e11e12`, `7aca7e2`, `760ac5a`) and the
cache/record exclusion they sit beside, judged against the binding text at
`ADVERSARIAL_REVIEW.md:189-208` and `README.md:928-930,1183`.

**Verdict.** One Medium bug. `is_cache_rel` and `is_record_path` compare the
`girpr-cache` prefix byte-for-byte. On a case-insensitive filesystem a case-only
spelling of the tool's own live cache is therefore ordinary content: the walk keeps
it, the plan copies it, and the copy target is the destination's live cache file.
The run fails (or panics inside redb), and the destination cache is left unreadable
until the backup written before the run is restored. The README says the prefix is
excluded "always" and "never sync"; neither claim is true of this spelling.

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| R11-1 | Medium | bug | `src/util.rs:67-68` (also `:51-55`; caller `src/scan.rs:76`) | `is_cache_rel`/`is_record_path` match `girpr-cache` case-sensitively, so on Windows a case-renamed cache is walked as content; a real sync copies it over the live destination cache, failing with `os error 33` or a redb panic, and leaves that cache unreadable |

---

## R11-1 — a case-renamed cache is copied over the live destination cache

**Behavior (source-verified).** The reservation is a raw `starts_with` on the
component text:

```rust
// util.rs:51-55
pub fn is_record_path(p: &Path) -> bool {
    p.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.starts_with(CACHE_PREFIX))
        .unwrap_or(false)
}

// util.rs:67-68
pub fn is_cache_rel(rel: &str) -> bool {
    rel.split('/').any(|c| c.starts_with(CACHE_PREFIX))
}
```

`CACHE_PREFIX` is `"girpr-cache"` (`src/cache/schema.rs:10`). `walk_live` prunes a
matching component only when `is_cache_rel(&rel)` is true (`src/scan.rs:76-81`), so
a component such as `GIRPR-CACHE` is yielded as a file. The unit test that fixes the
reserved vocabulary lists only lowercase spellings (`util.rs:154-173`), so nothing
pins the case rule.

On NTFS `GIRPR-CACHE` and `girpr-cache` name the same directory entry. A case-only
rename does not create a second file: it renames the live cache. The source walk
then reports two files (`a.txt`, `GIRPR-CACHE`) instead of one, while the
destination walk still reports one (its cache is lowercase and pruned). The
destination side of the plan gains nothing for `GIRPR-CACHE`, so the source entry
plans as a normal `COPY`; the copy target `dst\GIRPR-CACHE` is the destination's
live DB, which the same run holds open (`src/commands/sync/mod.rs:177-178`,
`src/effective/side.rs:149,235`). `copy_one` opens its destination with
`File::create` (`src/commands/sync/apply/copy.rs:279`), which truncates.

The same miss reaches `is_record_path`: a record named with different case is not
classified as a record, so `validate_inputs` falls through to the generic
"src and dst must both be directories" refusal (`mod.rs:439-443`) instead of the
record-specific one. That half is a message/classification defect; the cache half
is the one that writes.

**Reproduction (this host, Windows/NTFS).** In
`E:\Symlinks\Temp\girsync_case_cache_probe3`:

1. Create `src\a.txt` and `dst`, then run `girsync sync --src ... --dst ...` once.
2. `Rename-Item src\girpr-cache GIRPR-CACHE` — same file, new spelling.
3. `girsync sync ... --dry-run --output json` prints, among the log lines:

   ```
   {"event":"missing","path":"GIRPR-CACHE"}
   {"because":"missing","event":"copy","path":"GIRPR-CACHE"}
   ```

   The cache is classified as a missing source file and planned for copying.
4. A real run logs `copy start files=1`, `copy progress done=1`, then:

   ```
   fatal error="copy GIRPR-CACHE: copy ...\src\GIRPR-CACHE -> ...\dst\GIRPR-CACHE:
   The process cannot access the file because another process has locked a portion
   of the file. (os error 33)"
   ```

   exit 3. The path in the error is the destination's live cache under the source's
   spelling.
5. That failed run leaves the destination cache unreadable: `girsync update --dir
   dst` then panics at `redb-4.3.0\src\tree_store\btree.rs:145` with `internal
   error: entered unreachable code` (exit 101), and a second `sync` panics the same
   way after its dst backup/snapshot. Restoring
   `dst\girpr-cache-backup-20261029T023421.339591Z` (written before the run at
   `mod.rs:146-152`) and renaming the source cache back makes both `update` runs
   exit 0 with `"files":1`.
6. One earlier fixture (`E:\Symlinks\Temp\girsync_case_cache_probe`) ended its first
   real run with the same redb panic, exit 101, rather than the clean `os error 33`
   return. Both runs attempted the same write; which of the two surfaces was not
   instrumented.

A case-renamed cache also breaks `update` without any copy: in probe3, while the
source cache was still `GIRPR-CACHE`, `girsync update --dir src` walked it, tried to
hash the still-open DB, and exited 3 with
`fatal error="hash ...\GIRPR-CACHE: ... (os error 33)"`.

**Why it fails the standard.** `README.md:928-930` says every `girpr-cache*`
component "is always excluded from scans — at **any depth**", and `:1183` says the
backups "are `girpr-cache*` so never sync". Neither sentence is qualified by
spelling, and this platform cannot distinguish the spellings anyway. The doc comment
on `is_cache_rel` states the purpose exactly — copying a cache "would at best
duplicate a database and at worst fail on its live lock" (`util.rs:61-66`) — and
this spelling does the worst case, on the tool's own destination cache. A reservation
that misses a case variant of its own filename on a case-insensitive filesystem is
not a reservation; the guard exists for exactly the manual/tool-made rename that
produced the fixture.

**Suggested fix.** Add one component predicate in `util.rs` and use it from both
callers:

```rust
#[cfg(windows)]
fn has_cache_prefix(c: &str) -> bool {
    c.get(..CACHE_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(CACHE_PREFIX))
}
#[cfg(not(windows))]
fn has_cache_prefix(c: &str) -> bool {
    c.starts_with(CACHE_PREFIX)
}
```

Then `is_cache_rel` uses `has_cache_prefix(c)`, and `is_record_path` uses
`p.file_name().and_then(|s| s.to_str()).is_some_and(has_cache_prefix)`. Keep the exact
match on POSIX, where `GIRPR-CACHE` is a different file and can be genuine content.
This mirrors the platform rule the tenth pass already uses for `on_disk_name`
(Windows-only folding, `OPEN_FOR_MAINTAINERS.md:9`); macOS's default
case-insensitive volumes stay uncovered, as they already are for R10-1.

Tests: extend `cache_paths_are_reserved_at_any_depth` (`util.rs:154-173`) with
`GIRPR-CACHE`, `sub/Girpr-Cache/rows` and `GIRPR-CACHE-backup-1` under
`#[cfg(windows)]`, and assert they are ordinary under `#[cfg(not(windows))]`. Add an
end-to-end `sync`/`update` case that renames a synced cache to `GIRPR-CACHE` and
asserts no `copy`/`missing` event for it, the destination cache still opens, and
`update --dir` exits 0; the case-only rename works on this NTFS host, so the case can
run here (the case-only parity tests need two distinct entries and self-skip because
this filesystem pairs them). For the record half, a unit test of
`classify`/`is_record_path` suffices:
a case-renamed record input must reach the `mod.rs:439-441` refusal.

The tenth-pass fixes themselves were re-read at `400f8cb` — `on_disk_name`'s
exact-first/unique-fold/`None` rules (`plan.rs`), `target_rel`'s parent strip and
canonical check, `DiskSpelling::plan_rel` mapping, and the folded alias lookup
(`rename.rs`) — and no new gap was found there. R10-2's document text is present at
`DESIGN-NOTES.md:235-292` and `README.md:1056-1068`.

---

## Verification (eleventh pass, this host)

- `cargo build -p girsync` — exit 0, no warnings.
- `cargo test -p girsync` — **426 passed, 0 failed, 1 ignored**. Breakdown: lib 168;
  cli_dispatch 1, compare 13, compare_self 12, convert 4, hash_mode 18, helpers 12,
  lazy 27, migration_lock 11, output 16, rw_threads 15, stat_trust 19, symlink 32,
  sync 31 (+1 ignored), sync_plan 19, update 3, why 25; doctests 0.
- `cargo test -p girsync --test symlink -- --nocapture` — 32 passed, 0 failed.
  Thirteen file-link bodies print `skip:` and return (`os error 1314`); one
  case-only body self-skips because this filesystem pairs `CaseProbe` with
  `caseprobe`.
- `cargo clippy -p girsync --all-targets` — exit 0, no warnings.
- `cargo fmt -p girsync --check` — exit 0.
- R11-1 was reproduced outside the repo with the current binary, as steps 1-6
  above: dry run reports `MISSING`/`COPY` for `GIRPR-CACHE`; the real run exits 3
  with `os error 33` (one earlier run exited 101 with the redb panic); the
  destination cache then panics on open until its pre-run backup is restored.

Only this file was created; no other project file was modified.

## Not verified

- POSIX and macOS: the finding is exercised only on NTFS. On a case-sensitive
  filesystem `GIRPR-CACHE` is a distinct file, so the copy-over-live-cache shape
  does not arise; the suggested POSIX branch is reasoned, not run.
- A Windows directory made case-sensitive per directory (`fsutil file
  setCaseSensitiveInfo` is denied here), where folding would also reserve a
  genuinely distinct `GIRPR-CACHE`; that trade-off is not measured.
  `OPEN_FOR_MAINTAINERS.md:9` already records the same host limitation for R10-1.
- The exact mechanism that separates the clean `os error 33` return from the redb
  panic (and the byte ranges written before the failure) was not instrumented;
  both outcomes were observed, and the post-failure open panic was reproduced.
- File-symlink end-to-end cases still self-skip on this host (`os error 1314`), as
  recorded in `OPEN_FOR_MAINTAINERS.md:8,10`; this pass adds no link fixture.
- The fix and its tests are proposed, not implemented: this pass records the
  finding only.
