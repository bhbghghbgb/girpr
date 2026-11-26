//! A local mock of the whole API surface, built by the test.
//!
//! No dependency, no fixture file, and nothing borrowed from the
//! repair binary's own mock: the synthetic manifests, the chunk bodies, the
//! `getBuild` bodies and the four HYP bodies are all *constructed here*,
//! from a [`World`] the test writes. That is the point — a fixture file checked
//! into the tree is a fixture that can only test what it happened to contain,
//! and a mock that shares code with the code under test is a mock that agrees
//! with a bug.
//!
//! ## Why a hand-rolled HTTP/1.1 server
//!
//! Because `tokio`'s test utilities are a dependency this crate does not need
//! for anything else, and a mock server is the one place where "serve whatever
//! bytes the world says" has to be *obviously* correct. A `TcpListener`, a read
//! loop for the request line, and a write of the response is the whole protocol
//! this client uses, and being able to read it end to end is worth the forty
//! lines.
//!
//! ## The bases
//!
//! One listener serves all three families, under different path prefixes:
//!
//! | prefix   | what it answers                                    |
//! |----------|----------------------------------------------------|
//! | `/hyp`   | the four HYP endpoints                             |
//! | `/sophon`| `getBuild`                                          |
//! | `/cdn`   | manifest archives and chunk blobs                  |
//!
//! All three live on `127.0.0.1` with an ephemeral port, so **no test can reach
//! the network even by accident** — there is nothing else to reach. That is the
//! property the suite asserts, so it is enforced by construction rather than by
//! remembering to pass `--hyp-base`.
//!
//! ## Why a library, not an inlined `tests/common` module
//!
//! nextest runs one process per test and skips doctests, so a helper inlined
//! with `mod common;` is compiled into every integration binary. This crate is
//! a public library surface reached through the thin `tests/common/mod.rs`
//! shim: the mock, the world builders and the shared-bundle publisher compile
//! once. It depends on gircap's own public seams (a test-only inversion) and on
//! nothing from the repair binary, so the independence rule in
//! `docs/00-spec.md` §2 still holds.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use fs2::FileExt;
use gircap::cli::{Cmd, parse_from, recorded_argv};
use gircap::config::{capture_plan, materialize_plan, verify_plan};
use gircap::http::{HttpRequest, HttpResponse, HttpTransport, SendFuture};
use gircap::report::Record;
use gircap::run::{self, Failure, Outcome};
use girpr_tempdir::TempDir;

// ------------------------------------------------- in-process command dispatch

/// What a gircap command produced, in the shape `main.rs` would have printed.
///
/// The four public APIs in [`run`] are async and return a [`Record`] plus an
/// exit code; on failure they return a [`Failure`] that carries both its own
/// summary and — for a run that measured before failing — the run's own summary
/// in `preceded_by`. A harness that spawns the binary learns all of this by
/// parsing stdout. A harness that calls the APIs in-process gets the typed
/// values instead, which is strictly stronger: a field is read by name from a
/// [`Record`], not recovered from a rendered `k=v` line.
///
/// The command is still parsed by the real clap surface and resolved by the
/// real config functions, so the flags a test passes travel the same path a
/// user's command line does. Only the final `print`/`exit` shim is skipped.
pub struct Dispatch {
    /// The run's own SUMMARY: the success record, or the one a failure carries
    /// in `preceded_by`. `None` when the run refused before measuring anything.
    pub run_record: Option<Record>,
    /// The failure's own SUMMARY (`result=error`). Present only on failure.
    pub failure_record: Option<Record>,
    /// The one-line cause. Present only on failure.
    pub detail: Option<String>,
    /// The exit code the process would have ended with.
    pub code: i32,
}

impl Dispatch {
    /// The first SUMMARY a stdout reader would see: the run's own when it
    /// measured, otherwise the failure's.
    pub fn summary(&self) -> &Record {
        self.run_record
            .as_ref()
            .or(self.failure_record.as_ref())
            .expect("a command always emits one summary")
    }
}

/// A poll driver for the `run::*` futures.
///
/// `verify`/`materialize`/`list` do no real awaiting, but `capture` drives
/// reqwest's sockets and `spawn_blocking`, so it needs a real runtime's IO and
/// time drivers. A fresh current-thread runtime per dispatch is enough and
/// keeps a test free of a runtime dependency of its own.
pub fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build a current-thread runtime")
        .block_on(f)
}

/// Run a gircap command in-process, exactly as `main.rs` dispatches it, and
/// return the typed records/exit code instead of printing and exiting.
pub fn dispatch(argv: &[&str]) -> Dispatch {
    dispatch_with(argv, None)
}

/// [`dispatch`] with an injected HTTP transport for `capture`.
///
/// `None` is exactly [`dispatch`] and uses the live `reqwest` client. Passing an
/// in-memory transport runs the whole capture pipeline — same clap parse, same
/// config resolution, same library dispatch — against a fixture with no TCP
/// listener, no accept thread and no per-connection threads.
pub fn dispatch_with(argv: &[&str], http: Option<Arc<dyn HttpTransport>>) -> Dispatch {
    // `main.rs` parses `args_os()`, whose first entry is the program name that
    // clap strips. Prepend the same sentinel so the subcommand is not mistaken
    // for argv[0], and record it so `MANIFEST.json → tool_argv` carries the same
    // shape a spawned run would (byte-exact gates normalise only the `--out`
    // root, so this spelling must be stable across calls).
    let full: Vec<OsString> = std::iter::once(OsString::from("gircap"))
        .chain(argv.iter().map(OsString::from))
        .collect();
    let cli = parse_from(full.iter().cloned()).expect("a valid gircap command");
    let event = match &cli.cmd {
        Cmd::Capture(_) => "capture",
        Cmd::Verify(_) => "verify",
        Cmd::Materialize(_) => "materialize",
        Cmd::List(_) => "list",
    };
    let recorded = recorded_argv(full.iter().cloned());
    let outcome: Result<Outcome, Failure> = block_on(async {
        match cli.cmd {
            Cmd::Capture(a) => {
                run::capture_with_http(capture_plan(&a, recorded).expect("capture plan"), http)
                    .await
            }
            Cmd::Verify(a) => run::verify(verify_plan(&a).expect("verify plan")).await,
            Cmd::Materialize(a) => {
                run::materialize(materialize_plan(&a).expect("materialize plan")).await
            }
            Cmd::List(a) => run::list(a.root, a.common.output).await,
        }
    });
    match outcome {
        Ok((summary, code)) => Dispatch {
            run_record: Some(summary),
            failure_record: None,
            detail: None,
            code: code.code(),
        },
        Err(f) => Dispatch {
            run_record: f.preceded_by.clone(),
            failure_record: Some(f.summary(event)),
            detail: Some(f.detail.clone()),
            code: f.code.code(),
        },
    }
}

// ------------------------------------------------- the shared read-only bundle

/// The one capture variant, shared by every suite that reads "a document from
/// the default bundle".
///
/// The non-default variants this used to carry existed only to make a single
/// number different (a lower gzip threshold, a raised decompression ceiling).
/// Those numbers are now pinned by in-process unit tests that call the exact
/// production functions — `manifest::dump::write` for the gzip decision and
/// `emit::index::build_manifest` for the ceiling field — so a full second and
/// third capture can no longer earn their cost. One tree is captured once for
/// the whole suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BundleVariant {
    /// `World::all_shapes()` with no extra flags.
    Default,
}

impl BundleVariant {
    fn key(self) -> &'static str {
        match self {
            BundleVariant::Default => "default",
        }
    }

    fn extra(self) -> &'static [&'static str] {
        match self {
            BundleVariant::Default => &[],
        }
    }
}

/// 64-bit FNV-1a over a sequence of bytes, enough to key a temp directory on.
/// Only change detection matters here, not cryptographic use. `fnv` owns the
/// offset, prime and byte order; which bytes are hashed stays local.
fn fnv1a(bytes: impl IntoIterator<Item = u8>) -> u64 {
    use std::hash::Hasher;
    let mut hasher = fnv::FnvHasher::default();
    for byte in bytes {
        hasher.write(&[byte]);
    }
    hasher.finish()
}

/// Collect every Rust source file under `dir` (recursively) into `out`.
///
/// `WalkDir` owns the traversal. `sort_by_file_name` gives a depth-first,
/// per-directory order so the fingerprint never depends on the filesystem's
/// enumeration order.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in walkdir::WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() && entry.path().extension().is_some_and(|e| e == "rs") {
            out.push(entry.into_path());
        }
    }
}

