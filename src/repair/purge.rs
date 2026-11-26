//! Collapse-style files-cleanup (v1 scope) — docs/02 steps 5 and 7.
//!
//! There is exactly **one** cleanup: `--purge-before` and `--purge-after` call
//! [`plan_purge`] then [`apply_purge`]; only the timing differs. It is a
//! set-difference of the files on disk against the live manifest, the same idea
//! as Collapse's `GetUnusedFileInfoList`.
//!
//! The walk and the delete are **separate calls**, so the whole
//! decision — which files, which directories, how many bytes — exists before
//! anything is deleted. That is what makes "how much would this purge delete?"
//! answerable (`--max-delete-bytes`) and printable (`--print-plan`).
//!
//! The keep rules are a pure function ([`classify_purge_path`]) so they can be
//! unit-tested exhaustively without touching a filesystem.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::Result;
use walkdir::WalkDir;

#[cfg(windows)]
use crate::util::path::path_key;
use crate::util::path::rel_from_base;
use girpr_repair_plan::{PathScope, PurgeCandidate, PurgeKeys, PurgePlan};

/// Upper bound on the scoped workers a purge stat or delete stage opens. Enough
/// to overlap thousands of independent per-path syscalls without tying one OS
/// thread to each candidate.
const MAX_PURGE_THREADS: usize = 8;

/// Apply `f` to every item on at most [`MAX_PURGE_THREADS`] scoped threads and
/// return one result per input, in input order.
///
/// The tree walk and the plan stay single-threaded; only the independent
/// per-path syscalls are fanned out. Each worker owns a disjoint slice of
/// `results`, so no lock is needed. `thread::scope` joins every worker before
/// returning, which is what lets callers emit logs in plan order and run the
/// empty-directory sweep only after all deletes have finished. Callers emit the
/// lines themselves on the calling thread: the tracing dispatcher installed by
/// `spawn_blocking_traced` is thread-local and does not reach scoped workers.
fn parallel_map<T, R, F>(items: &[T], f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    let len = items.len();
    let mut results: Vec<Option<R>> = (0..len).map(|_| None).collect();
    if len == 0 {
        return Vec::new();
    }

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, MAX_PURGE_THREADS)
        .min(len);
    if workers == 1 {
        for (slot, item) in results.iter_mut().zip(items) {
            *slot = Some(f(item));
        }
    } else {
        let chunk = len / workers + usize::from(!len.is_multiple_of(workers));
        std::thread::scope(|scope| {
            let mut rest_items = items;
            let mut rest_results = results.as_mut_slice();
            for _ in 0..workers {
                let take = chunk.min(rest_items.len());
                if take == 0 {
                    break;
                }
                let (item_chunk, item_tail) = rest_items.split_at(take);
                rest_items = item_tail;
                let (result_chunk, result_tail) = rest_results.split_at_mut(take);
                rest_results = result_tail;
                let worker_f = &f;
                scope.spawn(move || {
                    for (slot, item) in result_chunk.iter_mut().zip(item_chunk) {
                        *slot = Some(worker_f(item));
                    }
                });
            }
        });
    }
    // Every slot was written by exactly one worker; the range covers all `len`
    // inputs. Keeping the conversion here means callers never see an Option.
    results
        .into_iter()
        .map(|slot| match slot {
            Some(value) => value,
            None => unreachable!("every parallel_map slot is filled by a worker"),
        })
        .collect()
}

/// NOTE(collapse-parity): set-difference of on-disk files vs the live manifest.
/// See <https://github.com/CollapseLauncher/Collapse/blob/dc47259171794596331dffcf90db85a6ac0415ac/CollapseLauncher/Classes/InstallManagement/Genshin/GenshinInstall.cs#L177-L263>
///
/// V1 GAP (deliberate, documented): Collapse's expected set is the union of
/// the Sophon primary manifests + dispatcher persistent manifests
/// (`res_versions_external`, `data_versions`) + plugin/SDK/WPF zip entries,
/// minus `EliminateUnnecessaryAssetIndex` (unselected audio) and `ctable*`.
/// v1 only uses `{latest Sophon manifest paths}` — no dispatcher, no
/// SDK/WPF/plugin zips (game launches without them; see README v1 limits).
/// Do not "fix" the missing union without reading the v1-limits section.
///
/// The verification-state markers and the root `pkg_version` are kept for
/// **all** runs (rules 2 and 5 in [`classify_purge_path_visible`]): leaving an
/// install's existing verification state deletable by a default run was a
/// correctness bug, independent of the persistent-update feature. The
/// dispatcher's *hotfix files* are still not part of this expected set; an
/// opted-in run adds them to the repair plan.
///
/// Temps (`*_tmp`, `*.hdiff`, `chunk/`, `ldiff/`, `staging/`, legacy `*.diff` /
/// `*deletefiles*`) are NOT skipped — they are not in the live manifest, so they
/// purge here like any other orphan, into the single `deleted_extra_bytes`
/// counter.
///
/// `server_keep` comes from [`keep_set`]: files that live inside `game_dir` but
/// NEVER appear in chunk manifests.
///
/// `blacklist` is the launcher's `blacklist_dir` set (already cleaned at its
/// reader). A blacklisted path is excluded from the *repair work list*, not
/// from the install: it is still an expected file and is **never** deleted
/// (docs/01). It is a separate input from `server_keep` so the two keep rules
/// keep their distinct provenance.
///
/// Pure with respect to the walk's contents: it reads the filesystem and
/// decides, but deletes nothing.
pub fn plan_purge(game_dir: &Path, keys: &PurgeKeys, scope: &PathScope) -> PurgePlan {
    finish_plan(game_dir, walk_and_classify(game_dir, keys, scope))
}

/// What one `WalkDir` pass collected, before it is sorted into a [`PurgePlan`].
struct WalkResult {
    files: Vec<PurgeCandidate>,
    all_dirs: Vec<PathBuf>,
    /// Ancestors of every `Keep`-classified file: anything else is left empty
    /// by the purge and eligible for the emptied-dir sweep.
    kept_dirs: HashSet<PathBuf>,
    hidden: u64,
    errors: u64,
}

