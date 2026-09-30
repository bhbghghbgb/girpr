# girsync — dev-only one-way folder mirror with hash cache

**Audience: developers and AI agents working on `girpr`. Not for end users.**
**Direction is always `src` (old-version reference) → `dst` (working dir). Never the reverse.**

## What it is

Workspace member (`tools/sync`, binary `girsync`) for testing version upgrades/downgrades:
snapshot a game folder's state, diff two folders/records, and restore `dst` to `src` content.
Separate from the `girpr` binary; shares no flags, no exit-code contract, no code.

## Source layout

`src/lib.rs` holds the whole implementation; `src/main.rs` is just
parse -> init tracing -> `run` -> exit. Tests are integration tests in `tests/`
and drive the public API.

| File | Responsibility |
| --- | --- |
| `cli.rs` | clap surface: `Cli`, `Cmd`, and `CommonArgs` (flattened into all three subcommands) |
| `config.rs` | validated per-run options: `CommonOpts`, `ScanMode`, `LogCtx`, `Update/Compare/SyncOpts` |
| `commands/mod.rs` | `run` dispatch; converts raw CLI strings into `CommonOpts` (where they are validated); the diff printer/exit code shared by `compare` and `compare-self` |
| `commands/update.rs` | refresh one folder's cache |
| `commands/compare.rs` | diff two sides, print, exit 4 on any difference |
| `commands/compare_self.rs` | diff a folder against its own cache, read-only |
| `commands/sync/mod.rs` | the coordinator: validate, back up, open both caches, plan across both sides, rename, then plan -> apply |
| `commands/sync/rename.rs` | case-fixing rename pass, so the diff can be case-sensitive |
| `commands/sync/plan.rs` | `Plan` + `build_plan` (pure) and the `--dry-run` printer |
| `commands/sync/apply.rs` | `Applier`: the ordered apply phases, plus `copy_one` |
| `cache.rs` | redb schema (`Meta`, `FileRec`, binary codec), `open_db` (`CacheOpen` read/write vs read-only), backup/snapshot helpers |
| `scan.rs` | `walk_live` (the on-disk walk) and `check_mixed_case` |
| `effective.rs` | the two phases for one side: `open_side`/`open_folder_side`/`scan_stat_only` (stat + cache, never hashes) and `resolve_side`/`resolve_folder`/`resolve_record` (produce what was planned); `merge_row`, the cache write rule |
| `planner.rs` | `HashPlan` and `plan_pairs`: the only place that decides which digests a run must compute |
| `diff.rs` | `Diff` buckets and `diff_maps` |
| `filter.rs`, `hash.rs`, `util.rs`, `logging.rs` | glob filters, digests, path/time/FS helpers, tracing setup |

Adding a flag: declare it in `cli.rs` (or `CommonArgs` if shared), read it from
`CommonArgs` in `config.rs`'s `TryFrom`, then use `common.<field>` in the command.

`build_plan` reads only the two effective maps, which is what makes `--dry-run`
exact. The apply phases are ordered inside `Applier::apply` because the order is
a correctness invariant: cache entries for every path a run will touch are
dropped *before* the filesystem change, so a crash re-copies rather than
trusting a half-written file.

## Commands

```
girsync update --dir <DIR> [--hash md5] [--include G --exclude G] [--case-sensitive] [--max-depth 10] [--ignore-cache]
girsync compare --src <DIR|RECORD> --dst <DIR|RECORD> [--hash md5] [--no-trust-cached-hashes src|dst] [...]
girsync compare-self --dir <DIR> [--hash md5] [--no-trust-cached-hashes] [...]
girsync sync --src <DIR> --dst <DIR> [--missing-only] [--keep-extra] [--dry-run] [--jobs 4] [--hash md5] [--no-trust-cached-hashes src|dst] [...]
```

`--hash` is repeatable (`md5`, `sha256`; default `md5`). `--hash none` is exclusive:
no hashing at all, decisions by size+mtime only (verify-after-copy also size+mtime).

`--no-trust-cached-hashes` is repeatable and takes a side: pass `src`, `dst`, or
both. On a named side, a cached digest is never reused — every file is rehashed
even when size+mtime match the cache. Off by default, and it only concerns
*digest* reuse, not the stat data (which is always re-read from disk). Combining
it with `--hash none` is harmless but pointless: there are no digests to distrust.
`update` has no such flag — it is defined as a full repopulate, so it never
trusts cached digests in the first place.

