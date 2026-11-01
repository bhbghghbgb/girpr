---
id: T09
title: Backup policy and the failure story
batch: D
checkpoint: D
report: [L118-L143, L542-L548, L557-L563]
design_notes: "Backups are a recovery aid, not a correctness mechanism; current is authoritative, backups are never auto-read"
depends_on: []
size: M
risk: medium
---

# T09 — Backup policy and the failure story

## Goal

Make cache backups a policy rather than a fixed pair of copies:

- `--no-backup` skips both the backup and the dst old-state snapshot;
- `--backup-retain N` keeps only the newest `N` siblings for each prefix;
- `sync` stops writing both `girpr-cache-backup-<ts>` and
  `girpr-cache-old-<ts>` for dst — dst gets only the `old-*` snapshot;
- the sibling name is claimed atomically (`create_new` for a file,
  `create_dir` for a legacy sled directory) so two runs cannot choose the same
  name;
- `README.md` documents the policy and what is left after a failure.

Default behaviour stays “back up, keep everything”: `--no-backup` is opt-in and
`--backup-retain` without a value is not allowed.

## Why

Every `sync` copies both caches and snapshots dst before it knows whether the
plan is empty. A no-op run pays for a second copy of each database and a third
copy of dst's. The report's §4.2 asks for cost proportional to the work, and
§4.4 asks for the operational story after a failure to be written down.

## Files to read

- `src/commands/sync/mod.rs` — the backup step (phase 1)
- `src/cache/open.rs` — `backup_db`, `snapshot_old`, `copy_aside`, `open_db`
- `src/util.rs` — `unique_sibling`, `ts_now`, `copy_dir_all`
- `src/config/common.rs` — `CommonOpts` and `TryFrom<CommonArgs>`
- `src/cli.rs` — `CommonArgs`
- `src/effective/side.rs` — `open_folder_cache(..., backup_first)`
- `src/commands/update.rs` — the `backup_first = true` caller
- `README.md` — the sync steps, `### update`, and the backup mentions

## Constraints

- A dry run still writes nothing; `--no-backup` changes no answer.
- `--ignore-cache` still removes both caches after the (possibly skipped)
  backup. The failure story must say that a failed removal can leave both an
  old snapshot and a live cache.
- Never auto-read a backup or `old-*` file. The policy changes how they are
  made, not how they are consumed.
- Keep `girpr-cache-backup-` and `girpr-cache-old-` prefixes: `scan.rs` and
  `util::is_cache_rel` reserve every `girpr-cache*` component.
- Retention applies only to siblings created by this tool with the same
  prefix; never delete an unrelated user file.
- Do not skip the backup based on the plan. The plan can be empty and still be
  wrong (a dry run and a real run must agree), and a plan-based skip would
  couple two phases that are deliberately separate.

## Implementation

1. Add to `CommonArgs`:
   ```rust
   #[arg(long, default_value_t = false)]
   pub no_backup: bool,
   /// Keep only the newest N backups/snapshots with this prefix.
   #[arg(long, value_name = "N")]
   pub backup_retain: Option<usize>,
   ```
   Validate `backup_retain >= 1` in `TryFrom<CommonOpts>` with exit-3-style
   `bail!`, naming `--backup-retain`.
2. Add `BackupPolicy` in `config/common.rs`:
   ```rust
   #[derive(Debug, Clone, Copy, Default)]
   pub struct BackupPolicy { pub disabled: bool, pub retain: Option<NonZeroUsize> }
   impl BackupPolicy { pub fn wants_backup(&self) -> bool { !self.disabled } }
   ```
   Put it on `CommonOpts` as `pub backup: BackupPolicy`. `CommonOpts::try_from`
   resolves the two flags into it.
3. Thread the policy through the two open paths:
   - `open_folder_cache` passes `backup_first && common.backup.wants_backup()`
     to `CacheOpen::ReadWrite`;
   - `sync`'s manual step checks `common.backup.wants_backup()` and, when true,
     calls `backup_db(&src_db_path)` and `snapshot_old(&dst_db_path)` only.
     Remove the dst `backup_db` call. Keep the `ignore_cache` removal after it.
4. In `cache/open.rs`, replace the check-then-use `unique_sibling` with an
   atomic claim. Keep a candidate loop with the same `_2`, `_3` fallback:
   - for a file, `OpenOptions::new().write(true).create_new(true).open(&dest)`;
     then `std::fs::copy(db_path, &dest)` over the empty claimed file;
   - for a directory, `std::fs::create_dir(&dest)`; then `copy_dir_all` into it;
   - on `AlreadyExists`, increment and retry.
   `unique_sibling` may stay for other callers, but `copy_aside` must no longer
   depend on its `exists()` check alone.
5. Add retention after a successful copy: list `parent` entries whose file name
   starts with `prefix`, sort by name (the timestamp format sorts
   lexicographically), and remove all but the newest `retain`. Directories are
   removed recursively; files with `remove_file`. Only apply it when
   `retain` is `Some`. A removal failure is an error (the user asked for it),
   not silently ignored.
6. Update `README.md` in the same task:
   - the sync step list: dst gets `old-*` only; src gets `backup-*`;
   - a “Backups and failures” paragraph: `--no-backup`, `--backup-retain N`,
     what a failed `--ignore-cache` removal leaves behind, and that backups are
     never read automatically;
   - the `girpr-cache-backup-*` / `old-*` mentions in `### update` and the
     concurrency note.
7. Tests:
   - `--no-backup` leaves no new `backup-*`/`old-*` sibling;
   - a normal sync creates one src `backup-*` and one dst `old-*`, and no dst
     `backup-*`;
   - `--backup-retain 2` over three runs leaves the newest two per prefix;
   - a dry run still creates nothing;
   - a failed `--ignore-cache` removal story is asserted only if a failure can
     be injected portably; otherwise pin it in the README and test the
     successful ordering.

## Commit plan

1. `feat(sync): add --no-backup and --backup-retain`
2. `fix(sync): claim backup siblings atomically and dedupe dst backups`
3. `docs(sync): document backup policy and the failure story`
4. `test(sync): pin backup policy and retention`

## Verification

```text
cargo fmt --all -- --check
cargo test -p girsync cache::
cargo test -p girsync --test migration_lock
cargo test -p girsync
```

## Acceptance

- Default runs keep the current backup set except that dst has one `old-*`
  snapshot instead of both a `backup-*` and an `old-*`.
- `--no-backup` creates no backup or snapshot, and still removes caches under
  `--ignore-cache`.
- `--backup-retain N` keeps exactly the newest N per prefix.
- Two concurrent runs cannot choose the same sibling name.
- README states what remains after a failure.

## Reviewer focus

- Is a dst `backup-*` really redundant with the `old-*` snapshot?
- Does retention ever consider a non-tool sibling?
- Is the atomic claim used for both files and legacy sled directories?
- Does `--no-backup` still honour `--ignore-cache`?
- Is the failure story accurate for the ordering in `cmd_sync`?