/// Fingerprint of the capture product source: every `.rs` under `src/` plus the
/// crate manifest, keyed by relative path and content.
///
/// The shared bundle is a capture of the current product code, so a change to
/// what `capture` writes must invalidate it even when the test-side world is
/// untouched. Paths are sorted so the hash never depends on directory iteration
/// order, and the bytes come from the one source tree every process reads — the
/// bundle is published under a cross-process file lock, so all processes must
/// agree on the key.
fn capture_source_key(manifest_dir: &Path) -> u64 {
    let mut files = Vec::new();
    collect_rs_files(&manifest_dir.join("src"), &mut files);
    files.push(manifest_dir.join("Cargo.toml"));
    files.sort();

    let mut bytes = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(manifest_dir)
            .expect("under the crate root");
        bytes.extend_from_slice(rel.to_string_lossy().as_bytes());
        bytes.push(0); // separator so path bytes cannot blend into content
        let content =
            std::fs::read(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        bytes.extend_from_slice(&content);
    }
    fnv1a(bytes)
}

/// A stable key for the tree a variant is built from.
///
/// Two fingerprints are folded in. The first is over this library's source,
/// which is where [`World::all_shapes`] lives: if the world changes (a chunk shrinks, a file is
/// added) the bundle is rebuilt rather than reusing a capture of the old world —
/// a stale bundle is self-consistent, so `verify` would not notice it and only
/// the assertions that assume the current world would silently drift. The
/// second is over the capture product source, so a change to what `capture`
/// writes invalidates the bundle even when the world is untouched. The format
/// byte lets a future change to this fixture force a rebuild without touching
/// either source.
fn bundle_source_key() -> String {
    // Bumped when the fixture source moved from `tests/common/mod.rs` into this
    // library, and again when the FNV hash behind both fingerprints became a
    // library hasher, so a bundle published under an old key is not reused.
    const FORMAT: u32 = 4;
    // This crate sits at `tools/capture/test-support`; the capture package root
    // is its parent. Deriving it here keeps the key identical in every test
    // process regardless of which binary reached the shim.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test-support crate sits under tools/capture");
    let world = fnv1a(
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"))
            .expect("read the shared fixture's own source"),
    );
    let capture = capture_source_key(manifest_dir);
    format!("{FORMAT}-{world:016x}-{capture:016x}")
}

/// A path into one capture written to a stable location on disk and shared by
/// every test that asks for the same [`BundleVariant`].
///
/// `cargo nextest` runs each test in its own process, so a process-global
/// `OnceLock` cannot share a capture: the processes never coexist. The sharing
/// is therefore on disk. Whichever process takes a file lock first performs
/// the one capture and publishes the finished tree by rename; every other
/// process blocks on the same lock, sees a complete tree, and reads it. Under
/// `cargo test` (one process, many threads) the same lock still serialises the
/// first build, so both runners are race-free.
///
/// The tree is **read-only by contract**: no test writes into it. A test that
/// must change a document copies the tree into its own scratch first. It is
/// kept under [`girpr_tempdir::root`] so a leftover from a killed run is
/// recognisable as this project's, and it is reused across runs rather than
/// deleted — removal is what would race with a process still reading it.
pub struct SharedBundle {
    bundle: PathBuf,
}

impl Deref for SharedBundle {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.bundle
    }
}

impl AsRef<Path> for SharedBundle {
    fn as_ref(&self) -> &Path {
        &self.bundle
    }
}

/// Return the on-disk bundle for `variant`, capturing it first if nobody has.
///
/// The tree has the same shape as any default capture against
/// [`World::all_shapes`], so every assertion that reads a document from a
/// capture applies to it unchanged.
pub fn shared_bundle(variant: BundleVariant) -> SharedBundle {
    let key = format!("gircap-shared-{}-{}", variant.key(), bundle_source_key());
    let root = girpr_tempdir::root();
    std::fs::create_dir_all(&root).expect("create the shared fixture root");
    let dir = root.join(&key);
    let lock = root.join(format!("{key}.lock"));

    // A lock file is only a handle to lock; its bytes are never read or
    // written. `truncate(false)` is explicit so reopening never disturbs a
    // process that already holds the lock.
    let guard = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock)
        .unwrap_or_else(|e| panic!("open {}: {e}", lock.display()));
    guard
        .lock_exclusive()
        .unwrap_or_else(|e| panic!("lock {}: {e}", lock.display()));

    // `checksums.json` is written in close-out, after every other document and
    // after the tree is assembled: its presence means the capture finished.
    // A tree with a manifest but no checksums is one a builder died halfway
    // through, and is safe to clear while holding the lock.
    let complete =
        dir.join("MANIFEST.json").is_file() && dir.join("verify/checksums.json").is_file();
    if !complete {
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .unwrap_or_else(|e| panic!("clear {}: {e}", dir.display()));
        }
        let staging = TempDir::new("gircap-shared", variant.key());
        let out = staging.join("artifacts");
        let mock = Mock::in_memory(World::all_shapes());
        let out_arg = out.to_string_lossy().into_owned();
        let mut argv: Vec<String> = vec![
            "capture".into(),
            "--out".into(),
            out_arg,
            "--hyp-base".into(),
            mock.hyp_base(),
            "--sophon-base".into(),
            mock.sophon_base(),
        ];
        argv.extend(variant.extra().iter().map(|s| s.to_string()));
        let borrowed: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        capture_ok(&borrowed, Some(mock.transport()));
        drop(mock);
        let built = find_bundle(&out);
        std::fs::rename(&built, &dir)
            .unwrap_or_else(|e| panic!("publish {} to {}: {e}", built.display(), dir.display()));
    }

    guard
        .unlock()
        .unwrap_or_else(|e| panic!("unlock {}: {e}", lock.display()));
    SharedBundle { bundle: dir }
}

/// Run `capture` in-process and require it to succeed. Only the healthy
/// variants are built through the shared fixture, so a non-zero exit is a
/// broken fixture, not a result to interpret. The injected transport answers
/// from the fixture table; only the process boundary and the socket are gone.
fn capture_ok(args: &[&str], http: Option<Arc<dyn HttpTransport>>) {
    let d = dispatch_with(args, http);
    assert_eq!(
        d.code,
        0,
        "shared capture failed: {:?}",
        d.detail.as_deref()
    );
}

/// The one bundle under `dir`, whichever way the layout nests it.
///
/// `WalkDir` owns the traversal; `sort_by_file_name` keeps the same depth-first
/// order as the old stack walk, so the chosen bundle is deterministic.
fn find_bundle(dir: &Path) -> PathBuf {
    for entry in walkdir::WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if entry.file_type().is_dir() && path.join("MANIFEST.json").is_file() {
            return path.to_path_buf();
        }
    }
    panic!("no bundle under {}", dir.display());
}

// --------------------------------------------------------------- a minimal bundle