### update

Refreshes `<DIR>/girpr-cache` to current disk state: stats every file, records
empty dirs (presence-only), prunes rows for deleted/excluded paths. Backs up
the old DB first (see below).

It recomputes every algorithm named by `--hash` on every run, but that does
**not** mean every stored digest is replaced:

- **stat unchanged** — algorithms this run did not ask for are kept, so
  `--hash none` records stat data without touching stored digests at all, and
  `--hash md5` leaves an existing `sha256` alone. To refresh an algorithm, ask
  for it by name.
- **stat changed** — *all* stored digests are dropped, including algorithms
  this run did not request, since the content is assumed to have changed with
  them. Only the `--hash` set is written back.

So the cache is a superset of what you last asked for, pruned to the current
stat. `girpr-cache-backup-*` (written before the run) is the manual way back
from a bad prune; it is a recovery aid, not a correctness mechanism.

### compare

Each side is classified by basename: path whose final component starts with
`girpr-cache` is a **record** (DB used as-is, no FS access, no writes);
otherwise it is a **folder** (embedded `<root>/girpr-cache` loaded/created, live
stat + lazy hash, cache updated as a side effect). All 4 combos work, subject to
the one-cache-per-run rule below. What gets read is decided per pair, not per
side: a file on both sides with equal size and mtime is the only thing that needs
a digest (see "Cache is a cache, not truth"). Output classes, one per line:
`MISSING` (src-only), `EXTRA` (dst-only), `CHANGED` (size/mtime/hash differ),
`TYPE-CONFLICT` (file vs dir), `CASE-MISMATCH a <=> A` (insensitive mode only),
then a `SUMMARY` line. Exit `4` if any diff, `0` if equal. `sync` accepts folders
only.

### compare-self

`compare` with one argument: the record side is `--dir`'s own `girpr-cache`, so
this reports what the cache has drifted from without repairing it in the
process. That repair is the reason it is not simply
`compare --src <DIR>/girpr-cache --dst <DIR>` — that spelling is rejected by the
one-cache-per-run rule, because a folder side rewrites its cache while scanning,
so the audit would fix the drift it was reporting and a rerun would come back
clean. Here the cache is opened read-only, which is the real guarantee: the
handle has no write path at all, so a cache write could not commit even if some
future code asked for one.

Output and exit codes are `compare`'s, with the **record as `src`**:
`MISSING` = a row the cache still holds for a path that is gone, `EXTRA` = a
path on disk the cache never recorded, `CHANGED` = stat or digest disagreement.
Exits `4` on drift, `0` when in step, `3` if there is no cache to compare
against (it never creates one — that would make the first run vacuously clean)
or if `--ignore-cache` was passed (it would delete the record under audit).

**The default checks stat, not content.** The disk side is still built with the
cache available, so a file whose size+mtime match keeps its recorded digest.
That is what makes this cheap, and it is why content that changed while
preserving both size and mtime is invisible until `--no-trust-cached-hashes`
forces a rehash. Treat that flag as the difference between "the cache matches
the folder's shape" and "the cache matches the folder's bytes". The same caveat
applies to `--max-depth` and to `--include`/`--exclude`: a run whose filters
differ from the ones the cache was built with reports drift that is not there.

### sync

Mirrors `src` → `dst`:

1. Abort if `--src`/`--dst` resolve to the same cache, if either side is a record
   path, or on any IO error
   (dangling symlink, walk loop, locked file — first error aborts the run).
2. Backup `src` + `dst` DBs to `girpr-cache-backup-<ts>`, plus snapshot dst to
   `girpr-cache-old-<ts>` — before any write. `current` is authoritative; `old-*`/`backup-*` are never auto-read.
