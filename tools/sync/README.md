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
| `commands/mod.rs` | `run` dispatch; converts raw CLI strings into `CommonOpts` (where they are validated) |
| `commands/update.rs` | refresh one folder's cache |
| `commands/compare.rs` | diff two sides, print, exit 4 on any difference |
| `commands/sync/mod.rs` | the mirror run: validate, back up, load, then rename -> plan -> apply |
| `commands/sync/rename.rs` | case-fixing rename pass, so the diff can be case-sensitive |
| `commands/sync/plan.rs` | `Plan` + `build_plan` (pure) and the `--dry-run` printer |
| `commands/sync/apply.rs` | `Applier`: the ordered apply phases, plus `copy_one` |
| `cache.rs` | redb schema (`Meta`, `FileRec`, binary codec), `open_db`, backup/snapshot helpers |
| `scan.rs` | `walk_live` (the on-disk walk) and `check_mixed_case` |
| `effective.rs` | collapses cache + filters + case rules into one `EffRec` map per side |
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

Fully refreshes `<DIR>/girpr-cache` to current disk state: stats + hashes every file
per `--hash`, records empty dirs (presence-only), prunes rows for deleted/excluded paths.
Backs up the old DB first (see below).

### compare

Each side is classified by basename: path whose final component starts with
`girpr-cache` is a **record** (DB used as-is, no FS access, no writes);
otherwise it is a **folder** (embedded `<root>/girpr-cache` loaded/created, live
stat + lazy hash, cache updated as a side effect). All 4 combos work.
Output classes, one per line: `MISSING` (src-only), `EXTRA` (dst-only),
`CHANGED` (size/mtime/hash differ), `TYPE-CONFLICT` (file vs dir),
`CASE-MISMATCH a <=> A` (insensitive mode only), then a `SUMMARY` line.
Exit `4` if any diff, `0` if equal. `sync` accepts folders only.

### sync

Mirrors `src` → `dst`:

1. Abort if canonical `src == dst`, if either side is a record path, or on any IO error
   (dangling symlink, walk loop, locked file — first error aborts the run).
2. Backup `src` + `dst` DBs to `girpr-cache-backup-<ts>`, plus snapshot dst to
   `girpr-cache-old-<ts>` — before any write. `current` is authoritative; `old-*`/`backup-*` are never auto-read.
3. Insensitive mode: rename dst paths to src casing first (`RENAME`, two-step for Windows).
4. `COPY` missing + changed files (skipped when `--missing-only`), `MKDIR` missing dirs,
   `DELETE` extra files + remove unknown dirs deepest-first (skipped when `--keep-extra`).
   Type-conflicts resolve toward src kind.
5. Copy = truncate + write in place, preserve mtime, verify-after-copy by rehash
   (size+mtime when `--hash none`); the dst record entry is deleted *before* each
   file change, so a crash re-copies rather than trusting a half-written file. No resume.

`--dry-run` prints `MKDIR/COPY/DELETE/RENAME` + `SUMMARY` and writes nothing
(no backups, no cache updates, no FS changes). The plan is exact: it is the same
work list a real run executes, including the insensitive-mode rename pass.

## Core semantics (must-know for AI edits)

- **Cache = redb file** at `<root>/girpr-cache`. Key = `/`-separated
  relative path (UTF-8; case preserved as stored). Value = binary
  `{kind, size, mtime_ns (ns since epoch), hashes{algo→raw bytes}}` in the
  `entries` table + `meta{version, case_sensitive}` in the `meta` table.
  `girpr-cache*` (DB file, backups, olds) is always excluded from scans.
  Legacy sled directories are rejected (rebuild with `--ignore-cache` or
  convert once with `sled2redb <old-dir> <new-file>`).
- **Cache is a cache, not truth.** Disk governs. Stale entries (size/mtime
  mismatch) are dropped wholesale per path and rehashed; missing algos are hashed
  on demand. `update` always populates; `compare`/`sync` populate lazily and may
  leave untouched entries stale — by design. A side reuses a cached digest only
  when size+mtime match *and* every requested algo is already stored; otherwise
  it rehashes. `--no-trust-cached-hashes <side>` rehashes that side regardless.
  Missing cache is created; corrupt cache errors (exit 3, `--ignore-cache` backs
  up + rebuilds).
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
  (bad flag values, `src == dst`, IO/verify/corrupt — message on stderr;
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

## Gotchas

- Deletes extras **by default**; in-place truncate means a killed run leaves
  truncated files (entries were pre-dropped, so rerun re-copies — verify with `compare`).
- Backups/olds accumulate (`keep-all`, suffixed `_2…` on timestamp collision);
  clean them manually. They are `girpr-cache*` so never sync.
- `--max-depth 10` silently skips deeper files — set higher for real game trees.
- Tests: `cargo test -p girsync` (incl. case-adoption regression test, which uses a
  two-step rename since Windows FS can't hold `a.txt` + `A.txt` simultaneously).
  Integration tests live in `tests/` and are grouped by concern: `helpers.rs`
  (primitives), `cli_dispatch.rs`, `update.rs`, `compare.rs`, `sync.rs`, with
  shared fixtures in `tests/common/mod.rs`. They run against the public API, so
  anything they touch must stay `pub`.