/// Write the smallest directory `gircap::verify::Bundle::open` accepts, using
/// the crate's own value types so a reader change cannot leave a stale
/// hand-rolled fixture.
///
/// The "delete a document" and "missing manifest field" tests need a bundle that
/// is valid in every respect *except* the one thing they break: otherwise the
/// refusal they assert could be about a broken fixture rather than the missing
/// document. Capturing a whole bundle for each such test is wasteful, so this
/// writes the same set of documents the writers would, empty where the assertion
/// does not care. The returned `TempDir` owns the tree and removes it on drop.
pub fn minimal_bundle(name: &str) -> (TempDir, PathBuf) {
    use gircap::blobs::ChunkIndex;
    use gircap::emit::index::{
        Biz_, Build, Capture, Checksums, Manifest, SelectionPool, Tool, Totals, Versions,
    };
    use gircap::emit::selfcheck::{Counts, Result_, Selfcheck};
    use gircap::http::ErrorResponses;
    use gircap::http::probe::ErrorShapes;
    use gircap::pathmap::PathMapDocument;
    use gircap::rejections::RejectionsDocument;

    let scratch = TempDir::new("gircap-minimal", name);
    let root = scratch.join("bundle");
    std::fs::create_dir_all(&root).expect("bundle root");

    let write = |rel: &str, value: serde_json::Value| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("makes dirs");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&value).expect("serializes"),
        )
        .expect("writes");
    };
    let v = gircap::SCHEMA_VERSION;

    // A manifest with every field the reader wants, so a test that removes a
    // required field fails on *that field* and not on another missing one.
    let manifest = Manifest {
        schema_version: v,
        tool: Tool {
            name: "gircap".into(),
            version: "test".into(),
        },
        capture: Capture {
            id: name.into(),
            captured_at: "20260101T000000Z".into(),
            host_os: "test".into(),
            tool_argv: vec![],
            tool_argv_raw: vec![],
            complete: true,
        },
        biz: Biz_ {
            id: "biz".into(),
            launcher_id: String::new(),
            game_id: String::new(),
            channel: String::new(),
            sub_channel: String::new(),
            cps: String::new(),
            hyp_base: String::new(),
            sophon_base: String::new(),
        },
        versions: Versions {
            new: Build {
                tag: "2.0.0".into(),
                build_id: "new-build".into(),
                source: "test".into(),
            },
            old: Build {
                tag: "1.0.0".into(),
                build_id: "old-build".into(),
                source: "test".into(),
            },
            diff_tags: vec![],
        },
        field: "game".into(),
        audio: vec![],
        gzip_threshold: 0,
        max_decompressed_bytes: 0,
        selection_pool: SelectionPool {
            fields: vec![],
            manifest_entries: BTreeMap::new(),
            note: String::new(),
        },
        totals: Totals {
            files_captured: 0,
            chunks_blobs: 0,
            bytes_downloaded: 0,
            bytes_on_disk: 0,
        },
        matrix: vec![],
        files: vec![],
        totals_by_set: BTreeMap::new(),
    };
    write(
        "MANIFEST.json",
        serde_json::to_value(manifest).expect("serializes"),
    );
    write(
        "chunks/index.json",
        serde_json::to_value(ChunkIndex::build(vec![])).expect("serializes"),
    );
    write(
        "verify/checksums.json",
        serde_json::to_value(Checksums {
            schema_version: v,
            algorithm: "sha256".into(),
            entries: vec![],
            file_count: 0,
            total_bytes: 0,
        })
        .expect("serializes"),
    );
    write(
        "verify/selfcheck.json",
        serde_json::to_value(Selfcheck {
            schema_version: v,
            result: Result_::Pass,
            counts: Counts {
                total: 0,
                passed: 0,
                failed: 0,
            },
            assertions: vec![],
            matrix: vec![],
        })
        .expect("serializes"),
    );
    write(
        "api/error_shapes.json",
        serde_json::to_value(ErrorShapes {
            schema_version: v,
            shapes: vec![],
        })
        .expect("serializes"),
    );
    write(
        "api/error_responses.json",
        serde_json::to_value(ErrorResponses {
            schema_version: v,
            budget_bytes: 0,
            total_retained_bytes: 0,
            responses: vec![],
        })
        .expect("serializes"),
    );
    write(
        "diagnostics/rejections.json",
        serde_json::to_value(RejectionsDocument {
            schema_version: v,
            cap_bytes: 0,
            total: 0,
            omitted: 0,
            rejections: vec![],
        })
        .expect("serializes"),
    );
    write(
        "diagnostics/path_map.json",
        serde_json::to_value(PathMapDocument {
            schema_version: v,
            entries: vec![],
        })
        .expect("serializes"),
    );

    (scratch, root)
}

// ---------------------------------------------------------------- the world

/// A path whose patch shape answers no matrix class: one chunk, no reuse.
///
/// It is the same shape the frozen measured corpus carries, so a test that
/// forces it exercises the real degenerate case rather than an invented one.
pub const DEGENERATE_PATH: &str = "GenshinImpact_Data/AssetBundles/data_revision";

/// One chunk, with the bytes a CDN would serve and everything a manifest
/// declares about it.
///
/// The declared sizes and digests are *computed from the bytes*, never
/// hardcoded, because a mock whose manifest lies about its own archive is a mock
/// that would let a real bug through: the capture would fail A3/A5 against a
/// fixture whose only sin was being hand-written.
#[derive(Debug, Clone)]
pub struct Blob {
    /// `<16 opaque hex>_<uncompressed_md5>`.
    pub id: String,
    /// The bytes as served: a zstd frame.
    pub raw: Vec<u8>,
    /// The bytes after zstd.
    pub body: Vec<u8>,
    /// `md5(raw)`.
    pub compressed_md5: String,
    /// `len(raw)`.
    pub compressed_size: i64,
    /// `md5(body)`.
    pub uncompressed_md5: String,
    /// `len(body)`.
    pub uncompressed_size: i64,
    /// The length the manifest declares, when it is not `len(body)`.
    ///
    /// `None` for every ordinary blob, so the world keeps its "declare what the
    /// bytes say" property by default. A test overwrites it to model a manifest
    /// whose arithmetic is hostile - a chunk at `i64::MAX`, which is the one
    /// extreme the decoder accepts, since it refuses a negative chunk size at
    /// its arithmetic boundary - while the bytes a CDN serves stay ordinary.
    /// The archive stats are derived from this value with the same `.max(0)`
    /// and saturating additions the decoder applies, so A5 still reconciles.
    pub declared_uncompressed_size: Option<i64>,
    /// The bytes the CDN serves, when they are not `raw`.
    ///
    /// `None` for every ordinary blob. A test sets it to model a server that
    /// returns something other than the frame the manifest declares; the
    /// declaration (`compressed_size`, `compressed_md5`) is left alone, which
    /// is exactly the mis-served-body case the capture has to record.
    pub served: Option<Vec<u8>>,
}

/// Per-process memo of generated blobs, keyed by `(seed, len)`.
///
/// [`blob`] is a pure function of its inputs: the same seed and length always
/// yield the same body, the same zstd frame and the same digests. Encoding and
/// hashing are the dearest part of building [`World::all_shapes`] — it makes
/// ~25 blobs on every call, and a suite calls it once per capture — so each
/// distinct blob is generated at most once per process and cloned out after.
///
/// The cached blobs are immutable by contract: the builder methods
/// ([`Blob::declaring_uncompressed_size`], [`File::declaring_size`]) consume and
/// return new values, and [`World::mis_serve`] edits a caller's world rather than
/// the memo. This is a **per-process** cache only — `cargo nextest` runs each
/// test in its own process, so it does not share across them; the on-disk shared
/// bundle fixture ([`shared_bundle`]) is what handles cross-process reuse.
static BLOBS: OnceLock<Mutex<HashMap<(u64, usize), Blob>>> = OnceLock::new();

/// A chunk whose bytes are a function of `seed`, and nothing else.
///
/// Deterministic so a test's expectations are stable, and distinct per `seed` so
/// two files cannot collide by accident. Zstd-encoding and hashing happen once
/// per distinct `(seed, len)` per process.
pub fn blob(seed: u64, len: usize) -> Blob {
    let cache = BLOBS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().expect("the blob cache is not poisoned");
    cache
        .entry((seed, len))
        .or_insert_with(|| {
            let mut body = Vec::with_capacity(len);
            let mut x = seed
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(0x1234_5678_9ABC_DEF0)
                | 1;
            for _ in 0..len {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                body.push((x >> 24) as u8);
            }
            let raw = zstd::encode_all(body.as_slice(), 3).expect("zstd encode");
            let uncompressed_md5 = gircap::hash::md5_hex(&body);
            // The 16-hex prefix is deliberately *not* a function of anything a reader
            // could check, because upstream's is not either (docs/00-spec.md §3.7).
            let prefix =
                gircap::hash::md5_hex(format!("prefix/{seed}").as_bytes())[..16].to_string();
            Blob {
                id: format!("{prefix}_{uncompressed_md5}"),
                compressed_md5: gircap::hash::md5_hex(&raw),
                compressed_size: raw.len() as i64,
                uncompressed_md5,
                uncompressed_size: body.len() as i64,
                declared_uncompressed_size: None,
                served: None,
                raw,
                body,
            }
        })
        .clone()
}

impl Blob {
    /// Declare a length other than `len(body)` in the manifest.
    ///
    /// The bytes, their digests and their compressed size are untouched: only
    /// the number the manifest says the chunk decodes to changes, which is what
    /// makes the capture's clamping visible in `chunks/index.json`.
    pub fn declaring_uncompressed_size(mut self, size: i64) -> Blob {
        self.declared_uncompressed_size = Some(size);
        self
    }

    /// The length the manifest declares for this chunk.
    pub fn declared_uncompressed_size(&self) -> i64 {
        self.declared_uncompressed_size
            .unwrap_or(self.uncompressed_size)
    }
}

/// One file's entry: its path and the chunks, in order.
#[derive(Debug, Clone)]
pub struct File {
    /// The manifest-relative path.
    pub path: String,
    /// Its chunks, left to right.
    pub chunks: Vec<Blob>,
    /// The file size the manifest declares, when it is not `Σ chunk size`.
    ///
    /// `None` for every ordinary file. A test sets it to model a manifest whose
    /// file-level arithmetic disagrees with its chunks - notably a negative
    /// `size`, which the decoder clamps out of the totals but still counts as a
    /// structural violation.
    pub declared_size: Option<i64>,
}