/// The walk half of [`plan_purge`]: read the tree and classify each entry.
///
/// The keep inputs were folded once per run; this pass only walks and keys the
/// paths it meets.
fn walk_and_classify(game_dir: &Path, keys: &PurgeKeys, scope: &PathScope) -> WalkResult {
    let expected = keys.expected();
    let server_keep = keys.server_keep();
    let blacklist = keys.blacklist();
    // Keep the DirEntry with each candidate so the stat stage can call the
    // same `DirEntry::metadata()` the old inline path did: on Windows that is a
    // cached read from the walk (no syscall), on Unix an lstat. Both are safe to
    // run on scoped threads, and only the latter is a real parallel syscall.
    let mut pending: Vec<(PurgeCandidate, walkdir::DirEntry)> = Vec::new();
    let mut all_dirs: Vec<PathBuf> = Vec::new();
    let mut kept_dirs: HashSet<PathBuf> = HashSet::new();
    let mut hidden = 0u64;
    let mut errors = 0u64;

    for e in WalkDir::new(game_dir).follow_links(false) {
        let e = match e {
            Ok(e) => e,
            // The documented missing-`game_dir` case: nothing to plan, not a
            // failure. Every other error means part of the install could not
            // be read — that is one candidate set the plan does not have, so
            // it is counted and logged, never dropped silently.
            Err(err)
                if err
                    .io_error()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                continue;
            }
            Err(err) => {
                errors = errors.saturating_add(1);
                tracing::warn!(
                    path = ?err.path(),
                    error = %err,
                    "purge walk: a path could not be read; the candidate list may be incomplete"
                );
                continue;
            }
        };
        if e.file_type().is_dir() {
            let path = e.path();
            // `game_dir` itself is never swept, and it is not an
            // out-of-scope path either.
            if path == game_dir {
                continue;
            }
            // A directory is a path like any other. The scope hides it, so the
            // sweep must not take it: without this, an empty directory has no
            // kept file to mark its ancestors (`mark_kept_ancestors` walks up
            // from *files*) and the sweep would delete an out-of-scope path.
            let Some(rel) = rel_from_base(path, game_dir) else {
                continue;
            };
            if !scope.is_visible(&rel) {
                hidden += 1;
                continue;
            }
            all_dirs.push(path.to_path_buf());
            continue;
        }
        if !e.file_type().is_file() {
            continue;
        }
        let path = e.path();
        let Some(rel) = rel_from_base(path, game_dir) else {
            continue;
        };
        // Match the scope once and hand the answer to the classifier: rule 0
        // needs exactly this value, and the `hidden` count needs it too.
        let visible = scope.is_visible(&rel);
        match classify_purge_path_visible(&rel, visible, expected, server_keep, blacklist) {
            PurgeVerdict::Keep => mark_kept_ancestors(path, game_dir, &mut kept_dirs),
            PurgeVerdict::Purge => {
                debug_assert!(visible, "a hidden path must classify Keep");
                // The stat is deferred to a bounded parallel stage after the
                // walk. A placeholder keeps the candidate's place until then.
                pending.push((
                    PurgeCandidate {
                        rel,
                        path: path.to_path_buf(),
                        bytes: 0,
                    },
                    e,
                ));
            }
        }
        if !visible {
            hidden += 1;
        }
    }

    // Stat the candidates on bounded scoped threads. Sorting first makes the
    // result slots — and the warnings emitted from them — follow the same
    // deterministic path order the plan's delete list uses.
    pending.sort_by(|(a, _), (b, _)| a.path.cmp(&b.path));
    let stats = parallel_map(&pending, |(_, entry)| entry.metadata().map(|m| m.len()));
    let mut files = Vec::with_capacity(pending.len());
    for ((candidate, _), stat) in pending.into_iter().zip(stats) {
        let bytes = match stat {
            Ok(len) => len,
            Err(err) => {
                // A candidate that cannot be stat'd is still a candidate: the
                // delete does not need its size. Counting it as zero would
                // understate the ceiling, so say so and count the failure.
                errors = errors.saturating_add(1);
                tracing::warn!(
                    path = %candidate.path.display(),
                    error = %err,
                    "purge: cannot stat a delete candidate; its bytes are not counted"
                );
                0
            }
        };
        files.push(PurgeCandidate { bytes, ..candidate });
    }

    WalkResult {
        files,
        all_dirs,
        kept_dirs,
        hidden,
        errors,
    }
}

/// The finish half of [`plan_purge`]: sort, total, sweep the emptied dirs, and
/// report what the walk found.
fn finish_plan(game_dir: &Path, walk: WalkResult) -> PurgePlan {
    let WalkResult {
        mut files,
        all_dirs,
        kept_dirs,
        hidden,
        errors,
    } = walk;

    // Deterministic delete order, so a rerun's log is comparable.
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut dirs: Vec<PathBuf> = all_dirs
        .into_iter()
        .filter(|d| *d != game_dir && !kept_dirs.contains(d))
        .collect();
    // Deepest first. `sort_by_cached_key` computes the depth once per directory
    // instead of on every comparison (`components()` walks the path lexically).
    // It is a *stable* sort with the same key as before, so directories at the
    // same depth keep their walk order — byte-for-byte identical removal order.
    dirs.sort_by_cached_key(|a| std::cmp::Reverse(a.components().count()));
    let total_bytes = files.iter().fold(0u64, |n, c| n.saturating_add(c.bytes));
    // The scope and a purge flag agreeing is correct, but it should be
    // visible. `plan_purge` runs once per purge pass, so a run with both
    // `--purge-before` and `--purge-after` says this twice — one line per
    // decision, which is what actually happened.
    if hidden > 0 {
        tracing::warn!(
            "{hidden} on-disk paths are outside the active --include/--exclude scope and will not be deleted"
        );
    }
    if errors > 0 {
        tracing::warn!(
            errors,
            "purge plan is incomplete: the delete total is a lower bound"
        );
    }
    PurgePlan {
        files,
        dirs,
        total_bytes,
        hidden,
        errors,
    }
}

/// Carry "this directory still has something to keep" up from a kept file to
/// `game_dir`, so the emptied-dir sweep never sweeps a dir whose only contents
/// were kept. `game_dir` itself is excluded (it is never removed).
fn mark_kept_ancestors(file: &Path, game_dir: &Path, out: &mut HashSet<PathBuf>) {
    let Some(mut dir) = file.parent() else {
        return;
    };
    while dir != game_dir {
        if !dir.starts_with(game_dir) {
            return;
        }
        // An earlier kept file already marked this directory and, by
        // construction, every ancestor up to `game_dir`; no new key is needed.
        if out.contains(dir) {
            return;
        }
        out.insert(dir.to_path_buf());
        let Some(parent) = dir.parent() else {
            return;
        };
        dir = parent;
    }
}

/// Execute a decided [`PurgePlan`]. Returns the number of bytes actually
/// deleted, which is always `0` in dry-run (the candidates are only logged,
/// and the sweep is skipped entirely).
///
/// A dry run always ends with the per-pass `purge dry-run: N bytes would be
/// freed` line, **including** `N = 0` for a plan with no candidates. The zero
/// line is the evidence that this pass reached the delete step and found
/// nothing to list; it keeps an empty pass distinguishable from one the caller
/// skipped or from the after pass's dedup early return, which emits no byte
/// line at all.
pub fn apply_purge(plan: &PurgePlan, dry_run: bool) -> Result<u64> {
    if dry_run {
        let mut would_be = 0u64;
        for c in &plan.files {
            // Saturating: the sum of on-disk sizes can only exceed `u64::MAX`
            // if the volume itself could, so the clamp is unreachable and the
            // add is never allowed to panic on a large install.
            would_be = would_be.saturating_add(c.bytes);
            tracing::info!("would purge extra {} ({} bytes)", c.path.display(), c.bytes);
        }
        tracing::info!(
            "purge dry-run: {would_be} bytes would be freed (deleted_extra_bytes stays 0 by design)"
        );
        return Ok(0);
    }

    // The candidates are distinct paths from one non-following walk, so their
    // deletes are independent. Fan them out on bounded scoped threads and
    // collect one outcome per index slot. `scope` joins every worker before the
    // next line runs.
    let outcomes = parallel_map(&plan.files, |c| std::fs::remove_file(&c.path));
    // Emit in plan order, not completion order, so a rerun's log is comparable.
    // Logging here (not in a worker) also keeps it on the caller's dispatcher.
    let mut deleted_bytes = 0u64;
    for (c, outcome) in plan.files.iter().zip(outcomes) {
        match outcome {
            Ok(()) => {
                deleted_bytes = deleted_bytes.saturating_add(c.bytes);
                tracing::info!("purged extra {} ({} bytes)", c.path.display(), c.bytes);
            }
            Err(e) => {
                // Best effort: a locked file is not a reason to fail a run
                // whose repair already succeeded, but it must not be
                // counted as freed space either.
                tracing::warn!(
                    path = %c.path.display(),
                    error = %e,
                    "cleanup could not delete a candidate; left in place"
                );
            }
        }
    }
    // All deletes are joined above; only now may the emptied dirs be swept.
    remove_emptied_dirs(&plan.dirs);
    Ok(deleted_bytes)
}