3. Open both caches and plan **across** the two sides: a path is read only when it
   is a file on both sides with equal size and mtime (see "Cache is a cache, not
   truth"). A `MISSING`, `EXTRA` or already-stat-different path costs no read.
4. Insensitive mode: rename dst paths to src casing (`RENAME`, two-step for
   Windows). This runs *after* the plan and keys on the two maps' path sets, not
   on any digest.
5. `COPY` missing + changed files (skipped when `--missing-only`), `MKDIR` missing dirs,
   `DELETE` extra files + remove unknown dirs deepest-first (skipped when `--keep-extra`).
   Type-conflicts resolve toward src kind.
6. Copy = truncate + write in place, preserve mtime, verify-after-copy by rehash
   (size+mtime when `--hash none`); the dst record entry is deleted *before* each
   file change. No resume.

Note the two case arguments in (3) and (5), which look like a typo and are not.
The plan pairs the sides **as they are on disk**, so in insensitive mode it pairs
case-insensitively; the rename in (4) then collapses exactly those pairs onto exact
keys, so the diff in (5) can be taken case-sensitively. Passing `true` to the
planner would make a case-only pair look one-sided and skip a digest it may need;
passing `false` to the diff would resurrect the `CASE-MISMATCH` bucket the rename
exists to eliminate.

The pre-drop in (5) is belt-and-braces, not the safety mechanism — every way a
copy can die is already caught by size (truncation makes a partial file
strictly smaller), by mtime (stamped only after a complete write), or by the
pre-copy digest no longer matching. It stays because it costs one batched write
and depends on none of those. Do not treat a *missing* row as a crash signal,
though: a scan may legitimately record stat without a digest, so absence proves
nothing.

`--dry-run` prints `MKDIR/COPY/DELETE/RENAME` + `SUMMARY` and writes nothing
(no backups, no cache updates, no FS changes). The plan is exact: it is the same
work list a real run executes, including the insensitive-mode rename pass. It is
asserted byte-for-byte, in both case modes, against the real binary's stdout in
`tests/sync_golden.rs` — laziness may remove reads, never a plan line.

"writes nothing" is enforced by never opening a cache for writing: an existing
cache is opened read-only and a missing one is served from an in-memory DB, so a
dry run against cache-less folders does not leave `girpr-cache` behind. This is
why `sync` opens its own handles and hands them to `effective::open_folder_side`
rather than calling `effective::open_side`, which opens a folder cache
read/write. `--dry-run --ignore-cache` is the one combination that asks for
something a dry run may not do, so the cache is treated as *absent* rather than
rebuilt — the folder is scanned from disk alone and the real cache is left as it
was.

## Core semantics (must-know for AI edits)

- **Cache = redb file** at `<root>/girpr-cache`. Key = `/`-separated
  relative path (UTF-8; case preserved as stored). Value = binary
  `{kind, size, mtime_ns (ns since epoch), hashes{algo→raw bytes}}` in the
  `entries` table + `meta{version, case_sensitive}` in the `meta` table.
  `girpr-cache*` (DB file, backups, olds) is always excluded from scans.
  Legacy sled directories are rejected (rebuild with `--ignore-cache` or
  convert once with `sled2redb <old-dir> <new-file>`).
- **Read-only vs read/write opens.** `open_db` takes a `CacheOpen`:
  `ReadWrite` may create the file, back it up, and reconcile `meta`;
  `ReadOnly` requires an existing, current-version cache and never opens it for
  writing, so the file comes out byte-identical and every write method on the
  handle fails. A read-only open cannot rewrite `meta` when the recorded
  `case_sensitive` flag disagrees with the run's mode; since that flag is
  informational, it warns and continues rather than refusing the cache. A wrong
  schema version is still fatal in both modes. Record sides and `sync --dry-run`
  both use read-only opens.
- **One cache per run.** A read/write open locks its file exclusively, so a run
  must never name the same cache twice. `compare` and `sync` both reject a
  `--src`/`--dst` pair that resolves to one cache file — the same folder twice,
  the same record twice, or a folder paired with the record inside it.
  Resolution is by *canonical* path, so two spellings of one target (`F/sub/..`
  vs `F`) collide too. Exit `3`.
- **Cache is a cache, not truth.** Disk governs. `update` always populates the
  requested algos. A folder side reuses a cached digest only when size+mtime
  match; what still has to be *read* is decided per **pair**, by
  `planner::plan_pairs`: a path needs a digest only when it is a file on both
  sides with equal size and equal mtime. A `MISSING`, `EXTRA`, `TYPE-CONFLICT` or
  already-`CHANGED` path costs no read at all, and neither does a stat-differing
  pair under `--no-trust-cached-hashes` — distrusting the cache does not make an
  unequal size uncertain. A folder side therefore leaves **stat-only rows** for
  paths it did not hash, which is a valid state: the next run sees a fresh row
  with nothing cached and asks for a digest.

  Separately, and needing no knowledge of the other side, a folder side
  **corrects its own cache** as it walks: a stale row (stat no longer matches
  disk) loses its digests, and an orphan (file gone) is dropped. That is phase A
  work, so it finishes before the planner runs and the entries the planner reads
  are already self-consistent — a stat-differing pair can never be judged against
  a pre-change digest. `--no-trust-cached-hashes` cannot suppress it: distrusting
  a cache means do not *reuse* it, never keep the rows that are wrong. Both
  corrections go out through `merge_row`, which keys on the stat alone —
  unchanged stat merges over whatever the row had, changed stat drops every
  stored digest. Missing cache is created; corrupt cache errors (exit 3,
  `--ignore-cache` backs up + rebuilds).
- **Case rules.** Stored names keep their casing. `--case-sensitive` (default
  **false**): within one side, two live/record paths differing only by case is a
  fatal conflict; across sides it is `CASE-MISMATCH` (compare) or rename-dst-first
  (sync). If disk casing differs from the cached key, the key is fixed to the
  disk name first with hashes reused. Mode mixing never errors by itself — the
  meta flag is informational; the same record works in both modes.
- **Filters.** `glob`-crate patterns on relpaths; `--include` narrows, `--exclude`
  wins over include. Excluded paths are treated as nonexistent on **both** sides
  (skipped in walks, filtered from records, pruned from DBs, kept untouched in
  dst during sync). Glob case-sensitivity follows the case mode.
- **Symlinks** are followed (`--max-depth`, default 10, applies to the whole walk —
  raise for deep trees). Dangling links abort. Link targets outside the root are
  followed and materialized as plain files/dirs. Dirs compare by presence only.
- **Exit codes:** `0` ok/equal · `2` CLI parse error (clap) · `3` fatal at runtime
  (bad flag values, both sides naming one cache, IO/verify/corrupt — message on
  stderr;
  note runtime usage mistakes also exit `3`, unlike `girpr`'s `1`)
  · `4` compare found diff. `--jobs 0` is a runtime error → exit `3`.

## Typical dev workflow

```powershell
cargo build -p girsync
.\target\debug\girsync update --dir D:\game-old          # record the reference
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live   # exit 4 + diff list
.\target\debug\girsync sync --src D:\game-old --dst D:\game-live --dry-run
.\target\debug\girsync sync --src D:\game-old --dst D:\game-live --jobs 4
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live   # exit 0
```

Record-vs-folder without touching the folder's cache:

```powershell
.\target\debug\girsync compare --src D:\game-old\girpr-cache --dst D:\game-live
```

Is a folder's own cache still in step with it? (`compare-self` never writes, so
this can be run at any time; add `--no-trust-cached-hashes` to rehash):

```powershell
.\target\debug\girsync compare-self --dir D:\game-live              # exit 0 = in step
```

## Gotchas

- Deletes extras **by default**; in-place truncate means a killed run leaves
  truncated files (entries were pre-dropped, so rerun re-copies — verify with `compare`).
- Backups/olds accumulate (`keep-all`, suffixed `_2…` on timestamp collision);
  clean them manually. They are `girpr-cache*` so never sync.
- `--max-depth 10` silently skips deeper files — set higher for real game trees.
- Tests: `cargo test -p girsync` (incl. case-adoption regression test, which uses a
  two-step rename since Windows FS can't hold `a.txt` + `A.txt` simultaneously).
  Integration tests live in `tests/` and are grouped by concern: `helpers.rs`
  (primitives), `cli_dispatch.rs`, `update.rs`, `compare.rs`, `compare_self.rs`,
  `sync.rs`, `lazy.rs` (per-fixture expected verdicts *and* expected read
  counts, each stated before the code it pins), with
  shared fixtures in `tests/common/mod.rs`. They run against the public API, so
  anything they touch must stay `pub`.