impl File {
    /// A file of `n` chunks derived from `seeds`, each `len` bytes.
    pub fn new(path: &str, seeds: &[u64], len: usize) -> File {
        File {
            path: path.to_string(),
            chunks: seeds.iter().map(|s| blob(*s, len)).collect(),
            declared_size: None,
        }
    }

    /// Declare a whole-file size other than the chunk sum.
    pub fn declaring_size(mut self, size: i64) -> File {
        self.declared_size = Some(size);
        self
    }

    /// The bytes a reader would get back.
    pub fn bytes(&self) -> Vec<u8> {
        self.chunks.iter().flat_map(|c| c.body.clone()).collect()
    }

    /// The declared size, which is `Σ uncompressed_size` unless a test
    /// overwrote it.
    ///
    /// Saturating, because a test may declare a chunk at `i64::MAX`; the
    /// decoder sums file sizes the same way.
    pub fn size(&self) -> i64 {
        if let Some(size) = self.declared_size {
            return size;
        }
        self.chunks.iter().fold(0, |total, c| {
            total.saturating_add(c.declared_uncompressed_size())
        })
    }

    /// The declared whole-file MD5.
    pub fn md5(&self) -> String {
        gircap::hash::md5_hex(&self.bytes())
    }

    /// This file's chunks, with the offsets they tile from.
    pub fn with_offsets(&self) -> Vec<gircap::manifest::proto::SophonChunk> {
        let mut cursor = 0_i64;
        self.chunks
            .iter()
            .map(|c| {
                let size = c.declared_uncompressed_size();
                let chunk = gircap::manifest::proto::SophonChunk {
                    id: c.id.clone(),
                    uncompressed_md5: c.uncompressed_md5.clone(),
                    offset: cursor,
                    compressed_size: c.compressed_size,
                    uncompressed_size: size,
                    unknown_field6: 0,
                    compressed_md5: c.compressed_md5.clone(),
                };
                // A hostile declaration can make the next start unrepresentable.
                // The decoder measures that as a broken tiling rather than
                // refusing the manifest, and the mock must be able to produce
                // the row it measures.
                cursor = cursor.saturating_add(size);
                chunk
            })
            .collect()
    }
}

/// One build's published content, per `matching_field`.
#[derive(Debug, Clone)]
pub struct Build {
    /// `7.1.0`.
    pub tag: String,
    /// The Sophon build id, which namespaces the CDN paths.
    pub build_id: String,
    /// `(matching_field, files)`.
    pub fields: Vec<(String, Vec<File>)>,
}

impl Build {
    /// The files published for `field`.
    pub fn field(&self, field: &str) -> Option<&[File]> {
        self.fields
            .iter()
            .find(|(f, _)| f == field)
            .map(|(_, files)| files.as_slice())
    }

    /// Every chunk this build publishes, by id.
    pub fn chunks(&self) -> BTreeMap<String, Blob> {
        let mut out = BTreeMap::new();
        for (_, files) in &self.fields {
            for f in files {
                for c in &f.chunks {
                    out.insert(c.id.clone(), c.clone());
                }
            }
        }
        out
    }
}

/// The whole synthetic world: the builds, and what the four HYP endpoints
/// say about them.
///
/// The branch head is `new`; `old` must appear in `diff_tags` or the tool will
/// refuse the pair, which is itself worth being able to test.
#[derive(Debug, Clone)]
pub struct World {
    /// The branch head.
    pub new: Build,
    /// The version to patch from.
    pub old: Build,
    /// Extra tags `getBuild` will serve but the pair does not use.
    pub other_tags: Vec<String>,
    /// Whether `diff_tags` lists the old tag.
    pub old_in_diff_tags: bool,
    /// `getGameConfigs → audio_pkg_scan_dir`, which the fixture writes under
    /// `game/`. A test can make it hostile to exercise the API-path boundary.
    pub audio_pkg_scan_dir: String,
}

