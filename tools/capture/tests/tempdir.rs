//! A scratch directory for one test: named so nothing can collide with it,
//! and removed when the test ends however it ends.
//!
//! ## Why this is duplicated from `tests/common/mod.rs`
//!
//! That module is the mock API, and it is compiled into every integration
//! target that speaks to the network. The handful of test targets that only
//! spawn the binary (`cli_contract.rs`, `offline_bundle.rs`) do not pull it in,
//! so they include this file directly. One definition, reachable from both
//! sides, so the two cannot drift.
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
//!   the next run, so the second run asserts against the first run's leftovers.
//!
//! So the name carries the process id, a wall-clock timestamp and a
//! per-process counter — the three things that, together, separate two tests
//! in two processes at two times — and [`Drop`] removes the tree, so cleanup
//! does not depend on the test reaching its last line.

#![allow(dead_code)]

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many directories this process has handed out. The pid and the timestamp
/// already make the name unique across processes and across runs; this is what
/// separates two directories created *within* one test, where both would
/// otherwise land in the same nanosecond.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// A path under the system temp dir that no other test, and no other run, will
/// use: `<temp>/<prefix>-<tag>-<pid>-<nanos>-<seq>`.
///
/// Every part earns its place. `prefix` says which suite leaked a directory if
/// one survives a crash, `tag` names the test that wanted it, `pid` separates
/// two suites running at once, `nanos` separates two runs of the same binary
/// (a pid is reused, and a killed run leaves its tree behind), and `seq`
/// separates two directories made inside one test.
///
/// The directory is **not** created: a test whose subject is what the code does
/// when a path is absent needs it that way.
pub fn unique_path(prefix: &str, tag: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the system clock is before 1970")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "{prefix}-{tag}-{}-{nanos}-{seq}",
        std::process::id()
    ))
}

/// A scratch directory, removed when the value goes out of scope.
///
/// Derefs to [`Path`], so `dir.join(..)`, `&dir` where a `&Path` is wanted, and
/// every other path operation work without a conversion at the call site.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// An empty directory, created.
    pub fn new(prefix: &str, tag: &str) -> Self {
        let path = unique_path(prefix, tag);
        std::fs::create_dir_all(&path).expect("create the temp dir");
        Self { path }
    }

    /// A path that does not exist yet.
    ///
    /// For the tests whose subject is what the code under test does when the
    /// path is *missing* — an `emit` case that must create its own parents, or
    /// a `--log-file` under a directory that is not there yet.
    pub fn absent(prefix: &str, tag: &str) -> Self {
        Self {
            path: unique_path(prefix, tag),
        }
    }

    /// A guard over a path the *caller* chose.
    ///
    /// For the few cases where the path is not free: a `materialize` run that
    /// defaults to writing *beside* the bundle, so the assertion can only be
    /// made against the sibling the tool itself picks. Constructed from a
    /// [`unique_path`], so it is as collision-proof as any other; `new` exists
    /// for the cases that have a free choice.
    pub fn at(path: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&path);
        Self { path }
    }

    /// `rel` under this root, with its parents created.
    pub fn subdir(&self, rel: &str) -> PathBuf {
        let p = self.path.join(rel);
        std::fs::create_dir_all(&p).expect("create the temp subdir");
        p
    }

    /// The path, cloned, for the call sites that hand an owned `PathBuf` to the
    /// binary under test or to a struct that outlives this guard.
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
        // Best effort: on Windows a log sink or blob file the test left open
        // refuses the delete, and a leftover in the temp dir is a far smaller
        // problem than a test that cannot report its own failure over it.
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