/// Remove the directories the purge emptied, deepest first. The set comes from
/// the walk ([`PurgePlan::dirs`]), so no second walk is needed. A `remove_dir`
/// that fails — usually because something appeared since the plan was built,
/// but also a sharing violation or an ACL — is best effort and logged with its
/// reason; the plan's job here is only to shrink the tree. `game_dir` is never
/// in the set.
fn remove_emptied_dirs(dirs: &[PathBuf]) {
    for d in dirs {
        // A re-appearing file is the expected failure, but a sharing
        // violation, an ACL or a read-only parent is not — and after `let _`
        // the two were indistinguishable. The directory the plan said it
        // would remove is state: keep the reason it is still there.
        if let Err(e) = std::fs::remove_dir(d) {
            tracing::debug!(
                dir = %d.display(),
                error = %e,
                "the emptied dir was not removed"
            );
        }
    }
}

/// Root-level files the files-cleanup must keep.
///
/// `config.ini` lives inside `game_dir` but NEVER appears in Sophon chunk
/// manifests, and the tool rewrites it after patching — purging it would break
/// the version bookkeeping. The lock is another root-level file that is not in
/// the manifest: older runs wrote `girpr.lock` there, and a leftover is
/// preserved rather than deleted behind the user's back. The run's own lock
/// lives outside the install ([`girpr_lock`]), so a live run never depends on
/// this rule; it exists only for the legacy file.
///
/// `pkg_version` is the HoYoPlay launcher's verification list. It is a
/// Sophon-tracked base file today, but an install restored by other means may
/// carry one that is not in the current plan; deleting it invites a
/// launcher-side re-check (docs/01 §6). It is kept for **all** runs, not only
/// persistent ones.
///
/// The rest of the verification state (`*_persist`, `*_revision`, the marker
/// names, `cache_versions_*`, `ctable*`, `audio_lang_*`,
/// `Audio_*_pkg_version`) is matched by filename in rule 5 of
/// [`classify_purge_path_visible`], not here, because its parent directory
/// varies by channel and exe stem. Everything else (exe, res-category /
/// audio-scan bookkeeping, `ScreenShot/`, logs, temps) is intentionally NOT
/// kept: Collapse's `GenshinInstall.GetUnusedFileInfoList` protects only
/// `FilesCleanupIgnoreList` (empty for GI) plus the `Audio_*_pkg_version`
/// regexes, and this tool maximizes free space — if the user wants anything
/// else kept, that needs an explicit `--keep` flag (not implemented in v1).
///
/// Blacklisted paths are a **separate** keep input to [`plan_purge`] (rule 3),
/// not part of this set: they come from the launcher's `blacklist_dir`, while
/// these are metadata keeps the cleanup recognizes. The doc for [`keep_set`]
/// is not the whole expected set; [`classify_purge_path`] is.
///
/// NOTE: the game flags unknown executables inside the game folder, so the
/// tool binary itself must never live there; the exe needs no keep rule.
///
/// All entries are `/`-separated rel paths (same normalization as
/// `girpr_repair_plan::build_plan` and the purge walk). The set is fixed,
/// so it is built once and shared rather than allocated per call.
///
/// The match is against the **whole rel path**, so these rules are root-only:
/// `config.ini`, `girpr.lock` and `pkg_version` are kept at the game dir's
/// root, while `sub/pkg_version` and `sub/config.ini` are ordinary orphans
/// and are purged. That is deliberate — these files only ever lived at the
/// root — and is pinned by `the_root_keep_rules_are_root_only`.
pub fn keep_set() -> &'static HashSet<String> {
    static KEEP: OnceLock<HashSet<String>> = OnceLock::new();
    KEEP.get_or_init(|| {
        HashSet::from([
            "config.ini".to_string(),
            // Older runs kept the single-writer lock at the game dir's root and
            // it is not in the manifest. A leftover must not be eaten by the
            // sweep: this run's lock is outside the install, so the keep rule
            // preserves the legacy file without carrying its exclusion.
            girpr_lock::LOCK_FILE.to_string(),
            // The launcher's verification list. Kept at the root only: a
            // nested file with the same name is not that list (docs/01 §6).
            "pkg_version".to_string(),
        ])
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeVerdict {
    Keep,
    Purge,
}

/// Decide the fate of one `/`-separated rel path for the files-cleanup.
/// Pure function (no I/O) so it can be unit-tested exhaustively.
///
/// Convenience wrapper around [`classify_purge_path_visible`]: it computes
/// the path-scope verdict itself. [`plan_purge`] already knows whether a
/// walked path is visible, so it calls the variant directly and matches the
/// scope once per file.
#[cfg(test)]
pub fn classify_purge_path(
    rel: &str,
    expected: &HashSet<String>,
    server_keep: &HashSet<String>,
    blacklist: &HashSet<String>,
    scope: &PathScope,
) -> PurgeVerdict {
    classify_purge_path_visible(rel, scope.is_visible(rel), expected, server_keep, blacklist)
}

/// `classify_purge_path` with the path-scope verdict (`visible`) already
/// computed.
///
/// `expected`, `server_keep` and `blacklist` must already be keyed with
/// `path_key`; [`PurgeKeys`] does that once per run. `rel` is keyed here,
/// once per file. That keeps the classification case-insensitive on Windows
/// (the plan's dedup and the scope already are) while leaving Unix paths
/// distinct.
///
/// Rule order matters (first match wins); each rule names its provenance:
/// 0. **Path scope**: a path outside `--include`/`--exclude`
///    does not exist, so it is kept — *before* the manifest check, not after.
///    Order is load-bearing: the plan's rels (`expected_rels()`) are the only
///    manifest-derived input to the keep set, so a scope applied anywhere but
///    here would leave the cleanup with a *narrower* keep set than an
///    unfiltered run, and the fixture `gircap` hands us would be eaten.
/// 1. Manifest membership (Collapse expected set).
/// 2. `config.ini` and the single-writer lock ([`keep_set`] — the metadata
///    keeps the tool itself owns).
/// 3. Blacklist membership (the launcher's `blacklist_dir`): a blacklisted
///    path is excluded from the work list but is still part of the expected
///    install, so the cleanup must never delete it (docs/01).
/// 4. Collapse audio pkg-version parity (`audio_lang_*` + case-insensitive
///    `Audio_*_pkg_version`, `GenshinInstall.cs:232-241`). v1 matches broadly
///    by pattern rather than per-`audio_lang_14`-line exact regexes.
/// 5. Verification-state markers and Sophon bookkeeping (`*_persist`,
///    `*_revision`, `ChannelName`, `ScriptVersion`, `PatchDone`,
///    `base_res_version_hash`, `cache_versions_*`, `ctable*`). Matched by
///    filename because their parent dir varies by channel and exe stem.
///    `*versions_remote*` and `VideoAssets/*.tmp` are explicitly NOT kept:
///    they are stale game-generated download state (docs/01 §6).
///
///    Everything else purges — including temps (`*_tmp`, `*.hdiff`, `chunk/`,
///    `ldiff/`, `staging/`, `*.diff`, `*deletefiles*`), `ScreenShot/`, logs,
///    exe and server bookkeeping files.
pub fn classify_purge_path_visible(
    rel: &str,
    visible: bool,
    expected: &HashSet<String>,
    server_keep: &HashSet<String>,
    blacklist: &HashSet<String>,
) -> PurgeVerdict {
    // 0. Out of scope → does not exist.
    if !visible {
        return PurgeVerdict::Keep;
    }
    // The three sets are keyed with `path_key`. That fold is the identity on
    // non-Windows hosts, so there a borrowed `&str` probe hits the same buckets
    // with no throwaway `String` per walked path (`HashSet<String>` borrows `str`).
    // Only Windows, where the key is ASCII-folded, needs an owned folded copy.
    #[cfg(windows)]
    let folded: Option<String> = Some(path_key(rel));
    #[cfg(not(windows))]
    let folded: Option<String> = None;
    let key: &str = folded.as_deref().unwrap_or(rel);
    // 1. In the live manifest → keep.
    if expected.contains(key) {
        return PurgeVerdict::Keep;
    }
    // 2. config.ini → keep.
    if server_keep.contains(key) {
        return PurgeVerdict::Keep;
    }
    // 3. Blacklisted → excluded from repair, still part of the install.
    if blacklist.contains(key) {
        return PurgeVerdict::Keep;
    }
    // 4. Collapse audio pkg-version parity → keep.
    let name = file_name(rel);
    if is_audio_version_file(name) {
        return PurgeVerdict::Keep;
    }
    // 5. Verification state → keep. Never gated on a flag: an install's
    // existing markers must survive a plain `--purge-after`/`--purge-only`.
    if is_verification_state_file(name) {
        return PurgeVerdict::Keep;
    }
    PurgeVerdict::Purge
}

/// Collapse `GenshinInstall.cs:232-241` parity: `audio_lang_14` (any
/// `audio_lang_*`) plus `Audio_<lang>_pkg_version` (matched case-insensitively
/// — on disk it is `Audio_English_pkg_version`, i.e. capital A).
///
/// The rejection from [`is_verification_state_file`] is repeated here because
/// this rule runs first: `VideoAssets/*.tmp` and `*versions_remote*` are stale
/// game-generated download state and must stay purge candidates even when an
/// audio-style name would otherwise keep them (docs/01 §6).
fn is_audio_version_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    if lower.contains("versions_remote") || lower.ends_with(".tmp") {
        return false;
    }
    lower.starts_with("audio_lang_")
        || (lower.starts_with("audio_") && lower.ends_with("_pkg_version"))
}

/// Verification-state markers and Sophon bookkeeping that the game reads.
///
/// Matched on the file name, not the full rel path: `<exe>_Data/Persistent/`
/// moves with the game's exe stem, and the same `*_revision` / `*_persist`
/// names occur in both the base and hotfix trees. Without this rule a marker
/// absent from the live Sophon manifest is deleted by `--purge-after` /
/// `--purge-only`, and the game re-verifies at next launch (docs/01 §6).
///
/// `*versions_remote*` and `*.tmp` are rejected first: they are stale
/// game-generated download state and must stay purge candidates even when
/// their name contains an otherwise protected token (for example
/// `cache_versions_remote`).
fn is_verification_state_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    if lower.contains("versions_remote") || lower.ends_with(".tmp") {
        return false;
    }
    // The three dispatcher mirrors (`res_versions_persist`,
    // `data_versions_persist`, `silence_data_versions_persist`) plus every
    // future `*_persist` marker.
    if lower.ends_with("_persist") {
        return true;
    }
    // `res_revision_eternal` carries the token mid-name, so `contains`.
    if lower.contains("_revision") {
        return true;
    }
    if matches!(
        lower.as_str(),
        "channelname" | "scriptversion" | "patchdone" | "base_res_version_hash"
    ) {
        return true;
    }
    lower.starts_with("cache_versions") || lower.starts_with("ctable")
}

fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repair::scope::{LimitHit, Limits, Preflight};
    use crate::util::path::path_key;
    use girpr_repair_plan::{PlannedFile, RepairPlan};
    use girpr_tempdir::TempDir;
    use std::sync::Arc;

    fn purge_fixture() -> (HashSet<String>, HashSet<String>) {
        let expected: HashSet<String> = ["game.dat".to_string()].into_iter().collect();
        let server_keep = keep_set().clone();
        (expected, server_keep)
    }

    /// The plan-derived keep inputs, folded the way `plan_phase` does.
    fn keys_for(plan: &RepairPlan, blacklist: &HashSet<String>) -> PurgeKeys {
        PurgeKeys::new(plan, keep_set(), blacklist)
    }

    /// No blacklist, for the tests that predate it.
    fn no_blacklist() -> HashSet<String> {
        HashSet::new()
    }

    fn plan_of(rel: &str) -> RepairPlan {
        plan_of_all(&[rel])
    }

    fn plan_of_all(rels: &[&str]) -> RepairPlan {
        RepairPlan::from_files(
            "5.0",
            rels.iter()
                .map(|rel| PlannedFile {
                    rel: (*rel).into(),
                    size: 4,
                    md5: String::new(),
                    url_prefix: Arc::from(""),
                    chunks: vec![],
                })
                .collect(),
        )
    }

    fn purge_now(dir: &Path, plan: &RepairPlan, dry_run: bool) -> u64 {
        let keys = keys_for(plan, &no_blacklist());
        apply_purge(&plan_purge(dir, &keys, &PathScope::default()), dry_run).unwrap()
    }

    /// Compile a scope for the classifier tests.
    fn scope(include: &[&str], exclude: &[&str]) -> PathScope {
        let inc: Vec<String> = include.iter().map(|s| s.to_string()).collect();
        let exc: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
        PathScope::new(&inc, &exc).unwrap()
    }

    fn keyed(rels: &[&str]) -> HashSet<String> {
        rels.iter().map(|r| path_key(r)).collect()
    }

    /// N-5: the walked rel is the **on-disk** spelling, which on Windows can
    /// differ in case from the manifest (and from `server_keep`/`blacklist`).
    /// `plan_purge` keys all three keep sets once; the classifier keys the
    /// walked rel, so the two spellings meet. On Unix they are distinct paths
    /// and the case variant must purge.
    #[test]
    fn the_purge_folds_case_on_windows_only() {
        let open = PathScope::default();
        let on_disk = "game/foo.dat";
        let manifest = "Game/Foo.dat";
        assert_eq!(
            path_key(manifest) == path_key(on_disk),
            cfg!(windows),
            "the purge key folds exactly where Windows paths are case-insensitive"
        );
        let none = HashSet::new();
        for (rule, expected, server_keep, blacklist) in [
            ("expected", keyed(&[manifest]), none.clone(), none.clone()),
            (
                "server_keep",
                none.clone(),
                keyed(&[manifest]),
                none.clone(),
            ),
            ("blacklist", none.clone(), none.clone(), keyed(&[manifest])),
        ] {
            let verdict = classify_purge_path(on_disk, &expected, &server_keep, &blacklist, &open);
            if cfg!(windows) {
                assert_eq!(
                    verdict,
                    PurgeVerdict::Keep,
                    "{rule}: the case variant names the same on-disk file"
                );
            } else {
                assert_eq!(
                    verdict,
                    PurgeVerdict::Purge,
                    "{rule}: on Unix the case variant is a distinct path"
                );
            }
        }
    }

    /// D2: the expected keep set is keyed straight from `plan.files`, so a
    /// plan rel whose spelling differs from the walk only in case still names
    /// the same file on Windows — and a distinct path on Unix, exactly as the
    /// classifier's `path_key` fold says.
    #[test]
    fn plan_purge_keys_the_keep_set_from_the_plan_rels() {
        let dir = TempDir::new("girpr_test_purge", "plan_keyed");
        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::write(dir.join("game/foo.dat"), b"keep").unwrap();

        let pp = plan_purge(
            &dir,
            &keys_for(&plan_of("Game/Foo.dat"), &no_blacklist()),
            &PathScope::default(),
        );
        let rels: Vec<&str> = pp.files.iter().map(|c| c.rel.as_str()).collect();
        if cfg!(windows) {
            assert!(
                rels.is_empty(),
                "the plan rel names the walked file: {rels:?}"
            );
        } else {
            assert_eq!(
                rels,
                vec!["game/foo.dat"],
                "on Unix the case variant is a distinct path"
            );
        }
    }

    /// V-1: a blacklisted path is dropped from the **repair** plan but is
    /// still part of the expected install. The cleanup must keep it even when
    /// the manifest no longer lists it (the "never deleted" claim in docs/01).
    #[test]
    fn a_blacklisted_path_is_kept_by_the_cleanup() {
        let (expected, keep) = purge_fixture();
        let blacklist: HashSet<String> = ["game/foo.dat".to_string()].into_iter().collect();
        let open = PathScope::default();
        assert_eq!(
            classify_purge_path("game/foo.dat", &expected, &keep, &blacklist, &open),
            PurgeVerdict::Keep
        );

        let dir = TempDir::new("girpr_test_purge", "blacklist");
        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::write(dir.join("game/foo.dat"), b"keep").unwrap();
        std::fs::write(dir.join("stale.dat"), b"drop").unwrap();
        // The file is not in this plan: the blacklist is the only keep rule
        // that names it, which is exactly the shape the plan's blacklist test
        // (`build_plan`) produces.
        let pp = plan_purge(&dir, &keys_for(&plan_of("game.dat"), &blacklist), &open);
        let rels: Vec<&str> = pp.files.iter().map(|c| c.rel.as_str()).collect();
        assert_eq!(
            rels,
            vec!["stale.dat"],
            "only the stale file is a candidate"
        );
        apply_purge(&pp, false).unwrap();
        assert!(
            dir.join("game/foo.dat").exists(),
            "blacklisted file survives"
        );
        assert!(!dir.join("stale.dat").exists(), "the stale file is deleted");
    }

    /// The load-bearing case of the scope rule above: a path the scope hides must be
    /// `Keep` **whatever** the other rules would say — a manifest path, a plain
    /// orphan, a temp, and an audio bookkeeping file alike.
    #[test]
    fn a_hidden_path_is_kept_whatever_the_other_rules_say() {
        let (expected, keep) = purge_fixture();
        let scoped = scope(&["game/foo.dat"], &[]);
        for rel in [
            "game.dat",      // rule 1 would keep it anyway
            "login.dat",     // rule "everything else purges"
            "a.dat_tmp",     // temps purge like any orphan
            "audio_lang_14", // rule 3 would keep it anyway
            "ScreenShot/s.png",
            "YuanShen.exe",
        ] {
            assert_eq!(
                classify_purge_path(rel, &expected, &keep, &no_blacklist(), &scoped),
                PurgeVerdict::Keep,
                "{rel} is outside the scope and must not be touched"
            );
        }
        // ...and the one admitted path is classified exactly as unfiltered.
        assert_eq!(
            classify_purge_path("game/foo.dat", &expected, &keep, &no_blacklist(), &scoped),
            PurgeVerdict::Purge
        );
    }

    #[test]
    fn an_exclude_only_scope_also_protects_orphans() {
        let (expected, keep) = purge_fixture();
        let scoped = scope(&[], &["ScreenShot/*"]);
        assert_eq!(
            classify_purge_path(
                "ScreenShot/s.png",
                &expected,
                &keep,
                &no_blacklist(),
                &scoped
            ),
            PurgeVerdict::Keep
        );
        assert_eq!(
            classify_purge_path("login.dat", &expected, &keep, &no_blacklist(), &scoped),
            PurgeVerdict::Purge
        );
    }

    /// A dry pass with no candidates still emits its zero-byte line: it is how
    /// the log distinguishes "the delete step ran and found nothing" from "the
    /// pass never reached the delete step" (the after pass's dedup return).
    #[test]
    fn a_dry_pass_with_no_candidates_still_emits_the_zero_byte_line() {
        let dir = TempDir::new("girpr_test_purge", "dryrun_empty");
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        let plan = plan_of("game.dat");
        let keys = keys_for(&plan, &no_blacklist());
        let pp = plan_purge(&dir, &keys, &PathScope::default());
        assert!(pp.files.is_empty(), "the fixture has no delete candidate");

        let logs = crate::repair::test_support::logs_of(|| {
            assert_eq!(apply_purge(&pp, true).unwrap(), 0);
        });
        assert!(
            logs.contains("purge dry-run: 0 bytes would be freed"),
            "an empty pass that reaches the delete step still says so: {logs}"
        );
    }

    #[test]
    fn purge_dry_run_deletes_nothing_but_reports() {
        let dir = TempDir::new("girpr_test_purge", "dryrun");
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("stray.dat"), b"drop").unwrap();
        let bytes = purge_now(&dir, &plan_of("game.dat"), true);
        assert_eq!(bytes, 0);
        assert!(dir.join("stray.dat").exists());
        assert!(dir.join("game.dat").exists());
    }

    #[test]
    fn purge_real_run_deletes_only_unexpected() {
        let dir = TempDir::new("girpr_test_purge", "real");
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("login.dat"), b"drop").unwrap();
        let bytes = purge_now(&dir, &plan_of("game.dat"), false);
        assert_eq!(bytes, 4);
        assert!(!dir.join("login.dat").exists());
        assert!(dir.join("game.dat").exists());
    }

    /// The delete stage fans syscalls out on scoped threads, but the purge log
    /// lines must still be emitted in plan (sorted-path) order, not completion
    /// order. Enough candidates to populate more than one worker.
    #[test]
    fn parallel_delete_emits_purge_lines_in_plan_order() {
        let dir = TempDir::new("girpr_test_purge", "parallel_order");
        let mut names: Vec<String> = (0..32).map(|i| format!("orphan_{i:02}.dat")).collect();
        for name in &names {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        names.sort();
        let plan = plan_of("game.dat");
        let pp = plan_purge(
            &dir,
            &keys_for(&plan, &no_blacklist()),
            &PathScope::default(),
        );
        assert_eq!(pp.files.len(), names.len());

        let logs = crate::repair::test_support::logs_of(|| {
            assert_eq!(apply_purge(&pp, false).unwrap(), names.len() as u64);
        });
        let mut last = None;
        for name in &names {
            let needle = format!("purged extra {}", dir.join(name).display());
            let at = logs
                .find(&needle)
                .unwrap_or_else(|| panic!("missing purge line for {name}: {logs}"));
            if let Some(prev) = last {
                assert!(prev < at, "{name} logged out of plan order: {logs}");
            }
            last = Some(at);
            assert!(!dir.join(name).exists(), "{name} was not deleted");
        }
    }

    #[test]
    fn purge_removes_emptied_dirs_but_keeps_game_dir() {
        let dir = TempDir::new("girpr_test_purge", "dirs");
        std::fs::create_dir_all(dir.join("emptied/nested")).unwrap();
        std::fs::write(dir.join("emptied/nested/orphan.dat"), b"x").unwrap();
        let plan = plan_of("game.dat");
        purge_now(&dir, &plan, false);
        assert!(!dir.join("emptied").exists(), "emptied dir tree removed");
        assert!(dir.exists(), "game_dir itself survives");
    }

    #[test]
    fn purge_keeps_dirs_that_still_hold_a_manifest_file() {
        let dir = TempDir::new("girpr_test_purge", "kept_dirs");
        std::fs::create_dir_all(dir.join("deep/nested")).unwrap();
        std::fs::write(dir.join("deep/nested/game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("orphan.dat"), b"x").unwrap();
        let plan = plan_of("deep/nested/game.dat");
        let pp = plan_purge(
            &dir,
            &keys_for(&plan, &no_blacklist()),
            &PathScope::default(),
        );
        // `deep` has a kept descendant, so neither it nor `deep/nested` may be
        // swept, even though every sibling of the kept file is being purged.
        assert!(
            !pp.dirs.iter().any(|d| d.ends_with("deep")),
            "dir with a kept file must not be swept: {:?}",
            pp.dirs
        );
        purge_now(&dir, &plan, false);
        assert!(dir.join("deep").exists(), "kept dir survives");
        assert!(dir.join("deep/nested/game.dat").exists());
        assert!(!dir.join("orphan.dat").exists());
    }

    #[test]
    fn plan_purge_totals_bytes_and_deletes_nothing() {
        let dir = TempDir::new("girpr_test_purge", "plan");
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("a.dat"), b"12345").unwrap();
        std::fs::write(dir.join("b.dat"), b"123").unwrap();
        let pp = plan_purge(
            &dir,
            &keys_for(&plan_of("game.dat"), &no_blacklist()),
            &PathScope::default(),
        );
        let rels: Vec<&str> = pp.files.iter().map(|c| c.rel.as_str()).collect();
        assert_eq!(rels, vec!["a.dat", "b.dat"], "sorted by path");
        assert_eq!(pp.total_bytes, 8);
        assert_eq!(pp.hidden, 0, "an open scope hides nothing");
        // Planning alone mutates nothing.
        assert!(dir.join("a.dat").exists());
        assert!(dir.join("b.dat").exists());
    }

    /// One folded [`PurgeKeys`] classifies every keep rule: expected manifest
    /// files (including a directory descendant), the root metadata keeps, a
    /// blacklisted path, the audio bookkeeping, the verification markers, a
    /// hidden path, and the stale download state that still purges.
    ///
    /// Rows are grouped by the rule they pin. Several were consolidated here
    /// from earlier single-purpose classifier tests, keeping their original
    /// path spellings so each regression stays covered. Every row is classified
    /// by `classify_purge_path_visible` — the same entry point the open-scope
    /// `classify_purge_path` helper delegates to (with `visible = true`).
    #[test]
    fn cached_keys_classify_every_keep_rule() {
        let plan = plan_of_all(&["game.dat", "dir/manifest.dat"]);
        let blacklist = keyed(&["mods/blacklisted.dat"]);
        let keys = keys_for(&plan, &blacklist);
        let open = PathScope::default();
        let scoped = scope(&["game.dat"], &[]);
        // Temp spelling from the shared name helper (ex-early temps test).
        let config_temp = crate::util::path::temp_sibling(std::path::Path::new("config.ini"));
        let config_temp = config_temp.to_str().unwrap();

        for (rel, scope, want) in [
            ("game.dat", &open, PurgeVerdict::Keep),
            ("dir/manifest.dat", &open, PurgeVerdict::Keep),
            ("config.ini", &open, PurgeVerdict::Keep),
            (girpr_lock::LOCK_FILE, &open, PurgeVerdict::Keep),
            ("mods/blacklisted.dat", &open, PurgeVerdict::Keep),
            ("audio_lang_14", &open, PurgeVerdict::Keep),
            ("Audio_English_pkg_version", &open, PurgeVerdict::Keep),
            ("stray.dat", &open, PurgeVerdict::Purge),
            ("a.dat_tmp", &open, PurgeVerdict::Purge),
            ("x.hdiff", &open, PurgeVerdict::Purge),
            ("hidden/orphan.dat", &scoped, PurgeVerdict::Keep),
            // Everything-else purges (ex-early manifest/config test).
            ("YuanShen.exe", &open, PurgeVerdict::Purge),
            ("blacklist.txt", &open, PurgeVerdict::Purge),
            ("res_category.txt", &open, PurgeVerdict::Purge),
            ("audio_scan.txt", &open, PurgeVerdict::Purge),
            ("ScreenShot/shot.png", &open, PurgeVerdict::Purge),
            ("log/output.txt", &open, PurgeVerdict::Purge),
            ("login.dat", &open, PurgeVerdict::Purge),
            // Audio pkg-version case-insensitivity (ex-early audio test).
            ("audio_chinese_pkg_version", &open, PurgeVerdict::Keep),
            ("AUDIO_JAPANESE_PKG_VERSION", &open, PurgeVerdict::Keep),
            // Temp spellings still purge (ex-early temps test).
            (config_temp, &open, PurgeVerdict::Purge),
            ("chunk/abc123", &open, PurgeVerdict::Purge),
            ("ldiff/p1", &open, PurgeVerdict::Purge),
            ("staging/y", &open, PurgeVerdict::Purge),
            ("old.diff", &open, PurgeVerdict::Purge),
            ("patch.deletefiles.txt", &open, PurgeVerdict::Purge),
            // Legacy lock kept; a slash-path orphan still purges (ex-early lock test).
            ("stray/orphan.dat", &open, PurgeVerdict::Purge),
            // Rule 2: the launcher's verification list, root-only.
            ("pkg_version", &open, PurgeVerdict::Keep),
            ("mods/pkg_version", &open, PurgeVerdict::Purge),
            // Rule 5: the three dispatcher mirrors and every *_persist marker.
            (
                "GenshinImpact_Data/Persistent/res_versions_persist",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/data_versions_persist",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/silence_data_versions_persist",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/audio_versions_persist",
                &open,
                PurgeVerdict::Keep,
            ),
            // Rule 5: the revision block, including HoYo's "eternal" twin.
            (
                "GenshinImpact_Data/Persistent/res_revision",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/res_revision_eternal",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/data_revision",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/silence_revision",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/audio_revision",
                &open,
                PurgeVerdict::Keep,
            ),
            // Rule 5: plain-text marker names (case-insensitive like rule 4).
            (
                "GenshinImpact_Data/Persistent/ChannelName",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/ScriptVersion",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/PatchDone",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/base_res_version_hash",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "YuanShen_Data/Persistent/channelname",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/audio_lang_14",
                &open,
                PurgeVerdict::Keep,
            ),
            // Rule 5: cache and ctable families.
            (
                "GenshinImpact_Data/Persistent/cache_versions_streaming",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/Persistent/cache_versions",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/StreamingAssets/ctable.dat",
                &open,
                PurgeVerdict::Keep,
            ),
            (
                "GenshinImpact_Data/StreamingAssets/ctable_streaming.dat",
                &open,
                PurgeVerdict::Keep,
            ),
            // Negative: game-generated partial-download state stays purgeable
            // even when its name contains a protected token (docs/01 §6).
            (
                "GenshinImpact_Data/Persistent/res_versions_remote",
                &open,
                PurgeVerdict::Purge,
            ),
            (
                "GenshinImpact_Data/Persistent/data_versions_remote_12",
                &open,
                PurgeVerdict::Purge,
            ),
            (
                "GenshinImpact_Data/Persistent/cache_versions_remote",
                &open,
                PurgeVerdict::Purge,
            ),
            ("VideoAssets/movie.tmp", &open, PurgeVerdict::Purge),
            (
                "VideoAssets/cache_versions_streaming.tmp",
                &open,
                PurgeVerdict::Purge,
            ),
            // Rule 4's audio-style names are no exception to the rejection:
            // an audio_lang temp or a versions_remote sibling stays purgeable.
            ("VideoAssets/audio_lang_14.tmp", &open, PurgeVerdict::Purge),
            (
                "VideoAssets/audio_lang_14_versions_remote",
                &open,
                PurgeVerdict::Purge,
            ),
            // An unrelated file with none of these names still purges.
            ("VideoAssets/unrelated-19.bin", &open, PurgeVerdict::Purge),
        ] {
            assert_eq!(
                classify_purge_path_visible(
                    rel,
                    scope.is_visible(rel),
                    keys.expected(),
                    keys.server_keep(),
                    keys.blacklist(),
                ),
                want,
                "{rel}"
            );
        }
    }

    /// The keys are shared by every purge pass, but the candidate list is not:
    /// each pass walks the tree afresh, so a file that appears between the
    /// before and after passes is seen by the later one.
    #[test]
    fn shared_keys_are_reused_while_each_pass_walks_afresh() {
        let dir = TempDir::new("girpr_test_purge", "cached_passes");
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("stray.dat"), b"drop").unwrap();

        let plan = plan_of("game.dat");
        let keys = keys_for(&plan, &no_blacklist());
        let open = PathScope::default();

        let before = plan_purge(&dir, &keys, &open);
        assert_eq!(
            before
                .files
                .iter()
                .map(|c| c.rel.as_str())
                .collect::<Vec<_>>(),
            vec!["stray.dat"]
        );
        // The same keys over an unchanged tree decide the same thing.
        let after = plan_purge(&dir, &keys, &open);
        assert_eq!(before, after);

        // The walk is per-pass: a late orphan is a candidate on the next pass.
        std::fs::write(dir.join("late.dat"), b"drop").unwrap();
        let later = plan_purge(&dir, &keys, &open);
        assert_eq!(
            later
                .files
                .iter()
                .map(|c| c.rel.as_str())
                .collect::<Vec<_>>(),
            vec!["late.dat", "stray.dat"]
        );
    }

    /// The case a scope has to get right for it not to be destructive: a purge
    /// under a scope must leave the whole fixture on disk, *and* leave its
    /// directories alone — a hidden path must count as "kept", so the
    /// emptied-dir sweep never sweeps the tree it lives in.
    #[test]
    fn a_purge_under_a_scope_does_not_delete_or_sweep_anything_outside_it() {
        let dir = TempDir::new("girpr_test_purge", "scoped");
        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::create_dir_all(dir.join("adopted")).unwrap();
        // Two manifest paths, one admitted and one hidden, plus two hidden
        // orphans. Under `--include game/foo.dat` every one of them survives.
        std::fs::write(dir.join("game/foo.dat"), b"f").unwrap();
        std::fs::write(dir.join("game/bar.dat"), b"b").unwrap();
        std::fs::write(dir.join("adopted/stray.dat"), b"y").unwrap();
        std::fs::write(dir.join("login.dat"), b"z").unwrap();

        let scoped = scope(&["game/foo.dat"], &[]);
        let pp = plan_purge(
            &dir,
            &keys_for(
                &plan_of_all(&["game/foo.dat", "game/bar.dat"]),
                &no_blacklist(),
            ),
            &scoped,
        );
        assert_eq!(pp.total_bytes, 0, "nothing is a delete candidate");
        assert_eq!(
            pp.hidden, 5,
            "bar.dat + stray.dat + login.dat and the game/ + adopted/ dirs hidden"
        );
        apply_purge(&pp, false).unwrap();

        assert_eq!(std::fs::read(dir.join("game/foo.dat")).unwrap(), b"f");
        assert_eq!(
            std::fs::read(dir.join("game/bar.dat")).unwrap(),
            b"b",
            "an out-of-scope *manifest* path is still kept"
        );
        assert!(
            dir.join("adopted").is_dir(),
            "a dir whose only file was hidden must not be swept"
        );
        assert_eq!(std::fs::read(dir.join("adopted/stray.dat")).unwrap(), b"y");
        assert_eq!(std::fs::read(dir.join("login.dat")).unwrap(), b"z");
    }

    #[test]
    fn a_scope_keeps_an_empty_out_of_scope_directory_out_of_the_sweep() {
        // `mark_kept_ancestors` only protects directories reached from a kept
        // *file*. An empty directory has no file to mark it, so an
        // `--include`-hidden empty directory stays out of the sweep.
        let dir = TempDir::new("girpr_test_purge", "scope_empty_dir");
        std::fs::create_dir_all(dir.join("game")).unwrap();
        std::fs::create_dir_all(dir.join("mods/nested")).unwrap();
        std::fs::write(dir.join("game/foo.dat"), b"f").unwrap();

        let scoped = scope(&["game/foo.dat"], &[]);
        let pp = plan_purge(
            &dir,
            &keys_for(&plan_of("game/foo.dat"), &no_blacklist()),
            &scoped,
        );
        assert!(
            pp.dirs.is_empty(),
            "no directory is inside the scope: {:?}",
            pp.dirs
        );
        assert_eq!(
            pp.hidden, 3,
            "game/, mods/ and mods/nested/ are hidden paths"
        );
        apply_purge(&pp, false).unwrap();

        assert!(dir.join("mods").is_dir(), "the hidden dir survives");
        assert!(dir.join("mods/nested").is_dir());
        assert!(dir.join("game/foo.dat").is_file());
    }

    #[test]
    fn a_scope_still_sweeps_a_directory_it_admits() {
        // The control: a directory the scope admits is still swept when the
        // purge empties it. Only hidden paths are protected.
        let dir = TempDir::new("girpr_test_purge", "scope_visible_dir");
        std::fs::create_dir_all(dir.join("game/emptied")).unwrap();
        std::fs::write(dir.join("game/emptied/orphan.dat"), b"x").unwrap();

        let scoped = scope(&["*"], &[]);
        let pp = plan_purge(
            &dir,
            &keys_for(&plan_of("game.dat"), &no_blacklist()),
            &scoped,
        );
        assert_eq!(pp.hidden, 0);
        assert_eq!(pp.total_bytes, 1);
        apply_purge(&pp, false).unwrap();
        assert!(
            !dir.join("game/emptied").exists(),
            "an admitted emptied dir is swept"
        );
    }

    #[test]
    fn a_walk_that_cannot_be_read_is_counted_not_silently_dropped() {
        // An invalid root path (an embedded NUL) is not "not there yet": the
        // walk yields an error rather than a tree. The plan is empty, but it
        // says so instead of reporting a complete, zero-byte delete set. A
        // permission failure takes the same branch; this seam is the one that
        // is deterministic on every platform without an ACL.
        let dir = TempDir::new("girpr_test_purge", "walk_error");
        let pp = plan_purge(
            &dir.join("bad\0name"),
            &keys_for(&plan_of("game.dat"), &no_blacklist()),
            &PathScope::default(),
        );
        assert!(pp.files.is_empty(), "{:?}", pp.files);
        assert_eq!(pp.total_bytes, 0);
        assert!(pp.errors >= 1, "the unreadable walk is counted");

        // N-19: the same real counted failure refuses a delete ceiling instead
        // of being compared as the complete delete set.
        let ceiling = Limits {
            delete_bytes: Some(u64::MAX),
            ..Default::default()
        };
        let plan = plan_of("game.dat");
        let hits = ceiling.check_preflight(Preflight {
            plan: &plan,
            est_download_bytes: 0,
            purge: Some(&pp),
            persistent_files: 0,
            persistent_bytes: 0,
        });
        assert!(
            matches!(hits.as_slice(), [LimitHit::IncompletePurge { .. }]),
            "the walk failure must refuse the ceiling: {hits:?}"
        );
        assert!(ceiling.check_delete(&pp).is_some());
        // Without a ceiling the plan stays diagnostic, exactly as before.
        assert!(Limits::default().check_delete(&pp).is_none());
    }

    #[test]
    fn a_missing_game_dir_is_an_empty_plan_and_not_an_error() {
        // The documented case: `--print-plan` before `game_dir` exists. It is
        // an empty plan, not a walk failure.
        let dir = TempDir::new("girpr_test_purge", "missing_root");
        let pp = plan_purge(
            &dir.join("not_there_yet"),
            &keys_for(&plan_of("game.dat"), &no_blacklist()),
            &PathScope::default(),
        );
        assert!(pp.files.is_empty());
        assert_eq!(pp.total_bytes, 0);
        assert_eq!(pp.errors, 0);
    }

    #[test]
    fn a_scope_that_admits_an_orphan_still_deletes_it() {
        let dir = TempDir::new("girpr_test_purge", "scope_purge");
        std::fs::write(dir.join("game.dat"), b"keep").unwrap();
        std::fs::write(dir.join("stray.dat"), b"drop").unwrap();
        // `--include "*.dat"` admits the orphan; it is still not in the
        // manifest, so it purges.
        let scoped = scope(&["*.dat"], &[]);
        let pp = plan_purge(
            &dir,
            &keys_for(&plan_of("game.dat"), &no_blacklist()),
            &scoped,
        );
        assert_eq!(pp.total_bytes, 4);
        assert_eq!(pp.hidden, 0);
        assert_eq!(apply_purge(&pp, false).unwrap(), 4);
        assert!(!dir.join("stray.dat").exists());
        assert!(dir.join("game.dat").exists());
    }

    #[test]
    fn dry_run_leaves_dirs_alone() {
        let dir = TempDir::new("girpr_test_purge", "dirs_dry");
        std::fs::create_dir_all(dir.join("empty_dir")).unwrap();
        std::fs::write(dir.join("orphan.dat"), b"x").unwrap();
        let plan = plan_of("game.dat");
        purge_now(&dir, &plan, true);
        assert!(
            dir.join("empty_dir").exists(),
            "dry-run must not sweep dirs"
        );
    }

    #[test]
    fn the_keep_set_is_config_the_lock_and_root_pkg_version() {
        // The legacy lock file lives at the game dir's root and is not in the
        // manifest: the cleanup must not eat a leftover. This run's own lock is
        // outside the install. `pkg_version` is the launcher's verification
        // list and is kept for every run, not only persistent ones.
        let keep = keep_set();
        assert!(keep.contains("config.ini"));
        assert!(keep.contains(girpr_lock::LOCK_FILE));
        assert!(keep.contains("pkg_version"));
        assert_eq!(keep.len(), 3);
    }

    /// The metadata keeps are keyed by the full rel path, so they match only at
    /// the game dir's root. A lock-, config- or pkg_version-shaped file in a
    /// subdirectory is not the root file and is an ordinary delete candidate.
    /// Pinning this keeps a future "helpful" widened match from silently
    /// sparing an install's worth of nested files.
    #[test]
    fn the_root_keep_rules_are_root_only() {
        let plan = plan_of("game.dat");
        let keys = keys_for(&plan, &no_blacklist());
        for rel in ["config.ini", girpr_lock::LOCK_FILE, "pkg_version"] {
            assert_eq!(
                classify_purge_path_visible(
                    rel,
                    true,
                    keys.expected(),
                    keys.server_keep(),
                    keys.blacklist(),
                ),
                PurgeVerdict::Keep,
                "{rel} at the root is kept"
            );
            let nested = format!("nested/{rel}");
            assert_eq!(
                classify_purge_path_visible(
                    &nested,
                    true,
                    keys.expected(),
                    keys.server_keep(),
                    keys.blacklist(),
                ),
                PurgeVerdict::Purge,
                "{nested} is not the root-level metadata file"
            );
        }
    }

    /// The sweep's expected failure — something appeared after the plan was
    /// built — must surface its reason rather than being discarded like a
    /// sharing violation or an ACL.
    #[test]
    fn a_dir_the_sweep_could_not_remove_is_logged_with_the_reason() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_purge", "sweep");
        let d = dir.join("emptied");
        std::fs::create_dir_all(&d).unwrap();
        // Non-empty: `remove_dir` fails, which is the failure the sweep meets.
        std::fs::write(d.join("appeared.dat"), b"raced").unwrap();

        let logs =
            crate::repair::test_support::logs_of(|| remove_emptied_dirs(std::slice::from_ref(&d)));

        assert!(d.is_dir(), "the sweep does not force what it cannot remove");
        assert!(
            logs.contains("the emptied dir was not removed")
                && logs.contains(&d.display().to_string()),
            "the directory and the reason are logged: {logs}"
        );
    }

    /// The finish step sorts the delete list by path, keeps only directories
    /// no kept file reaches, totals the bytes, and carries the walk's hidden and
    /// error counts through unchanged.
    #[test]
    fn the_finish_step_sorts_and_totals_the_walk() {
        let dir = TempDir::new("girpr_test_purge", "finish");
        let a = dir.join("a.dat");
        let b = dir.join("b.dat");
        let nested = dir.join("nested");
        let kept = dir.join("kept");
        let walk = WalkResult {
            files: vec![
                PurgeCandidate {
                    rel: "b.dat".into(),
                    path: b,
                    bytes: 3,
                },
                PurgeCandidate {
                    rel: "a.dat".into(),
                    path: a,
                    bytes: 5,
                },
            ],
            all_dirs: vec![nested.clone(), kept.clone()],
            kept_dirs: [kept].into_iter().collect(),
            hidden: 2,
            errors: 1,
        };

        let pp = finish_plan(&dir, walk);
        let rels: Vec<&str> = pp.files.iter().map(|c| c.rel.as_str()).collect();
        assert_eq!(rels, vec!["a.dat", "b.dat"], "sorted by path");
        assert_eq!(pp.total_bytes, 8);
        assert_eq!(pp.dirs, vec![nested], "a kept file protects its dir");
        assert_eq!(pp.hidden, 2);
        assert_eq!(pp.errors, 1);
    }
}