impl World {
    /// A world whose `game` field exercises every patch shape.
    ///
    /// | file                 | old → new                            | shape                |
    /// |----------------------|--------------------------------------|----------------------|
    /// | `browser_assets`     | same chunks, same order              | `identical`          |
    /// | `libNew.dll`         | absent → one new chunk              | `added`              |
    /// | `old.blk`            | one old chunk → absent              | `removed`            |
    /// | `zero.blk`           | all chunks replaced                 | `no_reuse`           |
    /// | `HoYoChannel.dll`    | first chunk replaced, rest shared   | `reuse_in_place`     |
    /// | `eager.blk`          | the file shrinks, no source past the end | `reuse_in_place` |
    /// | `deferred.blk`       | the file shrinks, a source past the end | `reuse_moved`    |
    /// | `grow.blk`           | the file grows, the content moves later | `reuse_moved`   |
    ///
    /// The last three are there for the **coverage** assertions (A21-A23): a
    /// bundle that does not show both sides of the reuse boundary, both sides
    /// of the moved boundary and both sides of the size boundary cannot claim
    /// to cover the patch space, and the capture says so — which means the
    /// fixture has to show them or every capture against this world would be
    /// red.
    ///
    /// **`en-us` is published too**, because `--audio` defaults to `en-us` and
    /// the capture then fetches that manifest from *both* builds. A world that
    /// published only `game` would not be a narrower fixture — it would be a
    /// capture that cannot finish. The audio file is byte-identical across the
    /// pair so it does not take a shape away from `game`.
    pub fn all_shapes() -> World {
        let shared = File::new(
            "GenshinImpact_Data/StreamingAssets/Audio/en-us/vo_all",
            &[200, 201],
            256,
        );
        let old = Build {
            tag: "7.0.0".into(),
            build_id: "sMOCKold00".into(),
            fields: vec![
                (
                    "game".into(),
                    vec![
                        File::new("GenshinImpact_Data/Resources/browser_assets", &[1, 2], 512),
                        // The **dearest** capture on purpose: it is the one a
                        // budget can refuse without taking the coverage with
                        // it, because a `removed` capture contributes nothing
                        // to A21-A23. Its 32 KB body and digest are pinned byte
                        // for byte by `the_request_log_rows_are_what_the_spec_says_they_are`
                        // (capture_layout.rs) and by the 7.0.0 game-manifest
                        // checksum that derives from them, so the chunk cannot
                        // be shrunk without rewriting those assertions.
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/old.blk",
                            &[91],
                            32768,
                        ),
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/zero.blk",
                            &[70, 71],
                            512,
                        ),
                        File::new(
                            "GenshinImpact_Data/Plugins/HoYoChannel.dll",
                            &[10, 11, 12],
                            1024,
                        ),
                        // The eager trim: the file on disk is longer than the
                        // target and no reuse source reaches past the target's
                        // end, so the tail is dead weight.
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/eager.blk",
                            &[301, 302, 303, 304],
                            2048,
                        ),
                        // The deferred trim: the file on disk is longer than the
                        // target and the reused chunk's source reaches past the
                        // target's end, so the trim has to wait for the end.
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/deferred.blk",
                            &[311, 312, 313],
                            1024,
                        ),
                        // The content moves **later** and the file grows: the
                        // descending write order, and a file the repair has to
                        // extend.
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/grow.blk",
                            &[321, 322],
                            1024,
                        ),
                    ],
                ),
                ("en-us".into(), vec![shared.clone()]),
            ],
        };
        let new = Build {
            tag: "7.1.0".into(),
            build_id: "sMOCKnew00".into(),
            fields: vec![
                (
                    "game".into(),
                    vec![
                        File::new("GenshinImpact_Data/Resources/browser_assets", &[1, 2], 512),
                        // Present only on the new side: `added`.
                        File::new("GenshinImpact_Data/Plugins/libNew.dll", &[90], 256),
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/zero.blk",
                            &[80, 81],
                            512,
                        ),
                        File::new(
                            "GenshinImpact_Data/Plugins/HoYoChannel.dll",
                            &[99, 11, 12],
                            1024,
                        ),
                        // One chunk shorter, and the one it drops is not a
                        // source: nothing the repair reads lives in the tail.
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/eager.blk",
                            &[301, 302, 305],
                            2048,
                        ),
                        // The reused chunk moves **earlier** — its source is at
                        // the old file's end, past the target's end — and the
                        // file is one chunk shorter.
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/deferred.blk",
                            &[313, 314],
                            1024,
                        ),
                        // One chunk longer, and the two that survive move
                        // **later**: the content shifts up the file.
                        File::new(
                            "GenshinImpact_Data/StreamingAssets/blocks/grow.blk",
                            &[323, 321, 322],
                            1024,
                        ),
                    ],
                ),
                ("en-us".into(), vec![shared]),
            ],
        };
        World {
            new,
            old,
            other_tags: vec!["6.7.0".into()],
            old_in_diff_tags: true,
            audio_pkg_scan_dir: "GenshinImpact_Data/Persistent".into(),
        }
    }

    /// [`World::all_shapes`] with no audio field at all.
    ///
    /// Only usable with `--audio none`: with the default, the capture asks for an
    /// `en-us` manifest this world does not publish, and refuses. That refusal is
    /// correct and is not what this world is for.
    pub fn game_only() -> World {
        let mut w = World::all_shapes();
        w.old.fields.retain(|(f, _)| f == "game");
        w.new.fields.retain(|(f, _)| f == "game");
        w
    }

    /// [`World::all_shapes`] plus one path that answers no matrix class.
    ///
    /// The path is a one-chunk file whose content changes on both sides: one
    /// new chunk, no reuse. `Class::all_satisfied_by` is legitimately empty for
    /// that shape, so a `--files` request for it must be recorded as *unscored*
    /// rather than dropped between `requested` and `skipped`. Nothing else in
    /// the world changes, so the six ordinary classes are still capturable.
    pub fn with_degenerate_file() -> World {
        let mut w = World::all_shapes();
        for (build, seed) in [(&mut w.old, 777), (&mut w.new, 778)] {
            build
                .fields
                .iter_mut()
                .find(|(field, _)| field == "game")
                .expect("the all-shapes world publishes game")
                .1
                .push(File::new(DEGENERATE_PATH, &[seed], 256));
        }
        w
    }

    /// A second path that is new-only, so two forced paths share `added`.
    pub const SECOND_ADDED: &str = "GenshinImpact_Data/Plugins/libSecond.dll";

    /// [`World::all_shapes`] with a **second** file that is new-only.
    ///
    /// Two distinct `--files` paths then share one primary class (`added`), which
    /// is the case where two forced paths share one primary class (`added`):
    /// without per-class numbering the second patchmap replaces the first while
    /// the run still reports two captures. Nothing else in the world changes, so
    /// every other class keeps its ordinary candidate.
    pub fn with_two_added_files() -> World {
        let mut w = World::all_shapes();
        w.new
            .fields
            .iter_mut()
            .find(|(field, _)| field == "game")
            .expect("the all-shapes world publishes game")
            .1
            .push(File::new(Self::SECOND_ADDED, &[92], 256));
        w
    }

    fn by_tag(&self, tag: &str) -> Option<&Build> {
        if self.new.tag == tag {
            Some(&self.new)
        } else if self.old.tag == tag {
            Some(&self.old)
        } else {
            None
        }
    }

    /// `diff_tags` as `getGameBranches` will report them.
    pub fn diff_tags(&self) -> Vec<String> {
        let mut v = vec![self.old.tag.clone()];
        if !self.old_in_diff_tags {
            v.clear();
        }
        v.extend(self.other_tags.iter().cloned());
        v
    }

    /// The manifest archive for one build's one field: the bytes a CDN would
    /// serve, plus the declaration `getBuild` makes about them.
    pub fn archive(&self, build: &Build, field: &str) -> Result<Archive, String> {
        let files = build
            .field(field)
            .ok_or_else(|| format!("{} publishes no manifest for {field}", build.tag))?;
        let pb = gircap::manifest::proto::SophonChunkManifest {
            chuncks: files
                .iter()
                .map(|f| gircap::manifest::proto::SophonChunkFile {
                    file: f.path.clone(),
                    chunks: f.with_offsets(),
                    is_folder: false,
                    size: f.size(),
                    md5: f.md5(),
                })
                .collect(),
        };
        use prost::Message as _;
        let plain = pb.encode_to_vec();
        let raw = zstd::encode_all(plain.as_slice(), 3).map_err(|e| e.to_string())?;
        let checksum = gircap::hash::md5_hex(&plain);
        let prefix = gircap::hash::md5_hex(format!("manifest/{}/{}", build.tag, field).as_bytes())
            [..16]
            .to_string();
        Ok(Archive {
            id: format!("manifest_{prefix}_{checksum}"),
            checksum,
            raw,
            plain_len: plain.len() as i64,
            file_count: files.len() as i64,
            chunk_count: files.iter().map(|f| f.chunks.len() as i64).sum(),
            // The same arithmetic `manifest::decode_counts` applies to the same
            // declarations: a negative file contributes zero, and the sums
            // saturate. A5 compares these against the decoder's totals, so a
            // mock that summed the raw signed values would fail the capture for
            // a fixture bug rather than for a tool bug.
            uncompressed_size: files.iter().fold(0_u64, |total, f| {
                total.saturating_add(f.size().max(0) as u64)
            }),
            compressed_size: files
                .iter()
                .flat_map(|f| &f.chunks)
                .fold(0_u64, |total, c| {
                    total.saturating_add(c.compressed_size.max(0) as u64)
                }),
        })
    }

    /// Every chunk's declared `(uncompressed_md5, uncompressed_size)`.
    ///
    /// Read from the same values the manifests encode, by id, so a test can
    /// compare an index row against the declaration it came from rather than
    /// restating a number. `uncompressed_size` is clamped here exactly as
    /// `BlobProps::unmeasured` clamps it.
    pub fn declared(&self) -> BTreeMap<String, (String, u64)> {
        let mut out = BTreeMap::new();
        for build in [&self.new, &self.old] {
            for (_, files) in &build.fields {
                for f in files {
                    for c in &f.chunks {
                        out.insert(
                            c.id.clone(),
                            (
                                c.uncompressed_md5.clone(),
                                c.declared_uncompressed_size().max(0) as u64,
                            ),
                        );
                    }
                }
            }
        }
        out
    }

    /// Replace the bytes the CDN serves for `chunk_id`, leaving the manifest
    /// declaration alone. Returns whether any build published that id.
    pub fn mis_serve(&mut self, chunk_id: &str, bytes: Vec<u8>) -> bool {
        let mut found = false;
        for build in [&mut self.new, &mut self.old] {
            for (_, files) in &mut build.fields {
                for f in files {
                    for c in &mut f.chunks {
                        if c.id == chunk_id {
                            c.served = Some(bytes.clone());
                            found = true;
                        }
                    }
                }
            }
        }
        found
    }
}

/// One manifest archive and the numbers `getBuild` declares about it.
#[derive(Debug, Clone)]
pub struct Archive {
    /// `manifest_<16 hex>_<32 hex>`.
    pub id: String,
    /// `md5(decompressed)`.
    pub checksum: String,
    /// The zstd frame.
    pub raw: Vec<u8>,
    /// `len(decompressed)`.
    pub plain_len: i64,
    /// `len(files)`.
    pub file_count: i64,
    /// `Σ chunk count`.
    pub chunk_count: i64,
    /// `Σ file size`.
    pub uncompressed_size: u64,
    /// `Σ chunk compressed_size`.
    pub compressed_size: u64,
}

// --------------------------------------------------------------- the server

/// One request the mock answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// The path, without the query.
    pub path: String,
    /// The query string, without the `?`.
    pub query: String,
    /// The `tag` query parameter, when there was one.
    pub tag: Option<String>,
}

impl Recorded {
    /// Whether this request was for a chunk blob.
    pub fn is_chunk(&self) -> bool {
        self.path.contains("/chunk/")
    }
}

/// A running mock. Dropping it stops the listener.
pub struct Mock {
    base: String,
    stop: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<Recorded>>>,
    /// Whether a TCP listener and serving threads were actually started.
    /// Only used to skip the Drop-time unblock probe for an in-memory mock.
    listening: bool,
    /// Kept so `fail_once_for` can arm a route after the listener is already
    /// bound. Shared with the serving threads, which is why the map inside is
    /// behind its own lock.
    table: Arc<Table>,
}

impl Mock {
    /// Bind `127.0.0.1:0` and serve `world` on a background thread.
    ///
    /// Port 0 rather than a chosen port: a fixed port makes the suite fail when
    /// two of its tests run at once, and this suite does run its tests in
    /// parallel.
    pub fn start(world: World) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let base = format!("http://{}", listener.local_addr().expect("local addr"));
        let stop = Arc::new(AtomicBool::new(false));
        let log = Arc::new(Mutex::new(Vec::new()));
        let table = Arc::new(build_table(&world, &base));

