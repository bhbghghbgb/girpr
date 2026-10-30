//! A scratch directory for one test: named so nothing can collide with it,
//! and removed when the test ends however it ends.
//!
//! One crate, three projects: the repair binary (`girpr`), the capture tool
//! (`gircap`) and the mirror (`girsync`) share this helper — one name, one
//! construction, one cleanup, for all three.
//!
//! It is a **dev-dependency** of every project that uses it. Nothing that
//! ships links against it: a scratch directory is a thing for a test, not for
//! a program.
//!
//! ## Why the name carries so much
//!
//! `cargo nextest run` runs **each test in its own process**, and CI (or a
//! developer) routinely runs two copies of the suite at once. A fixed
//! directory name is therefore shared by more than one test *and* by more than
//! one process, and both failure modes are silent:
//!
//! - one test's `remove_dir_all` deletes the tree another test is halfway
//!   through writing, which fails intermittently and reads like a bug in the
//!   code under test;
//! - a directory left behind by a run that was killed mid-test is inherited by
//!   the next run, so the second run asserts against the first run's
//!   leftovers.
//!
//! The second one is not hypothetical: a `temp_dir` the tests removed by hand
//! on the happy path and not at all when a case panicked was found leaving
//! one directory per run behind in the system temp dir.
//!
//! So the name carries the process id, a wall-clock timestamp and a
//! per-process counter — the three things that, together, separate two tests
//! in two processes at two times — and [`Drop`] removes the tree, so cleanup
//! does not depend on the test reaching its last line.
//!
//! ## One root, so a leftover is recognisable
//!
//! Every scratch tree this workspace makes lives under **one** directory:
//! `<temp>/girpr-test/`. Not directly in the system temp directory, where it
//! would be one anonymous entry among a thousand that belong to other
//! programs, but under a name that says whose it is.
//!
//! The reason is the question a leftover raises. When a directory survives a
//! run that was killed mid-test, somebody has to decide whether it is this
//! project's to remove or somebody else's to leave alone. In the system temp
//! directory that question has no answer — the name is all there is, and it
//! does not say. Under `girpr-test/` it does: the whole directory is this
//! project's, and everything in it is either live or a leftover of a run that
//! did not finish.
//!
//! It also makes the cost of a leak visible. "What did this project leave in
//! the temp directory" becomes "what is under `girpr-test/`", instead of a
//! question about the history of a machine.
//!
//! ## Using it
//!
//! ```no_run
//! use girpr_tempdir::TempDir;
//!
//! let dir = TempDir::new("girpr_example", "one_case");
//! let file = dir.join("fixture.bin");
//! std::fs::write(&file, b"x").unwrap();
//! // `dir` is removed here, however the test ends.
//! ```
//!
//! The example is also **run**, as an ordinary test: `cargo nextest` has no
//! doctest support, so a doctest alone would leave the public surface this
//! example uses checked by nothing. See `the_documented_usage_writes_into_the
//! _scratch_directory` below.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The one directory every scratch tree of this workspace lives under.
///
/// `<temp>/girpr-test/`: a name that says whose the contents are, so a
/// directory left behind by a killed run is recognisable as this project's
/// and not as somebody else's.
pub const ROOT: &str = "girpr-test";

/// The path of the shared root: `<temp>/girpr-test`.
///
/// The root itself is **not** created here. [`TempDir::new`] creates it on the
/// way down, and a caller who only ever asks for an absent path has no reason
/// to leave a directory behind.
pub fn root() -> PathBuf {
    std::env::temp_dir().join(ROOT)
}

/// How many directories this process has handed out. The pid and the timestamp
/// already make the name unique across processes and across runs; this is what
/// separates two directories created *within* one test, where both would
/// otherwise land in the same nanosecond.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// A path under the system temp dir that no other test, and no other run, will
/// use: `<temp>/girpr-test/<prefix>-<tag>-<pid>-<nanos>-<seq>`.
///
/// Every part earns its place. `prefix` says which suite leaked a directory if
/// one survives a crash, `tag` names the test that wanted it, `pid` separates
/// two suites running at once, `nanos` separates two runs of the same binary
/// (a pid is reused, and a killed run leaves its tree behind), and `seq`
/// separates two directories made inside one test.
///
/// The name is a contract: **exactly five parts, split on `-`**. A `prefix` or
/// a `tag` that carries a dash would break the shape, so the dash is folded
/// to an underscore here rather than left to the caller.
///
/// The directory is **not** created: a test whose subject is what the code does
/// when a path is absent needs it that way.
fn unique_path(prefix: &str, tag: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the system clock is before 1970")
        .as_nanos();
    let prefix = prefix.replace('-', "_");
    let tag = tag.replace('-', "_");
    root().join(format!(
        "{prefix}-{tag}-{}-{nanos}-{seq}",
        std::process::id()
    ))
}

