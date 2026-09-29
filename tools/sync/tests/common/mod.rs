//! Shared fixtures for the integration tests.
//!
//! Each test binary gets its own copy of this module, so not every helper is
//! used in every binary.

#![allow(dead_code)]

use girsync::cache::CacheOpen;
use girsync::{CommonOpts, CompareOpts, LogCtx, ScanMode, SyncOpts, TrustOpts, UpdateOpts};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static RUN_SEQ: AtomicU64 = AtomicU64::new(0);

/// The plain read/write open most tests want: no rebuild, no backup.
pub fn rw() -> CacheOpen {
    CacheOpen::ReadWrite {
        ignore_cache: false,
        backup_first: false,
    }
}

/// Isolated temp directory per test, so parallel `cargo test` workers never
/// share a cache file.
pub struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    pub fn new(tag: &str) -> Self {
        let n = RUN_SEQ.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "girsync_run_{}_{}_{}_{}",
            std::process::id(),
            nanos,
            n,
            tag
        ));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub fn mkdirs(&self, rel: &str) -> PathBuf {
        let p = self.path.join(rel);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    pub fn root(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        // Cache handles opened by cmd_* are dropped by then; best-effort cleanup.
        std::fs::remove_dir_all(&self.path).ok();
    }
}

pub fn wfile(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&p, bytes).unwrap();
}

pub fn rfile(root: &Path, rel: &str) -> Vec<u8> {
    std::fs::read(root.join(rel)).unwrap()
}

/// Actual directory entry names, excluding `girpr-cache*`.
///
/// Path existence cannot answer casing questions on a case-insensitive
/// filesystem: `data.txt` and `Data.txt` resolve to the same entry there.
pub fn entry_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with("girpr-cache"))
        .collect();
    names.sort();
    names
}

/// Pin `target`'s mtime to `source`'s, so two identical-content files compare
/// equal (compare treats size+mtime+hash as identity).
pub fn sync_mtime(source: &Path, target: &Path) {
    let mtime = std::fs::metadata(source).unwrap().modified().unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(target)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
}

/// Default options for the end-to-end tests: md5, no filters, case-sensitive.
pub fn opts() -> CommonOpts {
    CommonOpts {
        algos: vec!["md5".to_string()],
        includes: vec![],
        excludes: vec![],
        case_sensitive: true,
        max_depth: 10,
        ignore_cache: false,
    }
}

/// Quiet log context; tracing is a no-op without a subscriber anyway.
pub fn log() -> LogCtx {
    LogCtx {
        level: "error".to_string(),
        file: None,
    }
}

/// Scan mode for tests: `no_trust_cached_hashes` and `dry_run`.
pub fn scan(no_trust_cached_hashes: bool, dry_run: bool) -> ScanMode {
    ScanMode {
        no_trust_cached_hashes,
        dry_run,
    }
}

pub fn update(dir: PathBuf) -> UpdateOpts {
    UpdateOpts {
        dir,
        common: opts(),
    }
}

pub fn compare(src: PathBuf, dst: PathBuf) -> CompareOpts {
    CompareOpts {
        src,
        dst,
        trust: TrustOpts::default(),
        common: opts(),
    }
}

pub fn sync(src: PathBuf, dst: PathBuf) -> SyncOpts {
    SyncOpts {
        src,
        dst,
        trust: TrustOpts::default(),
        missing_only: false,
        keep_extra: false,
        dry_run: false,
        jobs: 1,
        common: opts(),
    }
}

pub fn md5arg() -> Vec<String> {
    vec!["md5".to_string()]
}

/// True when a `girpr-cache-backup-*` sibling exists next to `dir`.
pub fn has_backup_sibling(dir: &Path) -> bool {
    let parent = dir.parent().unwrap_or_else(|| Path::new("."));
    let Ok(rd) = std::fs::read_dir(parent) else {
        return false;
    };
    let needle = format!("{}-backup-", girsync::cache::CACHE_PREFIX);
    rd.filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .any(|n| n.starts_with(&needle))
}

// ---------------------------------------------------------------------------
// Deterministic tree fixtures
// ---------------------------------------------------------------------------

/// Seeded PRNG for test fixtures.
///
/// A named seed reproduces a failure exactly, which is why this is hand-rolled
/// rather than a `rand`/`proptest` dev-dependency: with a failing case you have
/// the seed in the assert message, so you can re-run the exact tree instead of
/// reconstructing it. xorshift64* is a handful of lines and gives the same stream
/// on every platform and toolchain, so a fixture does not drift when the
/// dependency graph moves.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // xorshift is stuck at zero; fold a degenerate seed away.
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform-ish in `0..n`. The bias is irrelevant for fixtures and, more to
    /// the point, it is stable — which is what reproducibility needs.
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }

    pub fn fill(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.below(256) as u8).collect()
    }
}