        let thread_stop = Arc::clone(&stop);
        let thread_log = Arc::clone(&log);
        let thread_table = Arc::clone(&table);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let table = Arc::clone(&thread_table);
                let log = Arc::clone(&thread_log);
                std::thread::spawn(move || {
                    let _ = serve(stream, &table, &log);
                });
            }
        });

        Mock {
            base,
            stop,
            log,
            listening: true,
            table,
        }
    }

    /// Build the same world with **no listener and no threads**.
    ///
    /// The in-memory transport answers from the very same `Table` the TCP
    /// server uses (`respond`, `no_such_key`, `content_type` and the
    /// fail-once map are shared, not reimplemented), so byte-exact bodies,
    /// status codes and content types are identical. The only difference is the
    /// HTTP framing layer, which is exactly what the injected transport
    /// replaces. A fixed five-digit port keeps `base` the same length as a real
    /// loopback base, so body lengths and therefore every `bytes_in` pin are
    /// unchanged.
    pub fn in_memory(world: World) -> Mock {
        let base = "http://127.0.0.1:50000".to_string();
        let log = Arc::new(Mutex::new(Vec::new()));
        let table = Arc::new(build_table(&world, &base));
        Mock {
            base,
            stop: Arc::new(AtomicBool::new(false)),
            log,
            listening: false,
            table,
        }
    }

    /// A transport handle backed by this mock's table, for [`dispatch_with`].
    pub fn transport(&self) -> Arc<dyn HttpTransport> {
        Arc::new(MemoryTransport {
            table: Arc::clone(&self.table),
            log: Arc::clone(&self.log),
            base: self.base.clone(),
        })
    }

    /// Answer the next request whose path contains `needle` with `503`, once.
    ///
    /// A `5xx` is what `RetryPolicy` retries, so this is the shape that exercises
    /// the retry loop end to end: the first attempt fails, the second succeeds,
    /// and the request log must say so in `attempts`. Call it before `start` has
    /// served anything, which is immediately after construction.
    ///
    /// Takes and returns `Self` so it chains:
    /// `Mock::start(world).fail_once_for("/chunk/")`.
    pub fn fail_once_for(self, needle: &str) -> Mock {
        let mut f = self
            .table
            .flaky
            .lock()
            .expect("the flaky table is not poisoned");
        let slot = f.entry(needle.to_string()).or_insert(0);
        *slot = 1;
        drop(f);
        self
    }

    /// `http://127.0.0.1:<port>`.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The `--hyp-base` a capture against this mock should be given.
    pub fn hyp_base(&self) -> String {
        format!("{}/hyp", self.base)
    }

    /// The `--sophon-base` a capture against this mock should be given.
    pub fn sophon_base(&self) -> String {
        format!("{}/sophon", self.base)
    }

    /// Every request answered so far, in order.
    pub fn requests(&self) -> Vec<Recorded> {
        self.log
            .lock()
            .expect("the request log is not poisoned")
            .clone()
    }

    /// The chunk ids requested, in order.
    pub fn chunk_requests(&self) -> Vec<String> {
        self.requests()
            .iter()
            .filter(|r| r.is_chunk())
            .map(|r| r.path.rsplit('/').next().unwrap_or_default().to_string())
            .collect()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if self.listening {
            // One connect-and-close is enough to unblock `incoming()`. An
            // in-memory mock has no listener, so it needs no unblock probe.
            let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        }
    }
}

/// An in-memory [`HttpTransport`] that answers from a mock [`Table`].
///
/// This is the test half of the injectable transport seam. It reuses the exact
/// routing and body construction the TCP server uses — there is no second
/// implementation of "what does this fixture serve" to drift — and builds an
/// [`HttpResponse`] carrying the same three headers `write_response` sends. It
/// also logs every request and honours the fail-once map, so `mock.requests()`
/// and a retried 503 behave identically on both paths.
struct MemoryTransport {
    table: Arc<Table>,
    log: Arc<Mutex<Vec<Recorded>>>,
    base: String,
}

impl fmt::Debug for MemoryTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryTransport")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl HttpTransport for MemoryTransport {
    fn send(&self, request: HttpRequest) -> SendFuture {
        let table = Arc::clone(&self.table);
        let log = Arc::clone(&self.log);
        let base = self.base.clone();
        let url = request.url;
        Box::pin(async move {
            // Nothing is allowed to escape the fixture, exactly like the TCP
            // mock's loopback-only listener. A URL under another host is a
            // transport error rather than a real request.
            if !url.starts_with(&base) {
                return Err(format!(
                    "in-memory transport refuses URL outside {base}: {url}"
                ));
            }
            let (path, query) = split_url(&url);
            let tag = query_param(&query, "tag");
            log.lock()
                .expect("the request log is not poisoned")
                .push(Recorded {
                    path: path.clone(),
                    query: query.clone(),
                    tag: tag.clone(),
                });

            let (status, content_type, body) = if table.take_flaky_failure(&path).is_some() {
                (503, "text/plain", b"armed to fail once".to_vec())
            } else {
                match respond(&table, &path, &query, tag.as_deref()) {
                    None if is_blob_route(&path) => (404, "application/xml", no_such_key(&path)),
                    None => (
                        404,
                        "text/plain",
                        format!("no route for {path}").into_bytes(),
                    ),
                    Some(bytes) => (200, content_type(&path), bytes),
                }
            };
            let final_url = url;
            Ok(HttpResponse::in_memory(
                status,
                final_url,
                Some(content_type),
                vec![body],
            ))
        })
    }
}

/// Split an absolute URL into the path (with its leading `/`) and query the way
/// the TCP server's request-line parser does, so [`respond`] and [`Recorded`]
/// see the same values on both paths.
fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let target = match rest.split_once('/') {
        Some((_, path)) => format!("/{path}"),
        None => "/".to_string(),
    };
    match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target, String::new()),
    }
}

/// Everything the server answers with, indexed the way it is looked up.
struct Table {
    hyp: BTreeMap<&'static str, Vec<u8>>,
    builds: BTreeMap<String, Vec<u8>>,
    archives: BTreeMap<(String, String), Archive>,
    chunks: BTreeMap<(String, String), Blob>,
    /// The `game_ids[]` this mock answers for; anything else is the bad-id case.
    game_id: String,
    /// The `package_id` this mock answers for; anything else is `-201`.
    package_id: String,
    /// Paths that should fail before they succeed: a substring of the request
    /// path, and how many more `503`s it still owes.
    ///
    /// **Mutable, because "fail once then succeed" is the only way to observe a
    /// retry.** Every other fixture here answers consistently — which is right
    /// for what those tests measure, and which is why `attempts` was `1` on every
    /// row of the pinned request log. A bug that *dropped* a retry was therefore
    /// indistinguishable from one that never happened.
    ///
    /// Interior mutability because the table is shared as `Arc<Table>` across a
    /// thread per connection, and whichever connection serves the request is the
    /// one that has to decrement the count.
    flaky: Mutex<BTreeMap<String, usize>>,
}

/// Every `matching_field` any build publishes, in a stable order.
///
/// **Derived from the builds, never declared separately.** A mock that can
/// disagree with itself hides a real bug behind a fabricated one: the branch
/// must publish exactly the fields the build carries, or the capture fails at
/// the join for a reason that has nothing to do with the code under test.
fn published_fields(world: &World) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for build in [&world.new, &world.old] {
        for (field, _) in &build.fields {
            if !v.contains(field) {
                v.push(field.clone());
            }
        }
    }
    v
}

/// The measured ids, so every body names them the same way.
const GAME: &str = "gopR6Cufr3";
const BIZ: &str = "hk4e_global";

/// A JSON value whose **object keys keep the order they were written in**.
///
/// `serde_json::json!` cannot do this: it builds a `Value`, a `Value`'s object is
/// a `BTreeMap`, and a `BTreeMap` sorts. That is invisible to the tool and fatal
/// to the claim this mock exists to support - "the body is stored exactly as
/// served" is only checkable if the server had an order to preserve. So the mock
/// renders its own bodies, in the measured key order, and
/// `the_api_bodies_are_byte_exact_and_the_generated_one_is_marked` compares them
/// against what the tool wrote.
///
/// Forty lines is the right price for removing a whole class of "the fixture is
/// malformed" debugging, and it is why these bodies are not string literals with
/// hand-counted braces.
#[derive(Debug, Clone)]
pub enum J {
    /// `true` / `false`.
    Bool(bool),
    /// A non-negative integer.
    Num(u64),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<J>),
    /// An object, in the order given.
    Obj(Vec<(String, J)>),
}

impl J {
    /// A string value.
    fn s(v: impl Into<String>) -> J {
        J::Str(v.into())
    }

    /// An array of strings.
    fn strings<I: IntoIterator<Item = S>, S: Into<String>>(items: I) -> J {
        J::Arr(items.into_iter().map(|i| J::Str(i.into())).collect())
    }