/// A scratch directory, removed when the value goes out of scope.
///
/// Every one of them lives under [`root()`] — `<temp>/girpr-test/` — so that a
/// directory left behind by a killed run is recognisable as this project's.
///
/// Derefs to [`Path`] and implements `AsRef<Path>`, so `dir.join(..)`, `&dir`
/// where a `&Path` is wanted, and every other path operation work without a
/// conversion at the call site.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// An empty directory, created.
    ///
    /// The path is unique by construction — pid, timestamp and per-process
    /// counter — so there is nothing at it to remove: the directory is
    /// **created, not cleared**. (A "remove first" on a path another run may
    /// already own is the exact failure this crate's name exists to prevent.)
    ///
    /// The shared root is created along with it, so a caller never has to make
    /// `<temp>/girpr-test` itself.
    pub fn new(prefix: &str, tag: &str) -> Self {
        let path = unique_path(prefix, tag);
        std::fs::create_dir_all(&path).expect("create the temp dir");
        Self { path }
    }

    /// A path that does not exist yet.
    ///
    /// For the tests whose subject is what the code under test does when the
    /// path is *absent* — a refused run must not create one, an `emit` case
    /// that must create its own parents, or a fixture written by the test must
    /// not find one already there.
    pub fn absent(prefix: &str, tag: &str) -> Self {
        Self {
            path: unique_path(prefix, tag),
        }
    }

    /// A guard over a path the *caller* chose.
    ///
    /// For the few cases where the path is not free: a `materialize` run that
    /// defaults to writing *beside* the bundle, so the assertion can only be
    /// made against the sibling the tool itself picks. Build the path the way
    /// the code under test builds it — a sibling of another scratch tree, say
    /// — and the guard owns it from there.
    ///
    /// **Anything already at the path is removed first**, whether it is a
    /// directory or a file. The directory is **not** created: the caller chose
    /// this path precisely because the code under test picks it, and making it
    /// is the code under test's job.
    pub fn at(path: PathBuf) -> Self {
        // `remove_dir_all` refuses a file and `remove_file` refuses a
        // directory, so both are tried: the promise is that the path is
        // clear, not that one of the two shapes was there.
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&path);
        Self { path }
    }

    /// `rel` under this root, with its parents created.
    pub fn subdir(&self, rel: &str) -> PathBuf {
        let p = self.path.join(rel);
        std::fs::create_dir_all(&p).expect("create the temp subdir");
        p
    }

    /// The path, cloned, for the call sites that hand an owned `PathBuf` to
    /// the binary under test or to a struct that outlives this guard.
    pub fn to_path_buf(&self) -> PathBuf {
        self.path.clone()
    }
}