/// A fixed base timestamp, so a generated tree is identical on every run and the
/// two roots an oracle compares really do start from the same state. Far enough
/// in the past to stay clear of filesystem timestamp granularity.
pub const BASE_MTIME_NS: i64 = 1_600_000_000_000_000_000;

/// What a path looks like on one side of a comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Absent,
    Dir,
    File(Vec<u8>),
}

/// One path across a generated pair of trees.
#[derive(Clone, Debug)]
pub struct Entry {
    pub rel: String,
    /// Where dst puts the path, when it differs from `rel`.
    ///
    /// Needed for a case-only difference: the two spellings are the same path,
    /// but on a filesystem that cannot hold `a.txt` and `A.txt` at once they have
    /// to live on separate sides to be a pair at all.
    pub dst_rel: Option<String>,
    pub src: Node,
    pub dst: Node,
    pub src_mtime: i64,
    pub dst_mtime: i64,
}

impl Entry {
    /// The path src holds it at.
    pub fn src_rel(&self) -> &str {
        &self.rel
    }

    /// The path dst holds it at.
    pub fn dst_rel(&self) -> &str {
        self.dst_rel.as_deref().unwrap_or(&self.rel)
    }

    /// True when the pair is decided only by comparing digests — the paths a
    /// lazy planner must still read.
    pub fn is_undecided(&self) -> bool {
        match (&self.src, &self.dst) {
            (Node::File(a), Node::File(b)) => {
                a.len() == b.len() && self.src_mtime == self.dst_mtime
            }
            _ => false,
        }
    }

    /// True when a case-*sensitive* comparison pairs these two at all. A
    /// case-only difference does not, so it is never undecided in that mode.
    pub fn pairs_sensitively(&self) -> bool {
        self.src_rel() == self.dst_rel()
    }
}

/// Which side of a pair to write.
#[derive(Clone, Copy, Debug)]
pub enum Which {
    Src,
    Dst,
}

/// The path shapes a comparison has to get right. Every one of these is a
/// distinct answer in the diff, so a fixture omitting any of them would let a
/// planner mistake one state for another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Same content, same stat: equal.
    Identical,
    /// Same size, same mtime, different content: the *only* shape that needs a
    /// digest, and the one the stat-mismatch short circuit must not skip.
    SameStatDiffContent,
    /// Different size: decided by size, so a lazy run must not read it.
    DiffSize,
    /// Same size, different mtime: decided by mtime.
    DiffMtime,
    DiffBoth,
    SrcOnly,
    DstOnly,
    /// File on one side, dir on the other.
    FileVsDir,
    DirVsFile,
    /// Same content and stat, different casing. A pair only in insensitive mode.
    CaseOnly,
    /// A directory present on one side only.
    EmptyDirOnly,
    /// A nested path, so walk depth and the `/` in a cache key are both used.
    Nested,
}

pub const SHAPES: [Shape; 12] = [
    Shape::Identical,
    Shape::SameStatDiffContent,
    Shape::DiffSize,
    Shape::DiffMtime,
    Shape::DiffBoth,
    Shape::SrcOnly,
    Shape::DstOnly,
    Shape::FileVsDir,
    Shape::DirVsFile,
    Shape::CaseOnly,
    Shape::EmptyDirOnly,
    Shape::Nested,
];

/// The path a shape's fixture is written at.
///
/// Shared by the generator and the tests that assert the generator produced every
/// shape, so the two cannot drift into disagreeing about what a shape is called.
pub fn shape_rel(index: usize, shape: Shape) -> String {
    match shape {
        Shape::Nested => format!("deep/sub{index}/nested.txt"),
        Shape::EmptyDirOnly => format!("emptydir{index}"),
        _ => format!("f{index}.dat"),
    }
}