    /// Render compactly, the way a server sends a response body.
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Num(n) => out.push_str(&n.to_string()),
            J::Str(s) => write_string(s, out),
            J::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            J::Obj(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// A JSON string, escaped the way `serde_json` escapes one.
///
/// Hand-rolled rather than delegating, because the point is that this module does
/// not depend on a serializer that would reorder things - and because a mock
/// that silently mis-escapes a body would fail a byte-exactness test for the
/// wrong reason.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `{"retcode":0,"message":"OK","data":…}`, the wrapper every endpoint returns.
fn envelope(data: J) -> J {
    J::Obj(vec![
        ("retcode".into(), J::Num(0)),
        ("message".into(), J::s("OK")),
        ("data".into(), data),
    ])
}

/// `data.game` - the game id, in the order the measured bodies carry it.
fn game_ref() -> J {
    J::Obj(vec![("id".into(), J::s(GAME)), ("biz".into(), J::s(BIZ))])
}

/// The body as text, validated on the way out.
/// A malformed literal then fails here, with the mock's own panic naming it,
/// rather than three layers downstream as a confusing parse error from the tool.
fn compact(text: String) -> Vec<u8> {
    serde_json::from_str::<serde_json::Value>(&text).expect("the mock's own body is valid JSON");
    text.into_bytes()
}

/// Render the world's responses.
fn build_table(world: &World, base: &str) -> Table {
    // `categories` is derived from what the builds actually publish, never
    // declared separately: a mock that claims to publish a manifest it does not
    // carry would fail the capture at the join for a reason that has nothing to
    // do with the code under test.
    let categories: Vec<J> = published_fields(world)
        .iter()
        .map(|f| {
            J::Obj(vec![
                (
                    "category_id".into(),
                    J::s(if f == "game" { "10016" } else { "10018" }),
                ),
                ("matching_field".into(), J::s(f.clone())),
                (
                    "type".into(),
                    J::s(if f == "game" {
                        "CATEGORY_TYPE_RESOURCE"
                    } else {
                        "CATEGORY_TYPE_AUDIO"
                    }),
                ),
                ("scenarios".into(), J::Arr(Vec::new())),
            ])
        })
        .collect();
    let mut hyp = BTreeMap::new();
    hyp.insert(
        "getGameConfigs",
        compact(
            envelope(J::Obj(vec![
                ("game".into(), game_ref()),
                (
                    "launch_configs".into(),
                    J::Arr(vec![J::Obj(vec![
                        ("game".into(), game_ref()),
                        ("exe_file_name".into(), J::s("GenshinImpact.exe")),
                        ("installation_dir".into(), J::s("Genshin Impact")),
                        (
                            "audio_pkg_scan_dir".into(),
                            J::s(world.audio_pkg_scan_dir.clone()),
                        ),
                        (
                            "audio_pkg_res_dir".into(),
                            J::s("GenshinImpact_Data/StreamingAssets/Audio"),
                        ),
                        // The one launcher-side value that can stop a capture.
                        // See `GameConfigs::require_chunk_mode`.
                        ("default_download_mode".into(), J::s("DOWNLOAD_MODE_CHUNK")),
                        ("enable_audio_pkg_mgmt".into(), J::Bool(true)),
                        ("enable_ldiff".into(), J::Bool(true)),
                    ])]),
                ),
            ]))
            .render(),
        ),
    );
    hyp.insert(
        "getGameBranches",
        compact(
            envelope(J::Obj(vec![
                ("game".into(), game_ref()),
                (
                    "game_branches".into(),
                    J::Arr(vec![J::Obj(vec![
                        ("game".into(), game_ref()),
                        (
                            "main".into(),
                            J::Obj(vec![
                                ("package_id".into(), J::s("ScSYQBFhu9")),
                                ("branch".into(), J::s("main")),
                                ("password".into(), J::s("sMOCKpassword")),
                                ("tag".into(), J::s(world.new.tag.clone())),
                                ("diff_tags".into(), J::strings(world.diff_tags())),
                                ("categories".into(), J::Arr(categories)),
                            ]),
                        ),
                    ])]),
                ),
            ]))
            .render(),
        ),
    );
    hyp.insert(
        "getGameDeprecatedFileConfigs",
        compact(
            envelope(J::Obj(vec![
                ("game".into(), game_ref()),
                (
                    "deprecated_file_configs".into(),
                    J::Arr(vec![J::Obj(vec![
                        ("game".into(), game_ref()),
                        ("deprecated_files".into(), J::Arr(Vec::new())),
                    ])]),
                ),
            ]))
            .render(),
        ),
    );
    hyp.insert(
        "getGamePackages",
        compact(
            envelope(J::Obj(vec![
                ("game".into(), game_ref()),
                ("game_packages".into(), J::Arr(Vec::new())),
            ]))
            .render(),
        ),
    );
    let mut builds = BTreeMap::new();
    let mut archives = BTreeMap::new();
    let mut chunks: BTreeMap<(String, String), Blob> = BTreeMap::new();
    for build in [&world.new, &world.old] {
        let mut manifests = Vec::new();
        for (field, files) in &build.fields {
            let archive = world
                .archive(build, field)
                .expect("the world is self-consistent");
            for f in files {
                for c in &f.chunks {
                    // Keyed by build as well as id: the same content under two
                    // builds is one fetch but **two addresses**, and only the
                    // second half of that is what the CDN enforces.
                    chunks
                        .entry((build.build_id.clone(), c.id.clone()))
                        .or_insert_with(|| c.clone());
                }
            }
            archives.insert(
                (build.build_id.clone(), archive.id.clone()),
                archive.clone(),
            );
            let download = |kind: &str| {
                J::Obj(vec![
                    ("encryption".into(), J::Num(0)),
                    ("password".into(), J::s("")),
                    // 1 = zstd, 0 = plaintext. Every value the tool refuses is
                    // a different number, so the mock uses the measured ones.
                    ("compression".into(), J::Num(1)),
                    (
                        "url_prefix".into(),
                        J::s(format!("{base}/cdn/{}/{kind}", build.build_id)),
                    ),
                    ("url_suffix".into(), J::s("")),
                ])
            };
            manifests.push(J::Obj(vec![
                (
                    "category_id".into(),
                    J::s(if field == "game" { "10016" } else { "10018" }),
                ),
                ("category_name".into(), J::s(field.clone())),
                (
                    "manifest".into(),
                    J::Obj(vec![
                        ("id".into(), J::s(archive.id.clone())),
                        ("checksum".into(), J::s(archive.checksum.clone())),
                        // Every `getBuild` size is measured as a **string**, and
                        // `IntOrStr` exists precisely because a `u64` field would
                        // parse these as null.
                        (
                            "compressed_size".into(),
                            J::s(archive.raw.len().to_string()),
                        ),
                        (
                            "uncompressed_size".into(),
                            J::s(archive.plain_len.to_string()),
                        ),
                    ]),
                ),
                ("chunk_download".into(), download("chunk")),
                ("manifest_download".into(), download("manifest")),
                ("matching_field".into(), J::s(field.clone())),
                (
                    "stats".into(),
                    J::Obj(vec![
                        (
                            "compressed_size".into(),
                            J::s(archive.compressed_size.to_string()),
                        ),
                        (
                            "uncompressed_size".into(),
                            J::s(archive.uncompressed_size.to_string()),
                        ),
                        ("file_count".into(), J::s(archive.file_count.to_string())),
                        ("chunk_count".into(), J::s(archive.chunk_count.to_string())),
                    ]),
                ),
            ]));
        }
        builds.insert(
            build.tag.clone(),
            compact(
                envelope(J::Obj(vec![
                    ("build_id".into(), J::s(build.build_id.clone())),
                    ("tag".into(), J::s(build.tag.clone())),
                    ("manifests".into(), J::Arr(manifests)),
                ]))
                .render(),
            ),
        );
    }

    // The ids the mock answers for, **read back out of the bodies it just built**
    // rather than restated here. `published_fields` above exists because a mock
    // carrying its own idea of the world eventually disagrees with itself; the
    // `game_ids[]` / `package_id` checks are exactly that shape, and hardcoding
    // them would make a typo look like a measured behaviour.
    let package_id = serde_json::from_slice::<serde_json::Value>(&hyp["getGameBranches"])
        .ok()
        .and_then(|v| {
            v.pointer("/data/game_branches/0/main/package_id")
                .and_then(|p| p.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    assert!(
        !package_id.is_empty(),
        "the branches body must carry a package_id, or every getBuild would 400"
    );
    Table {
        hyp,
        builds,
        archives,
        chunks,
        game_id: GAME.to_string(),
        package_id,
        // Empty until a test arms a route with `Mock::fail_once_for`.
        flaky: Mutex::new(BTreeMap::new()),
    }
}

/// A JSON API envelope carrying an error: `{"retcode":n,"message":…,"data":null}`.
///
/// `null` data on an error is the measured spelling and is what a consumer
/// distinguishing "no build" from "no tag" has to go on.
fn envelope_error(retcode: i64, message: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "data": serde_json::Value::Null,
        "message": message,
        "retcode": retcode,
    }))
    .expect("render json")
}

/// A JSON API envelope carrying `data` at an explicit retcode.
fn envelope_with_data(retcode: i64, message: &str, data: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "data": data,
        "message": message,
        "retcode": retcode,
    }))
    .expect("render json")
}

