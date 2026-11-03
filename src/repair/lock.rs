//! The single-writer lock.
//!
//! Two `girpr` runs against one install can write conflicting bytes into the
//! same file — different `--audio` or `--include` plans, or a `discard` that
//! deletes a file the other run has open — and neither run can know about the
//! other.
//!
//! The lock is an **OS advisory lock** on a file outside the install:
//! `{temp}/girpr-{key:016x}.lock`, where `key` is an FNV-1a hash of the
//! canonical game directory (ASCII case folded on Windows, because the
//! filesystem is). Every run that mutates the install takes it, regardless of
//! the path scope, so a scoped run cannot write beside an unscoped one.
//!
//! The open handle holds the lock: the OS releases it when the process dies,
//! so a killed run never leaves a wall-clock wait behind. The file body names
//! the pid and the time it was taken, so a refusal can usually name the run to
//! look at. An exclusive lock can hide that body on some platforms; the
//! refusal then says the body is unreadable instead of inventing a pid. A
//! leftover file with no holder is harmless: the next `try_lock`
//! succeeds and overwrites its body. `Drop` only releases the handle; it never
//! removes the file, because deleting it after unlocking would let a second and
//! a third run hold two different files for the same key at once.
//!
//! Read-only runs (`--check-only`, `--dry-run`, `--print-plan`) take no lock.
//!
//! Older runs wrote `{game_dir}/girpr.lock`; this one never creates it. If
//! one is left over it is left alone (and kept by the cleanup) rather than
//! deleted behind the user's back.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The in-install lock file older runs created. Kept only so the cleanup
/// recognizes and preserves it; this module never creates it.
pub const LOCK_FILE: &str = "girpr.lock";

/// FNV-1a: small, stable, and specified — unlike `DefaultHasher`, whose
/// algorithm is not promised across releases.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A held lock. The open handle is what excludes; dropping it unlocks.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    file: File,
}

impl Lock {
    /// Take the lock, or say why it could not be taken.
    ///
    /// The refusal names the file and the holder from its body, because the
    /// person hitting it wants to know *which* run to look at. An exclusive
    /// lock can only be held by a live process, so there is no staleness
    /// rule: the OS releases a dead owner's handle.
    pub fn acquire(game_dir: &Path) -> Result<Lock> {
        Self::acquire_at(lock_path(game_dir))
    }