/// Build one entry of the given shape, with content and lengths from `rng`.
///
/// Two rules make the shapes mean what they say: a size difference is always a
/// *length* difference, and an mtime difference is always a real one. Otherwise a
/// shape could decide for a different reason than intended and the test would be
/// measuring the wrong thing.
pub fn make_entry(shape: Shape, rel: &str, rng: &mut Rng) -> Entry {
    let len = 1 + rng.below(64) as usize;
    let m = BASE_MTIME_NS + (rng.below(1000) as i64) * 1_000_000_000;
    let content = rng.fill(len);
    // Same length, different bytes: only a digest can tell these apart.
    let mut twisted = content.clone();
    if let Some(last) = twisted.last_mut() {
        *last = last.wrapping_add(1);
    }
    let longer = {
        let mut v = content.clone();
        v.push(0xAB);
        v
    };
    let shifted = m + 3_600_000_000_000;
    let e = |src: Node, dst: Node, src_mtime: i64, dst_mtime: i64| Entry {
        rel: rel.to_string(),
        dst_rel: None,
        src,
        dst,
        src_mtime,
        dst_mtime,
    };
    match shape {
        Shape::Identical => e(Node::File(content.clone()), Node::File(content), m, m),
        Shape::SameStatDiffContent => e(Node::File(content), Node::File(twisted), m, m),
        Shape::DiffSize => e(Node::File(content), Node::File(longer), m, m),
        Shape::DiffMtime => e(Node::File(content.clone()), Node::File(content), m, shifted),
        Shape::DiffBoth => e(Node::File(content), Node::File(longer), m, shifted),
        Shape::SrcOnly => e(Node::File(content), Node::Absent, m, m),
        Shape::DstOnly => e(Node::Absent, Node::File(content), m, m),
        Shape::FileVsDir => e(Node::File(content), Node::Dir, m, m),
        Shape::DirVsFile => e(Node::Dir, Node::File(content), m, m),
        // Identical bytes, different spelling: the same path, so still undecided,
        // and only a digest can confirm the content matches.
        Shape::CaseOnly => Entry {
            dst_rel: Some(swap_case(rel)),
            ..e(Node::File(content.clone()), Node::File(content), m, m)
        },
        Shape::EmptyDirOnly => e(Node::Dir, Node::Absent, m, m),
        Shape::Nested => e(Node::File(content.clone()), Node::File(content), m, m),
    }
}

/// Uppercase the final component, so the pair differs only by case.
fn swap_case(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => format!("{}{}", &rel[..=i], rel[i + 1..].to_uppercase()),
        None => rel.to_uppercase(),
    }
}

/// Write one entry's one side into `root`, creating parent directories.
///
/// mtime is stamped explicitly from the entry rather than inherited from the
/// write, so a tree's stat identity is part of the fixture rather than a property
/// of how fast the test happened to run.
pub fn materialize(root: &Path, e: &Entry, which: Which) {
    let (node, mtime, rel) = match which {
        Which::Src => (&e.src, e.src_mtime, e.src_rel()),
        Which::Dst => (&e.dst, e.dst_mtime, e.dst_rel()),
    };
    let p = root.join(rel);
    match node {
        Node::Absent => {}
        Node::Dir => {
            std::fs::create_dir_all(&p).unwrap();
        }
        Node::File(bytes) => {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, bytes).unwrap();
            std::fs::OpenOptions::new()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(mtime as u64))
                .unwrap();
        }
    }
}

/// Write both sides of every entry into `src_root` / `dst_root`.
pub fn materialize_pair(src_root: &Path, dst_root: &Path, entries: &[Entry]) {
    for e in entries {
        materialize(src_root, e, Which::Src);
        materialize(dst_root, e, Which::Dst);
    }
}

/// A tree pair containing every shape in [`SHAPES`], plus `extra` random
/// undecided files.
///
/// Every shape is always present rather than sampled: the point of a randomized
/// fixture is the interaction between shapes, and a sampled subset would silently
/// stop testing whichever one it dropped. The randomness is in content, lengths
/// and mtimes — which is where a planner could plausibly disagree.
pub fn tree_pair(rng: &mut Rng, extra: usize) -> Vec<Entry> {
    let mut out = Vec::new();
    for (i, shape) in SHAPES.iter().enumerate() {
        out.push(make_entry(*shape, &shape_rel(i, *shape), rng));
    }
    for i in 0..extra {
        let len = 1 + rng.below(32) as usize;
        let a = rng.fill(len);
        let mut b = a.clone();
        if let Some(last) = b.last_mut() {
            *last = last.wrapping_add(1);
        }
        let m = BASE_MTIME_NS + (rng.below(1000) as i64) * 1_000_000_000;
        out.push(Entry {
            rel: format!("rand{i}.dat"),
            dst_rel: None,
            src: Node::File(a),
            dst: Node::File(b),
            src_mtime: m,
            dst_mtime: m,
        });
    }
    out
}