/// Serve requests on one connection until the peer closes it or asks to close.
///
/// **Keep-alive, because the real servers keep alive.** Answering one request
/// per connection and `Connection: close` unconditionally costs a fresh TCP
/// connection per chunk blob, which is not just unfaithful to the CDN — it
/// measures Windows' connect path rather than anything about the fetcher.
///
/// The client's pool is never the thing at fault: `Fetcher` builds one
/// `reqwest::Client` and every clone shares it, so a mock that refuses to let
/// the pool be used is the only way to see this.
///
/// A request body is still never read: this client sends none, and a body-carrying
/// request on a reused connection would desynchronise the stream. The reader is
/// only advanced to the blank line that ends the headers.
fn serve(mut stream: TcpStream, table: &Table, log: &Mutex<Vec<Recorded>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line)? == 0 {
            return Ok(()); // peer closed: the normal end of a pooled connection
        }
        if request_line.trim().is_empty() {
            continue; // tolerate a stray CRLF between pipelined requests
        }

        // Drain the headers, noting `Connection:` so the reply can agree.
        let mut client_says_close = false;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                return Ok(());
            }
            if line.trim().is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("connection:") && lower.contains("close") {
                client_says_close = true;
            }
        }

        let target = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("/")
            .to_string();
        let (path, query) = match target.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (target.clone(), String::new()),
        };
        let tag = query_param(&query, "tag");
        log.lock().expect("log").push(Recorded {
            path: path.clone(),
            query: query.clone(),
            tag: tag.clone(),
        });

        // HTTP/1.1 keeps alive unless someone asks otherwise; HTTP/1.0 is the
        // other way round. The mock is HTTP/1.1, and this client is 1.1.
        let keep_alive = !client_says_close;

        // An armed route answers `503` until its count runs out, which is what
        // makes the retry loop observable: `RetryPolicy` retries a `5xx`, so the
        // request succeeds on the second attempt and the row must say `2`.
        if table.take_flaky_failure(&path).is_some() {
            // The response is complete either way, so honour its `Connection:`
            // header: returning here would close a connection the reply just
            // advertised as reusable. The pool would then hand the retry a dead
            // socket, which turns the armed `503` into a transport error and
            // makes the retry count depend on whether the socket was reused.
            write_response(
                &mut stream,
                503,
                "text/plain",
                b"armed to fail once",
                keep_alive,
            )?;
            if !keep_alive {
                return Ok(());
            }
            continue;
        }

        let outcome = match respond(table, &path, &query, tag.as_deref()) {
            // A blob route's miss is the measured shape, not a generic one: the
            // real CDN (`Server: AliyunOSS`) answers `404` with
            // `application/xml` and a `NoSuchKey` envelope, and that is what
            // `api/error_shapes.json` records and a replayer will replay. A
            // `text/plain` stand-in would let the mock agree with itself about a
            // detail the mock cannot know.
            None if is_blob_route(&path) => write_response(
                &mut stream,
                404,
                "application/xml",
                &no_such_key(&path),
                keep_alive,
            ),
            None => write_response(
                &mut stream,
                404,
                "text/plain",
                format!("no route for {path}").as_bytes(),
                keep_alive,
            ),
            Some(bytes) => {
                write_response(&mut stream, 200, content_type(&path), &bytes, keep_alive)
            }
        };

        outcome?;
        if !keep_alive {
            return Ok(());
        }
    }
}

/// Whether `path` belongs to the CDN's two blob namespaces rather than the JSON
/// APIs. `api/` and the `get*` endpoints are the JSON side.
fn is_blob_route(path: &str) -> bool {
    !path.contains("getBuild") && !path.contains("getGame")
}

impl Table {
    /// Consume one armed failure for `path`, if any route matches it.
    ///
    /// Longest needle first, so arming `/chunk/` does not lose to a narrower
    /// `/chunk/abc` armed for the same path — the more specific answer is the
    /// one the test meant.
    fn take_flaky_failure(&self, path: &str) -> Option<usize> {
        let mut f = self.flaky.lock().expect("the flaky table is not poisoned");
        let mut best: Option<(usize, String)> = None;
        for needle in f.keys() {
            if path.contains(needle.as_str())
                && best.as_ref().is_none_or(|(len, _)| needle.len() > *len)
            {
                best = Some((needle.len(), needle.clone()));
            }
        }
        let (_, needle) = best?;
        let slot = f.get_mut(&needle)?;
        if *slot == 0 {
            return None;
        }
        *slot -= 1;
        Some(*slot)
    }
}

/// The AliyunOSS `NoSuchKey` envelope, with the fields that are volatile per
/// request filled in: `Key` echoes the path asked for, which is why the real
/// body's length varies and why `error_shapes.json` marks it un-replayable.
fn no_such_key(path: &str) -> Vec<u8> {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message><Key>{path}</Key><RequestId>5C0BD9A0C3E0E0A2</RequestId><HostId>cdn-mock</HostId></Error>"#
    )
    .into_bytes()
}

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".json") || path.contains("getBuild") || path.contains("getGame") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}

fn respond(table: &Table, path: &str, query: &str, tag: Option<&str>) -> Option<Vec<u8>> {
    // Whether this request asked for the game the mock knows. Hoisted because
    // both `hyp` endpoints need it and the guard is long enough to read better
    // as a name.
    let known_game = query_param(query, "game_ids[]").as_deref() == Some(table.game_id.as_str());
    let known_package =
        query_param(query, "package_id").as_deref() == Some(table.package_id.as_str());
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segs.as_slice() {
        ["hyp", endpoint] => match *endpoint {
            // A `game_ids[]` the mock does not know is measured to answer 200
            // with `retcode -1001` from getGameConfigs, and 200 with
            // `retcode 0` and an **empty** `game_branches` from
            // getGameBranches. The second is not a failure and is recorded as
            // such in `api/error_shapes.json`; the mock has to be able to say
            // both, or the document is the mock's opinion rather than a
            // measurement.
            "getGameConfigs" if !known_game => Some(envelope_error(-1001, "invalid params")),
            "getGameBranches" if !known_game => Some(envelope_with_data(
                0,
                "OK",
                serde_json::json!({ "game_branches": [] }),
            )),
            other => table.hyp.get(other).cloned(),
        },
        ["sophon", "getBuild"] => {
            // A `package_id` the mock does not know: 200 with `retcode -201`,
            // checked before the tag because a request with both wrong answers
            // with the parameter the API validates first.
            if !known_package {
                return Some(envelope_error(-201, "invalid params"));
            }
            let tag = tag?;
            match table.builds.get(tag) {
                Some(b) => Some(b.clone()),
                // The measured way of saying "no such tag": HTTP 200, retcode -202.
                None => Some(envelope_error(-202, "not found")),
            }
        }
        // **Scoped by build id, deliberately.** The measured CDN 404s a valid chunk id
        // under the *other* build's prefix — that is what makes the build id part
        // of a chunk's address, and it is what `api/error_shapes.json`'s
        // `blob_wrong_build` row records. A mock that served any known id under
        // any prefix would make that row say `200`, and the row would be a lie
        // about the only thing it exists to state.
        ["cdn", build, "manifest", id] => table
            .archives
            .get(&(build.to_string(), (*id).to_string()))
            .map(|a| a.raw.clone()),
        ["cdn", build, "chunk", id] => table
            .chunks
            .get(&(build.to_string(), (*id).to_string()))
            .map(|c| c.served.clone().unwrap_or_else(|| c.raw.clone())),
        _ => None,
    }
}

/// Look up one decoded query value. The standard form parser handles `%XX`,
/// `+` and lossy UTF-8; bracketed keys stay literal because that is the spelling
/// the recorded request URLs use.
fn query_param(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(k, v)| (k == key).then(|| v.into_owned()))
}

/// A minimal, correct HTTP/1.1 response.
fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    keep_alive: bool,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let connection = if keep_alive { "keep-alive" } else { "close" };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: {connection}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}