    /// The implementation, with the path injected so tests can take two
    /// locks for one key without depending on the global temp directory.
    fn acquire_at(path: PathBuf) -> Result<Lock> {
        let (mut file, created) =
            open_lock_file(&path).with_context(|| format!("open lock file {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(held(&path)),
            Err(TryLockError::Error(e)) => {
                return Err(e).with_context(|| format!("lock {}", path.display()));
            }
        }
        // The contents are for a human: which process holds it, and since
        // when. A lock whose body cannot be written is still a lock — the
        // open handle and its OS lock are what exclude. A file we did not
        // create is only rewritten when its body is already one of ours; an
        // arbitrary file (or a symlink target) is never truncated.
        if may_write_body(&path, &mut file, created) {
            let body = format!(
                "pid={}\nstarted={}\n",
                std::process::id(),
                chrono::Utc::now().to_rfc3339()
            );
            if file.set_len(0).is_ok() && file.seek(SeekFrom::Start(0)).is_ok() {
                let _ = file.write_all(body.as_bytes());
            }
        }
        Ok(Lock { path, file })
    }

    /// The lock file's path (in the temp directory, keyed by the game dir).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Does a run with these read-only flags mutate the install and therefore
/// need the lock?
///
/// `check_only` is already part of `readonly`, but stating it keeps the
/// contract explicit. `--purge-only` is not read-only and does take the lock.
pub(crate) fn takes_lock(readonly: bool, check_only: bool, print_plan: bool) -> bool {
    !readonly && !check_only && !print_plan
}

/// Where the lock for `game_dir` lives: the temp directory, keyed by the
/// canonical path so every spelling of the same install agrees.
fn lock_path(game_dir: &Path) -> PathBuf {
    let canonical = game_dir
        .canonicalize()
        .unwrap_or_else(|_| game_dir.to_path_buf());
    std::env::temp_dir().join(format!("girpr-{:016x}.lock", key_for(&canonical)))
}

/// A stable key for a canonical path.
///
/// The lossy spelling keeps every key for a valid UTF-8 path exactly as it was,
/// which matters because the key names a lock file that may already exist.
/// `to_string_lossy` replaces invalid bytes and unpaired surrogates, though, so
/// distinct non-Unicode paths can share a spelling. For those paths only, the
/// raw encoding is mixed in after a separator: apart from ASCII case it is
/// injective, so the keys differ, while valid UTF-8 paths keep their old key.
fn key_for(path: &Path) -> u64 {
    let mut key = FNV_OFFSET;
    for byte in path.as_os_str().to_string_lossy().bytes() {
        key = fnv_step(key, byte);
    }
    if path.to_str().is_none() {
        key = fnv_step(key, 0xff);
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            for &byte in path.as_os_str().as_bytes() {
                key = fnv_mix(key, byte);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            for unit in path.as_os_str().encode_wide() {
                // Fold a whole ASCII code unit, not each byte of the UTF-16
                // encoding: folding bytes would merge non-ASCII units whose
                // low or high byte happens to spell an ASCII letter.
                let unit = if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
                    unit + (b'a' - b'A') as u16
                } else {
                    unit
                };
                key = fnv_mix(key, (unit & 0xff) as u8);
                key = fnv_mix(key, (unit >> 8) as u8);
            }
        }
    }
    key
}

/// One FNV-1a step, with ASCII case folded on Windows because its filesystem
/// is case-insensitive for ASCII.
fn fnv_step(key: u64, byte: u8) -> u64 {
    #[cfg(windows)]
    let byte = byte.to_ascii_lowercase();
    fnv_mix(key, byte)
}

/// One FNV-1a step with no folding, for input that has already been folded.
fn fnv_mix(key: u64, byte: u8) -> u64 {
    (key ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
}

impl Drop for Lock {
    fn drop(&mut self) {
        // The OS handle is the exclusion; releasing it lets the next run in.
        // The file is deliberately left behind. Removing it after unlocking
        // would race a run that opens the same path between our unlock and our
        // delete: it holds the old file while a third run creates a new one,
        // and both then think they are the single writer. A leftover unlocked
        // file is harmless because the next `try_lock` succeeds.
        let _ = self.file.unlock();
    }
}

/// Open the lock file without creating or truncating anything unexpected.
///
/// A fresh file is claimed with `create_new`, which refuses to follow a
/// dangling symlink into a new target. An existing path is opened read/write
/// without create, so a pre-created symlink cannot make us create its target,
/// and nothing is truncated at open time. The bool reports whether this call
/// created the file, the only case where the body is certainly ours.
fn open_lock_file(path: &Path) -> io::Result<(File, bool)> {
    // Bounded: an existing file may vanish between the two opens, but only a
    // hostile or broken process keeps doing that.
    for _ in 0..3 {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(file) => return Ok((file, true)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                match OpenOptions::new().read(true).write(true).open(path) {
                    Ok(file) => return Ok((file, false)),
                    // The path still exists (for example a dangling symlink)
                    // but cannot be opened without creating its target.
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        if std::fs::symlink_metadata(path).is_ok() {
                            return Err(e);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("lock file kept changing while opening it"))
}

/// May this run rewrite the body of the lock file?
///
/// A file we created is ours. An existing file is only ours if it is a regular
/// file (not a symlink or other special file, which opening may have followed)
/// and already carries a lock body, or is empty. Anything else is left exactly
/// as it is: the OS lock still excludes, and a refusal can report an
/// unreadable body without having damaged a user file.
fn may_write_body(path: &Path, file: &mut File, created: bool) -> bool {
    if created {
        return true;
    }
    let regular = std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_file())
        .unwrap_or(false);
    if !regular {
        return false;
    }
    if file.seek(SeekFrom::Start(0)).is_err() {
        return false;
    }
    let mut head = [0u8; 8];
    let Ok(read) = file.read(&mut head) else {
        return false;
    };
    read == 0 || head[..read].starts_with(b"pid=")
}

/// The refusal for a lock that a live process holds.
fn held(path: &Path) -> anyhow::Error {
    match std::fs::read_to_string(path) {
        Ok(body) => {
            let pid = field(&body, "pid").unwrap_or("?");
            match field(&body, "started") {
                Some(started) => anyhow::anyhow!(
                    "another girpr run holds {} (pid {}, started {}); refusing to run",
                    path.display(),
                    pid,
                    started
                ),
                None => anyhow::anyhow!(
                    "another girpr run holds {} (pid {}); refusing to run",
                    path.display(),
                    pid
                ),
            }
        }
        Err(_) => anyhow::anyhow!(
            "another girpr run holds {} (its body is unreadable, possibly because \
             an exclusive lock hides it); refusing to run",
            path.display()
        ),
    }
}

/// One `name=value` field from a lock body.
fn field<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    body.lines().find_map(|line| {
        line.strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('='))
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_lock_excludes_a_second_run() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "fresh");
        let lock = Lock::acquire(&dir).expect("the first run takes the lock");
        assert!(lock.path().exists());
        // The lock lives outside the install, keyed by its canonical path.
        assert_eq!(lock.path().parent(), Some(std::env::temp_dir().as_path()));
        let name = lock.path().file_name().unwrap().to_string_lossy();
        assert!(
            name.starts_with("girpr-") && name.ends_with(".lock"),
            "{name}"
        );
        // The second run is refused, and the refusal names the file.
        let err = Lock::acquire(&dir).expect_err("the second run is refused");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(lock.path().to_string_lossy().as_ref()),
            "{msg}"
        );
        // An exclusive OS lock can hide the body from another process (on
        // Windows it does); the refusal says so rather than inventing a pid.
        assert!(
            msg.contains(&std::process::id().to_string()) || msg.contains("body is unreadable"),
            "{msg}"
        );
        // The lock is released when it is dropped, and the file is left for
        // the next run to reuse rather than unlinked under a racing opener.
        let path = lock.path().to_path_buf();
        drop(lock);
        assert!(
            path.exists(),
            "the unlocked file is left in place, not removed"
        );
        // And the next run takes it.
        let lock = Lock::acquire(&dir).expect("the lock is free again");
        assert_eq!(lock.path(), path);
    }

    #[test]
    fn a_junk_body_names_an_unknown_holder() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "body");
        let path = lock_path(&dir);
        std::fs::write(&path, "not a pid at all").unwrap();
        let msg = format!("{:#}", held(&path));
        assert!(msg.contains("pid ?"), "an unknown holder is named: {msg}");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn an_unreadable_body_says_so() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "unreadable");
        let path = lock_path(&dir);
        let msg = format!("{:#}", held(&path));
        // The body may be hidden while the holder owns the file; the refusal
        // must say that rather than claim a pid it cannot read.
        assert!(msg.contains("body is unreadable"), "{msg}");
        assert!(msg.contains("an exclusive lock hides it"), "{msg}");
        assert!(msg.contains("refusing to run"), "{msg}");
        assert!(msg.contains(path.to_string_lossy().as_ref()), "{msg}");
    }

    #[test]
    fn different_game_dirs_do_not_conflict() {
        let one = girpr_tempdir::TempDir::new("girpr_test_lock", "dir_one");
        let two = girpr_tempdir::TempDir::new("girpr_test_lock", "dir_two");
        let first = Lock::acquire(&one).expect("the first install is locked");
        let second = Lock::acquire(&two).expect("another install is independent");
        assert_ne!(first.path(), second.path());
    }

    #[test]
    fn dropping_releases_the_lock_and_keeps_the_file() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "release");
        let path = {
            let lock = Lock::acquire(&dir).expect("the lock is taken");
            lock.path().to_path_buf()
        };
        assert!(
            path.exists(),
            "the file is left behind so an opener cannot be unlinked"
        );
        let _again = Lock::acquire(&dir).expect("the OS released the lock");
    }

    #[test]
    fn a_file_left_by_a_dead_run_does_not_block() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "dead");
        // A killed run leaves its body behind, but the OS has released the
        // handle: there is no mtime rule and no wall-clock wait.
        let path = lock_path(&dir);
        std::fs::write(&path, "pid=1\nstarted=2000-01-01T00:00:00Z\n").unwrap();
        let lock = Lock::acquire(&dir).expect("an unlocked leftover is harmless");
        assert_eq!(lock.path(), path);
        // The stale body is replaced by the live holder's.
        drop(lock);
        let body = std::fs::read_to_string(&path).expect("the body is readable again");
        assert!(
            body.contains(&format!("pid={}", std::process::id())),
            "{body}"
        );
    }

    #[test]
    fn a_preexisting_foreign_file_is_not_truncated() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "foreign");
        let path = lock_path(&dir);
        std::fs::write(&path, "this is not a lock body").unwrap();
        let lock = Lock::acquire(&dir).expect("the path can still be locked");
        assert_eq!(lock.path(), path);
        drop(lock);
        // The file is still ours to lock, but its contents were left alone.
        assert_eq!(
            std::fs::read(&path).expect("the file is still there"),
            b"this is not a lock body"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_preexisting_symlink_target_is_not_truncated() {
        use std::os::unix::fs::symlink;

        let dir = girpr_tempdir::TempDir::new("girpr_test_lock", "symlink");
        let target = dir.join("user-written.txt");
        std::fs::write(&target, "user data").unwrap();
        let path = lock_path(&dir);
        symlink(&target, &path).expect("the symlink is created");
        let lock = Lock::acquire(&dir).expect("the existing target can be locked");
        assert_eq!(lock.path(), path);
        drop(lock);
        assert_eq!(
            std::fs::read(&target).expect("the target is untouched"),
            b"user data"
        );
    }

    #[test]
    fn lock_policy_covers_every_run_mode() {
        // A mutating run, including `--purge-only`: takes the lock.
        assert!(takes_lock(false, false, false));
        // `--dry-run` (readonly) and `--check-only`: no lock.
        assert!(!takes_lock(true, false, false));
        assert!(!takes_lock(false, true, false));
        assert!(!takes_lock(true, true, false));
        // `--print-plan`: no lock.
        assert!(!takes_lock(false, false, true));
    }

    #[cfg(windows)]
    #[test]
    fn the_key_folds_ascii_case_on_windows() {
        assert_eq!(
            key_for(Path::new(r"C:\Game")),
            key_for(Path::new(r"c:\game"))
        );
        assert_ne!(
            key_for(Path::new(r"C:\Game")),
            key_for(Path::new(r"D:\Game"))
        );
    }

    /// The hash of the lossy spelling alone: the key every valid UTF-8 path
    /// had before raw bytes were mixed in for non-Unicode paths.
    fn lossy_key(path: &Path) -> u64 {
        let mut key = FNV_OFFSET;
        for byte in path.as_os_str().to_string_lossy().bytes() {
            key = fnv_step(key, byte);
        }
        key
    }

    #[test]
    fn utf8_paths_keep_their_existing_key() {
        for path in [
            Path::new("/tmp/girpr game"),
            Path::new("/tmp/jeu d'ete"),
            Path::new(r"C:\Games\Jeu d'ete"),
        ] {
            assert_eq!(
                key_for(path),
                lossy_key(path),
                "a valid UTF-8 path keeps its key: {}",
                path.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn distinct_non_utf8_paths_do_not_share_a_key() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let one = Path::new(OsStr::from_bytes(b"/tmp/game-\x80"));
        let two = Path::new(OsStr::from_bytes(b"/tmp/game-\x81"));
        assert_eq!(one.to_string_lossy(), two.to_string_lossy());
        assert_ne!(key_for(one), key_for(two));
    }

    #[cfg(windows)]
    #[test]
    fn distinct_non_utf16_paths_do_not_share_a_key() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        let spelled = |base: &str, unit: u16| {
            let mut wide: Vec<u16> = base.encode_utf16().collect();
            wide.push(unit);
            PathBuf::from(OsString::from_wide(&wide))
        };
        let one = spelled(r"C:\game", 0xd800);
        let two = spelled(r"C:\game", 0xd801);
        assert_eq!(one.to_string_lossy(), two.to_string_lossy());
        assert_ne!(key_for(&one), key_for(&two));
        // Surrogate units whose low byte is an ASCII letter must not be folded
        // together: only a whole ASCII code unit is case-insensitive.
        let three = spelled(r"C:\game", 0xd841);
        let four = spelled(r"C:\game", 0xd861);
        assert_eq!(three.to_string_lossy(), four.to_string_lossy());
        assert_ne!(key_for(&three), key_for(&four));
        // A non-Unicode path still folds the ASCII spelling of its valid part,
        // so one install reached by two case spellings keeps one key.
        assert_eq!(
            key_for(&spelled(r"C:\GAME", 0xd800)),
            key_for(&spelled(r"c:\game", 0xd800))
        );
    }
}