impl AsRef<Path> for TempDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Best effort: on Windows a file the test left open refuses the delete
        // (a log sink, a cache handle, a blob), and a leftover in the temp dir
        // is a far smaller problem than a test that cannot report its own
        // failure over it.
        //
        // Both shapes are removed, for the reason `at` removes both: the path
        // is a tree in every case but one, and a file in that one — the path
        // the code under test picked and wrote to. Each call refuses the
        // other's shape, and the refusal is swallowed either way.
        let _ = std::fs::remove_dir_all(&self.path);
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The example in the module docs, **run**.
    ///
    /// `cargo nextest` — the runner this workspace uses — does not support
    /// doctests at all, so the example above would be checked by nothing: the
    /// suite would stay green against a `TempDir` that no longer took those
    /// arguments, and the documentation would be the only place left claiming
    /// otherwise. Mirroring the example here is what keeps it honest under the
    /// runner that is actually used.
    #[test]
    fn the_documented_usage_writes_into_the_scratch_directory() {
        let dir = TempDir::new("girpr_tempdir_example", "one_case");
        let file = dir.join("fixture.bin");
        std::fs::write(&file, b"x").expect("write the fixture");
        assert_eq!(std::fs::read(&file).expect("read the fixture"), b"x");
        assert!(dir.as_ref().is_dir(), "the scratch root is a directory");
    }

    /// Two directories made in the same process carry different names, even
    /// when the pid and the clock are the same.
    #[test]
    fn two_directories_never_share_a_name() {
        let a = TempDir::new("girpr_tempdir_test", "same_tag");
        let b = TempDir::new("girpr_tempdir_test", "same_tag");
        assert_ne!(a.path, b.path);
        assert!(a.path.is_dir());
        assert!(b.path.is_dir());
    }

    /// The name says which suite and which test asked for the directory, and
    /// it carries the process id — the three parts a reader needs to work out
    /// whose leftover a surviving directory is.
    #[test]
    fn the_name_carries_the_prefix_the_tag_and_the_pid() {
        let path = unique_path("girpr_tempdir_test", "a_tag");
        let name = path
            .file_name()
            .expect("the path has a last component")
            .to_string_lossy()
            .into_owned();
        // `<prefix>-<tag>-<pid>-<nanos>-<seq>`, and nothing else.
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 5, "the name is {name}");
        assert_eq!(parts[0], "girpr_tempdir_test");
        assert_eq!(parts[1], "a_tag");
        assert_eq!(parts[2], std::process::id().to_string());
        assert!(parts[3].parse::<u128>().is_ok(), "the name is {name}");
        assert!(parts[4].parse::<u64>().is_ok(), "the name is {name}");
    }

    /// Every scratch tree of this workspace lives under **one** directory in
    /// the system temp directory: `<temp>/girpr-test`.
    ///
    /// The point is recognisability. A directory left behind by a killed run
    /// has to be attributable — "whose leftover is this?" — and in the system
    /// temp directory the name is all the answer there is. Under a root named
    /// for the project the answer is the directory it sits in.
    #[test]
    fn every_scratch_tree_lives_under_this_projects_own_root() {
        let dir = TempDir::new("girpr_tempdir_test", "under_the_root");
        assert_eq!(
            dir.parent(),
            Some(root().as_path()),
            "the scratch tree is a child of <temp>/{ROOT}"
        );
        assert_eq!(
            dir.parent()
                .and_then(Path::file_name)
                .and_then(|s| s.to_str()),
            Some(ROOT),
            "and the root says whose it is"
        );
        assert!(
            root().starts_with(std::env::temp_dir()),
            "the root is in the system temp directory: {}",
            root().display()
        );
    }

    /// The root is there when a scratch tree has been made.
    ///
    /// Not "the root was created by this call": another test in another
    /// process may have made it already, and a test that removed the root to
    /// find out would delete the tree some other test is halfway through
    /// writing. The promise worth pinning is the one a caller can rely on —
    /// after `TempDir::new`, the root exists.
    #[test]
    fn the_root_exists_once_a_scratch_tree_has_been_made() {
        let dir = TempDir::new("girpr_tempdir_test", "makes_the_root");
        assert!(root().is_dir(), "the root is there");
        assert!(dir.is_dir(), "and so is the tree under it");
    }

    /// The five-part shape is the **code's** promise, not the caller's: a
    /// prefix or a tag that carries a dash has it folded to an underscore, so
    /// the name is still five parts.
    ///
    /// This matters because one caller builds its tag from data — a table row's
    /// name — and a row name is a sentence, so a dash in it is a matter of
    /// time rather than a matter of luck.
    #[test]
    fn a_dash_in_the_prefix_or_the_tag_cannot_break_the_shape() {
        let path = unique_path("girpr-tempdir-test", "a-tag");
        let name = path
            .file_name()
            .expect("the path has a last component")
            .to_string_lossy()
            .into_owned();
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 5, "the name is {name}");
        assert_eq!(parts[0], "girpr_tempdir_test");
        assert_eq!(parts[1], "a_tag");
    }

    /// An `absent` directory is a path and nothing more: the tests whose
    /// subject is what the code under test does when a path is *missing* need
    /// it that way.
    #[test]
    fn an_absent_directory_is_not_created() {
        let dir = TempDir::absent("girpr_tempdir_test", "not_there");
        assert!(!dir.path.exists(), "the path must not exist yet");
    }

    /// `at` takes the path the caller chose, and clears whatever was there.
    /// The directory itself is the code under test's to make.
    #[test]
    fn a_chosen_path_starts_clear() {
        let path = unique_path("girpr_tempdir_test", "chosen");
        std::fs::create_dir_all(&path).expect("create the path");
        std::fs::write(path.join("stale"), b"x").expect("write a stale file");

        let dir = TempDir::at(path.clone());
        assert_eq!(dir.path, path);
        assert!(!path.join("stale").exists(), "the stale file is gone");
        assert!(!path.exists(), "the path is not made for the caller");
    }

    /// A **file** at the chosen path is cleared too: `remove_dir_all` refuses
    /// one, and the promise is that the path is clear, not that a directory
    /// was there.
    #[test]
    fn a_file_at_the_chosen_path_is_cleared_too() {
        let path = unique_path("girpr_tempdir_test", "chosen_file");
        std::fs::write(&path, b"stale").expect("write a stale file");

        let dir = TempDir::at(path.clone());
        assert_eq!(dir.path, path);
        assert!(!path.exists(), "the stale file is gone");
    }

    /// `subdir` creates the parents it needs, so a fixture can ask for
    /// `a/b/c` in one call.
    #[test]
    fn a_subdir_creates_its_parents() {
        let dir = TempDir::new("girpr_tempdir_test", "sub");
        let sub = dir.subdir("a/b/c");
        assert!(sub.is_dir());
        assert_eq!(sub, dir.path.join("a").join("b").join("c"));
    }

    /// The tree is gone once the guard is dropped — however the test ends,
    /// because the removal is in `Drop` and not in the last line.
    #[test]
    fn the_tree_is_removed_when_the_guard_drops() {
        let path = {
            let dir = TempDir::new("girpr_tempdir_test", "dropped");
            dir.subdir("nested");
            dir.path.clone()
        };
        assert!(!path.exists(), "the tree must be gone");
    }
}
