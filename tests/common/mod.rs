//! Offline mock API for integration/e2e tests.
//!
//! Spins a tiny HTTP server on `127.0.0.1:<ephemeral>` (no external network)
//! that mimics the live HoYoPlay + Sophon shapes used by `girpr`:
//!
//! - `GET <hyp>/getGameConfigs` -> `{"launch_configs":[...]}`
//! - `GET <hyp>/getGameBranches` -> `{"game_branches":[...]}`
//! - `GET <sophon>/getBuild[?tag=]` -> chunk build, or `retcode -202` for
//!   unknown tags (exercises the local-build fallback in `repair::run`)
//! - `GET <hyp>/getGameDeprecatedFileConfigs` -> deprecated list
//! - `GET <manifest_prefix>/<manifest_id>` -> zstd(protobuf manifest)
//! - `GET <chunk_prefix>/<chunk_id>` -> zstd(chunk payload)
//!
//! Shapes were captured from the live `hk4e_global` endpoints (Sept 2026):
//! same wrapper (`retcode`/`message`/`data`), same node names, manifest
//! sizes as int-or-string, chunk/manifest URL prefix + id joining.

// The module is shared by several test binaries and each of them uses a
// different part of it, so most of it is dead code in any one of them.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use girpr::test_support::{
    Limits, NoSleep, PathScope, RepairMode, SophonChunk, SophonChunkFile, SophonChunkManifest,
    terminal_retcode,
};
use girpr::{Biz, OutputFormat, RunCtx, RunExit, RunFailure, Summary};
use prost::Message as _;

// ---------------------------------------------------------------------------
// shared test harness
// ---------------------------------------------------------------------------

/// Compile a path scope for a test, the way `Args::into_ctx` does.
pub fn scope(include: &[&str], exclude: &[&str]) -> PathScope {
    let inc: Vec<String> = include.iter().map(|s| s.to_string()).collect();
    let exc: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
    PathScope::new(&inc, &exc).expect("test globs must compile")
}

pub fn ctx_for(game_dir: std::path::PathBuf, mock: &MockServer, jobs: usize) -> RunCtx {
    ctx_for_biz(game_dir, mock, jobs, Biz::Hk4eGlobal)
}

pub fn ctx_for_biz(
    game_dir: std::path::PathBuf,
    mock: &MockServer,
    jobs: usize,
    biz: Biz,
) -> RunCtx {
    RunCtx {
        game_dir,
        biz,
        audio: HashSet::new(),
        audio_explicit: false,
        path_scope: PathScope::default(),
        limits: Limits::default(),
        repair_mode: RepairMode::Auto,
        repair_mode_explicit: false,
        jobs,
        check_only: false,
        scan: false,
        purge_only: false,
        fail_fast: false,
        dry_run: false,
        print_plan: false,
        plan_out: None,
        purge_after: false,
        purge_before: false,
        output: OutputFormat::Text,
        hyp_base_override: Some(mock.hyp_base.clone()),
        sophon_base_override: Some(mock.sophon_base.clone()),
        // Tests skip retry backoff: a failed fetch is retried immediately.
        sleep: Arc::new(NoSleep),
    }
}

pub fn load_counters(s: &Summary) -> (u64, u64, u64, u64, u64) {
    (
        s.files_skipped.load(Ordering::Relaxed),
        s.files_repaired.load(Ordering::Relaxed),
        s.files_failed.load(Ordering::Relaxed),
        s.download.settled(),
        s.deleted_extra_bytes.load(Ordering::Relaxed),
    )
}

/// The value of one `key=value` field on a `key=value` line.
pub fn field(line: &str, name: &str) -> u64 {
    line.split_whitespace()
        .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no {name} in {line}"))
}

/// A game dir that does not exist yet, pointed at by `parent/missing`.
pub fn absent_game_dir(parent: &std::path::Path) -> std::path::PathBuf {
    parent.join("not-created-yet")
}

mod run_lock;

// Each test binary uses one of the two helpers, so the other is unused here.
#[allow(unused_imports)]
pub use run_lock::{remove_run_lock, remove_run_lock_for_args};

/// Run the library's entry point, then remove the lock file it leaves in the
/// shared temp directory. See [`remove_run_lock`].
///
/// A read-only run takes no lock, so cleanup is skipped for it rather than
/// acquiring a lock just to find and delete the file that acquisition created.
pub async fn run(ctx: RunCtx) -> Result<(Summary, RunExit), RunFailure> {
    let game_dir = ctx.game_dir.clone();
    let takes_lock = run_lock::mode_takes_lock(ctx.readonly(), ctx.check_only, ctx.print_plan);
    let result = girpr::run(ctx).await;
    if takes_lock {
        run_lock::remove_run_lock(&game_dir);
    }
    result
}

/// Run the shipped binary. The endpoint overrides are the only seam a
/// *binary-level* test can use, and this is where that seam is exercised: the
/// library tests inject `RunCtx::hyp_base_override` directly,
/// which proves nothing about whether a flag reaches `HypClient`.
///
/// Async on purpose: a blocking `Command` inside `#[tokio::test]` would occupy
/// the single runtime thread the mock server needs to answer on.
pub async fn run_binary(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_girpr"));
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().await.expect("run girpr");
    remove_run_lock_for_args(args);
    (
        out.status.code().expect("process must exit normally"),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// ---------------------------------------------------------------------------
// tiny deterministic game fixture (2 files, 3 chunks, all in-memory)
// ---------------------------------------------------------------------------

pub struct FixtureChunk {
    pub id: String,
    pub data: Vec<u8>,
    pub offset: i64,
}

pub struct FixtureFile {
    pub rel: String,
    pub data: Vec<u8>,
    pub md5: String,
    pub chunks: Vec<FixtureChunk>,
}

pub struct GameFixture {
    pub latest_tag: String,
    pub manifest_id: String,
    pub manifest_zstd: Vec<u8>,
    pub manifest_checksum: String,
    pub files: Vec<FixtureFile>,
    /// chunk id -> zstd(compressed payload) bytes served by the mock
    pub chunk_store: HashMap<String, Vec<u8>>,
    /// chunk id -> zstd(payload with one byte flipped). Well-formed: same
    /// uncompressed length, different MD5, so the chunk decodes and then fails
    /// verification. That is what makes a file *retry* rather than just fail.
    pub corrupt_chunk_store: HashMap<String, Vec<u8>>,
    /// A second manifest, describing the **old** file's chunk layout, served
    /// for a non-latest tag. `None` = the server does not serve the local tag
    /// at all (`getBuild(local)` answers -202), which is the default.
    pub local_manifest: Option<FixtureManifest>,
    /// Additional manifests in the **latest** build, after the primary `game`
    /// one. Their metadata is part of `getBuild`'s answer, in this order.
    pub extra_latest_manifests: Vec<FixtureLatestManifest>,
    /// The old file's bytes — the thing the repair has to move — for the
    /// fixtures that have one.
    pub old_data: Option<(String, Vec<u8>)>,
    /// Every file that is **on disk**, by path.
    ///
    /// The old side's bytes are here rather than derived from the sides on
    /// purpose: the shapes the repair has to survive include a file that is
    /// longer on disk than its manifest describes (the dead tail the trim
    /// releases) and one that is shorter (the growth the repair writes).
    pub disk: BTreeMap<String, Vec<u8>>,
}

/// A second manifest: the old file's chunk layout.
pub struct FixtureManifest {
    pub id: String,
    pub zstd: Vec<u8>,
    pub checksum: String,
}

/// Another manifest in the same latest build, with its own fields and file
/// list. `served = false` advertises it in `getBuild` but answers `404` for its
/// body, which is what a build whose manifest is gone looks like.
pub struct FixtureLatestManifest {
    pub matching_field: String,
    pub category_id: String,
    pub id: String,
    pub zstd: Vec<u8>,
    pub checksum: String,
    pub served: bool,
}

/// The fixture's latest tag. The mock serves the chunk manifest for
/// `getBuild?tag=<LATEST_TAG>` and returns `retcode -202` for any other tag, so
/// writing this into `config.ini` is how a test says "the local build's
/// manifests are available" (i.e. chunk reuse is available).
pub const LATEST_TAG: &str = "9.9.9-test";

/// The local version a test writes into `config.ini` when its fixture has an
/// old side.
///
/// The mock answers `getBuild?tag=<requested>` with the local manifest tagged
/// with the **requested** tag, so any version a test writes resolves to a
/// matching local build. This name is the conventional choice, not a tag the
/// mock hardcodes.
pub const LOCAL_TAG: &str = "9.9.8-test";

/// A local version the standard fixture has no build for.
///
/// Any tag other than [`LATEST_TAG`] resolves to the fixture's local side when
/// it has one; the standard fixture has none, so this tag makes
/// `getBuild(local)` answer `-202` and the run fall back to a full fetch.
pub const UNSERVED_TAG: &str = "1.0.0-long-gone";

mod digest;

pub use digest::md5hex;

fn zstd_bytes(data: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(std::io::Cursor::new(data), 3).expect("zstd encode fixture")
}

// ---------------------------------------------------------------------------
// the fixture description
// ---------------------------------------------------------------------------
//
// A fixture is **described**, not built: the old side, the new side and the
// bytes on disk are three lists, and a shape is a shape rather than a builder
// of its own. That is what lets one mock serve a uniform shift, a mixed
// shift, a shrinking file and a file with a hole in its coverage — the four
// shapes the repair's own branches turn on — without four builders that
// each know a shape the others do not.

/// One chunk of a file: its bytes, where they land, and the id the mock
/// serves it under.
///
/// The id is part of the description because the mock's chunk store is keyed
/// by it and the repair's request log reports it: a test asserts against the
/// ids it named here.
#[derive(Clone, Debug)]
pub struct ChunkSpec {
    /// The id the store serves this chunk under.
    pub id: String,
    /// The chunk's bytes.
    pub payload: Vec<u8>,
    /// Where the chunk lands in the file.
    pub offset: i64,
}

impl ChunkSpec {
    /// A chunk with the given id, bytes and offset.
    pub fn new(id: &str, payload: Vec<u8>, offset: i64) -> Self {
        Self {
            id: id.to_string(),
            payload,
            offset,
        }
    }
}

/// One file: a path and the chunks it is made of.
#[derive(Clone, Debug)]
pub struct FileSpec {
    /// The path, relative to `--game-path`, `/`-separated.
    pub rel: String,
    /// The chunks, in manifest order.
    pub chunks: Vec<ChunkSpec>,
}

impl FileSpec {
    /// A file with the given path and chunks.
    pub fn new(rel: &str, chunks: Vec<ChunkSpec>) -> Self {
        Self {
            rel: rel.to_string(),
            chunks,
        }
    }
}

/// The ways a fixture's manifest can lie about itself.
///
/// A manifest is a claim about bytes the tool has not seen yet, so every
/// field in it is a hint. These are the ways a hint can be wrong, and each
/// one has a branch in the repair that has to survive it.
#[derive(Clone, Debug, Default)]
pub struct Lies {
    /// Chunk ids whose declared `uncompressed_md5` no payload can match: the
    /// digest gate has to refuse the bytes it is given.
    pub wrong_chunk_md5: Vec<String>,
    /// Chunk ids whose declared `compressed_size` is not the blob's length:
    /// the size is a hint, and the real length is what crosses the wire.
    pub wrong_chunk_size: Vec<String>,
    /// Chunk ids the store serves as a body that is **not a zstd frame**: the
    /// decode has to fail on it rather than land garbage in the install.
    pub not_zstd: Vec<String>,
    /// Chunk ids whose declared `offset` is `i64::MAX`.
    ///
    /// The range they claim ends past `i64::MAX`. The fixture's own layout
    /// keeps the real offset, so only the manifest lies: that is what a
    /// corrupt server response looks like.
    pub huge_offset: Vec<String>,
    /// File rel paths whose declared `size` is `i64::MAX`.
    ///
    /// The plan's disk estimate sums the sizes of every planned file. Three
    /// files at `i64::MAX` do not fit in a `u64`, so the aggregate has to
    /// saturate rather than panic (debug) or wrap (release). The layout on
    /// disk is unchanged; only the claim is hostile.
    pub huge_file_size: Vec<String>,
    /// Chunk ids whose declared `compressed_size` is `i64::MAX`.
    ///
    /// Same class as [`Self::huge_file_size`], for the download estimate and
    /// the per-file document: three such chunks overflow the sum.
    pub huge_compressed_size: Vec<String>,
    /// File rel paths whose declared whole-file `md5` is a digest no payload
    /// can match.
    ///
    /// The bytes on disk are unchanged, so the size gate passes and the digest
    /// gate is what has to refuse. On a zero-length entry the honest digest is
    /// [`girpr::test_support::EMPTY_MD5`]; the lie is a non-empty digest, which a real
    /// server would never publish for a file with no bytes.
    pub wrong_file_md5: Vec<String>,
}

/// Another manifest in the latest build, beyond the primary `game` one.
#[derive(Clone, Debug)]
pub struct ExtraManifestSpec {
    /// The `matching_field` the build advertises.
    pub matching_field: String,
    /// The `category_id` the build advertises. Defaults to `matching_field`
    /// for the fixtures that model the two as one value; [`Self::with_category_id`]
    /// is how a fixture names the excluded category independently.
    pub category_id: String,
    /// The files this manifest describes.
    pub files: Vec<FileSpec>,
    /// Whether the mock serves the body. `false` models a build whose manifest
    /// is advertised but no longer exists at the origin (a `404`).
    pub served: bool,
}

impl ExtraManifestSpec {
    /// An extra latest-build manifest with the given field and files.
    pub fn new(matching_field: &str, files: Vec<FileSpec>) -> Self {
        Self {
            matching_field: matching_field.to_string(),
            category_id: matching_field.to_string(),
            files,
            served: true,
        }
    }

    /// The same manifest, advertised by `getBuild` but `404` when fetched.
    pub fn not_served(mut self) -> Self {
        self.served = false;
        self
    }

    /// The same manifest, advertised under a `category_id` other than its
    /// `matching_field`.
    ///
    /// The `res_category_dir` ignore set is matched against both fields, so
    /// this is how a fixture models a category the user excluded without
    /// also changing the field the manifest is selected by.
    pub fn with_category_id(mut self, category_id: &str) -> Self {
        self.category_id = category_id.to_string();
        self
    }
}

/// A whole fixture: the new side, the old side, and what is on disk.
#[derive(Clone, Debug, Default)]
pub struct FixtureSpec {
    /// The files the **latest** manifest describes — the repair's target.
    pub new: Vec<FileSpec>,
    /// Additional latest-build manifests, in build order after the primary one.
    pub extra_manifests: Vec<ExtraManifestSpec>,
    /// The files the **local** build's manifest describes — the reuse map's
    /// sources. `None` = the server does not serve the local tag at all,
    /// which is what "no chunk reuse" looks like.
    pub old: Option<Vec<FileSpec>>,
    /// The bytes on disk, by path. A path that is not listed does not exist.
    pub disk: BTreeMap<String, Vec<u8>>,
    /// The ways the manifest lies.
    pub lies: Lies,
}

impl FixtureSpec {
    /// A fixture with the given new-side files and nothing else: no local
    /// manifest, and nothing on disk.
    pub fn new(files: Vec<FileSpec>) -> Self {
        Self {
            new: files,
            ..Default::default()
        }
    }

    /// The same fixture, with a local build's manifest — the reuse map's
    /// sources.
    pub fn with_old(mut self, files: Vec<FileSpec>) -> Self {
        self.old = Some(files);
        self
    }

    /// The same fixture, with another manifest in the latest build.
    pub fn with_extra_manifest(mut self, extra: ExtraManifestSpec) -> Self {
        self.extra_manifests.push(extra);
        self
    }

    /// The same fixture, with a file on disk.
    pub fn with_disk(mut self, rel: &str, bytes: Vec<u8>) -> Self {
        self.disk.insert(rel.to_string(), bytes);
        self
    }

    /// The same fixture, with the manifest lying in the given ways.
    pub fn with_lies(mut self, lies: Lies) -> Self {
        self.lies = lies;
        self
    }
}

/// The bytes of a file laid out at its chunks' offsets.
///
/// A gap between two chunks is **zero bytes**, not nothing: the file on disk
/// is a byte range, and the hole is a part of it.
pub fn lay_out(chunks: &[ChunkSpec]) -> Vec<u8> {
    let end = chunks
        .iter()
        .map(|c| c.offset.max(0) as usize + c.payload.len())
        .max()
        .unwrap_or(0);
    let mut out = vec![0u8; end];
    for c in chunks {
        let at = c.offset.max(0) as usize;
        out[at..at + c.payload.len()].copy_from_slice(&c.payload);
    }
    out
}

/// Build one manifest's files and protobuf view, filling the chunk stores.
///
/// The primary manifest and extra latest-build manifests share this so a
/// multi-manifest fixture cannot describe its files differently from a
/// single-manifest one.
fn build_new_side(
    files_spec: &[FileSpec],
    lies: &Lies,
    chunk_store: &mut HashMap<String, Vec<u8>>,
    corrupt_chunk_store: &mut HashMap<String, Vec<u8>>,
) -> (Vec<FixtureFile>, Vec<SophonChunkFile>) {
    // The new side, as the mock's own view of it.
    let mut files: Vec<FixtureFile> = Vec::new();
    for f in files_spec {
        let data = lay_out(&f.chunks);
        files.push(FixtureFile {
            rel: f.rel.clone(),
            md5: md5hex(&data),
            data,
            chunks: f
                .chunks
                .iter()
                .map(|c| FixtureChunk {
                    id: c.id.clone(),
                    data: c.payload.clone(),
                    offset: c.offset,
                })
                .collect(),
        });
    }

    // The manifest, and the store it points at.
    let mut pb_files: Vec<SophonChunkFile> = Vec::new();
    for f in &files {
        let mut pb_chunks: Vec<SophonChunk> = Vec::new();
        for c in &f.chunks {
            let compressed = zstd_bytes(&c.data);
            // A chunk the store serves as something that is not a zstd frame
            // is served **raw**: the decode is what has to fail, and a
            // well-formed frame with wrong bytes would fail somewhere else.
            let served = if lies.not_zstd.contains(&c.id) {
                c.data.clone()
            } else {
                compressed.clone()
            };
            chunk_store.insert(c.id.clone(), served);
            // Same length, one byte different: decodes, fails its MD5.
            let mut bad = c.data.clone();
            if let Some(last) = bad.len().checked_sub(1) {
                bad[last] ^= 0xFF;
            }
            corrupt_chunk_store.insert(c.id.clone(), zstd_bytes(&bad));
            pb_chunks.push(SophonChunk {
                id: c.id.clone(),
                uncompressed_md5: if lies.wrong_chunk_md5.contains(&c.id) {
                    // A digest no payload can have: the chunk must be refused.
                    "00000000000000000000000000000000".to_string()
                } else {
                    md5hex(&c.data)
                },
                offset: if lies.huge_offset.contains(&c.id) {
                    // Past `i64::MAX` once the chunk's length is added: the
                    // layout of the file on disk is unchanged, only the claim.
                    i64::MAX
                } else {
                    c.offset
                },
                compressed_size: if lies.wrong_chunk_size.contains(&c.id) {
                    compressed.len() as i64 + 7
                } else if lies.huge_compressed_size.contains(&c.id) {
                    // The blob on the wire is still a normal frame; only the
                    // manifest's claim about its length is hostile.
                    i64::MAX
                } else {
                    compressed.len() as i64
                },
                uncompressed_size: c.data.len() as i64,
                unknown: 0,
                compressed_md5: md5hex(&compressed),
            });
        }
        pb_files.push(SophonChunkFile {
            file: f.rel.clone(),
            chunks: pb_chunks,
            is_folder: false,
            size: if lies.huge_file_size.contains(&f.rel) {
                // The bytes on disk are the fixture's real layout; only the
                // manifest's claim about the file's length is hostile.
                i64::MAX
            } else {
                f.data.len() as i64
            },
            md5: if lies.wrong_file_md5.contains(&f.rel) {
                // A digest no payload can have. The layout on disk is
                // unchanged; only the manifest's claim about the file is
                // hostile.
                "00000000000000000000000000000000".to_string()
            } else {
                f.md5.clone()
            },
        });
    }
    (files, pb_files)
}

/// Build the mock's fixture from a description.
///
/// The two sides are two lists of files and the bytes on disk are a third, so
/// every shape the repair has a branch for is a description rather than a
/// builder.
pub fn build_fixture_spec(spec: &FixtureSpec) -> GameFixture {
    let mut chunk_store: HashMap<String, Vec<u8>> = HashMap::new();
    let mut corrupt_chunk_store: HashMap<String, Vec<u8>> = HashMap::new();
    let (files, pb_files) = build_new_side(
        &spec.new,
        &spec.lies,
        &mut chunk_store,
        &mut corrupt_chunk_store,
    );
    let (manifest_zstd, manifest_checksum) = encode_manifest(&pb_files);

    // Extra manifests in the same latest build, in the order the fixture
    // described them. The mock's own build answer lists them after `game`, so
    // selection-order tests can rely on that order.
    let extra_latest_manifests: Vec<FixtureLatestManifest> = spec
        .extra_manifests
        .iter()
        .enumerate()
        .map(|(index, extra)| {
            // A manifest the build advertises but does not serve is a `404`
            // for its body, so no plan can ever name one of its chunks. Build
            // those chunks into a throwaway store: publishing bytes under ids
            // no reachable manifest names would make an "absent manifest"
            // fixture indistinguishable from a served one at the chunk route.
            let mut extra_chunks: HashMap<String, Vec<u8>> = HashMap::new();
            let mut extra_corrupt_chunks: HashMap<String, Vec<u8>> = HashMap::new();
            let (_, extra_pb) = build_new_side(
                &extra.files,
                &Lies::default(),
                &mut extra_chunks,
                &mut extra_corrupt_chunks,
            );
            if extra.served {
                chunk_store.extend(extra_chunks);
                corrupt_chunk_store.extend(extra_corrupt_chunks);
            }
            let (zstd, checksum) = encode_manifest(&extra_pb);
            FixtureLatestManifest {
                matching_field: extra.matching_field.clone(),
                category_id: extra.category_id.clone(),
                id: format!("manifest_test_extra_{index}_001"),
                zstd,
                checksum,
                served: extra.served,
            }
        })
        .collect();

    // The local build's manifest: the old side's chunk layout, and nothing
    // else. Its chunks are never fetched — the local manifest only ever feeds
    // the reuse map.
    let local_manifest = spec.old.as_ref().map(|old| {
        let mut old_pb: Vec<SophonChunkFile> = Vec::new();
        for f in old {
            let data = lay_out(&f.chunks);
            let mut pb_chunks: Vec<SophonChunk> = Vec::new();
            for c in &f.chunks {
                let compressed = zstd_bytes(&c.payload);
                pb_chunks.push(SophonChunk {
                    id: c.id.clone(),
                    uncompressed_md5: md5hex(&c.payload),
                    offset: c.offset,
                    compressed_size: compressed.len() as i64,
                    uncompressed_size: c.payload.len() as i64,
                    unknown: 0,
                    compressed_md5: md5hex(&compressed),
                });
            }
            old_pb.push(SophonChunkFile {
                file: f.rel.clone(),
                chunks: pb_chunks,
                is_folder: false,
                size: data.len() as i64,
                md5: md5hex(&data),
            });
        }
        let (zstd, checksum) = encode_manifest(&old_pb);
        FixtureManifest {
            id: "manifest_test_local_001".to_string(),
            zstd,
            checksum,
        }
    });

    // The old file's bytes: the first file on disk. Kept as its own field
    // because that is the shape the moved fixture has always had.
    let old_data = spec
        .disk
        .iter()
        .next()
        .map(|(rel, bytes)| (rel.clone(), bytes.clone()));

    GameFixture {
        latest_tag: LATEST_TAG.to_string(),
        manifest_id: "manifest_test_game_001".to_string(),
        manifest_zstd,
        manifest_checksum,
        files,
        chunk_store,
        corrupt_chunk_store,
        local_manifest,
        extra_latest_manifests,
        old_data,
        disk: spec.disk.clone(),
    }
}

pub fn build_fixture() -> GameFixture {
    build_fixture_with_lies(Lies::default())
}

/// The standard fixture, with the manifest lying in the given ways.
///
/// The bytes are the same; only the manifest's claims about them change, which
/// is what a test about a hint-versus-reality branch needs.
pub fn build_fixture_with_lies(lies: Lies) -> GameFixture {
    // Fixed payloads (distinct bytes per chunk so corruption is detectable).
    let foo_c0 = vec![0x41u8; 64]; // 'A' x64
    let foo_c1 = vec![0x42u8; 64]; // 'B' x64
    let bar_c0 = vec![0x43u8; 48]; // 'C' x48

    build_fixture_spec(
        &FixtureSpec::new(vec![
            FileSpec::new(
                "game/foo.dat",
                vec![
                    ChunkSpec::new("chunk_foo0", foo_c0, 0),
                    ChunkSpec::new("chunk_foo1", foo_c1, 64),
                ],
            ),
            FileSpec::new(
                "game/bar.dat",
                vec![ChunkSpec::new("chunk_bar0", bar_c0, 0)],
            ),
        ])
        .with_lies(lies),
    )
}

/// A fixture whose **old** file has a different chunk layout from the new
/// one — the content moved, so the repair has to move bytes *inside* the
/// file.
///
/// ```text
///   old:  [0,64)=P   [64,128)=Q   [128,192)=R
///   new:  [0,64)=R   [64,128)=P   [128,192)=T
/// ```
///
/// The new chunk 0 (`R`) has its source *later* in the file (offset 128) and
/// the new chunk 1 (`P`) has its source *earlier* (offset 0). The sources are
/// therefore in **both** directions, which is the only case where the two
/// write orders differ: the RAM tier keeps every source (it stages the bytes
/// first) and the ordered tier keeps one direction's worth.
///
/// The old file's manifest is served for a non-latest tag, so the run loads
/// it as the local build and the reuse map points at the old offsets — which
/// is what makes the sources "elsewhere in the file" at all.
pub fn build_moved_fixture() -> GameFixture {
    let p = vec![0x50u8; 64]; // 'P'
    let q = vec![0x51u8; 64]; // 'Q'
    let r = vec![0x52u8; 64]; // 'R'
    let t = vec![0x54u8; 64]; // 'T'

    // The old file: P || Q || R, with its own chunk ids — they are never
    // fetched, because the local manifest only ever feeds the reuse map.
    let old_data = lay_out(&[
        ChunkSpec::new("local_p", p.clone(), 0),
        ChunkSpec::new("local_q", q.clone(), 64),
        ChunkSpec::new("local_r", r.clone(), 128),
    ]);

    build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new(
            "game/moved.dat",
            vec![
                ChunkSpec::new("chunk_r", r.clone(), 0),
                ChunkSpec::new("chunk_p", p.clone(), 64),
                ChunkSpec::new("chunk_t", t.clone(), 128),
            ],
        )])
        .with_old(vec![FileSpec::new(
            "game/moved.dat",
            vec![
                ChunkSpec::new("local_p", p, 0),
                ChunkSpec::new("local_q", q, 64),
                ChunkSpec::new("local_r", r, 128),
            ],
        )])
        .with_disk("game/moved.dat", old_data),
    )
}

/// Encode a manifest body the way the server sends it: zstd over the
/// protobuf, with the checksum being the MD5 of the *decompressed* bytes.
fn encode_manifest(files: &[SophonChunkFile]) -> (Vec<u8>, String) {
    let manifest = SophonChunkManifest {
        chuncks: files.to_vec(),
    };
    let mut raw = Vec::new();
    manifest.encode(&mut raw).expect("prost encode fixture");
    let checksum = md5hex(&raw);
    (zstd_bytes(&raw), checksum)
}

// ---------------------------------------------------------------------------
// temp dirs
// ---------------------------------------------------------------------------

/// Fresh unique temp dir for one test, removed when the returned guard drops.
///
/// The name carries the pid, a timestamp and a per-process counter so two runs
/// — or two `cargo nextest run` invocations — never share a tree, and the guard
/// means the tree is removed even when a case fails before its last line. See
/// `girpr_tempdir` for why each part is there.
pub fn temp_dir(name: &str) -> girpr_tempdir::TempDir {
    girpr_tempdir::TempDir::new("girpr_it", name)
}

pub fn write_game_file(game_dir: &std::path::Path, rel: &str, bytes: &[u8]) {
    let rel_fs = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    let p = game_dir.join(rel_fs);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(&p, bytes).expect("write game file");
}

pub fn read_game_file(game_dir: &std::path::Path, rel: &str) -> Option<Vec<u8>> {
    let rel_fs = rel.replace('/', std::path::MAIN_SEPARATOR_STR);
    std::fs::read(game_dir.join(rel_fs)).ok()
}

/// Drive `f` under a debug-level scoped subscriber and return its result with
/// the log lines the run emitted.
///
/// `set_default` installs the subscriber on the current thread only, and
/// `#[tokio::test]` runs `f` on a current-thread runtime, so the future is
/// polled on the thread that installed it and no other test can observe the
/// subscriber. The blocking tasks whose lines a test asserts use
/// `spawn_blocking_traced`, which installs the same dispatcher on the pool
/// thread, so their lines (including the post-phase's listing) are captured
/// too. The async form is duplicated in the metadata tests: sharing it would
/// require exporting a public test helper. The stdout data plane
/// (`REPORT`/`SUMMARY`) is separate and is asserted through the returned
/// `Summary`, not through these lines.
pub async fn logs_of<F: std::future::Future>(f: F) -> (F::Output, String) {
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("captured log lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let buf = Captured::default();
    let writer = buf.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let out = f.await;
    drop(guard);
    let text = String::from_utf8(buf.0.lock().expect("captured log lock").clone())
        .expect("tracing writes UTF-8");
    (out, text)
}

// ---------------------------------------------------------------------------
// mock server
// ---------------------------------------------------------------------------

/// How `getGameBranches` fails.
///
/// The retry classifier reads the body, not the HTTP status: a JSON wrapper
/// with a retryable `retcode` is retried, a terminal one ends the call, and a
/// body that is not JSON fails to parse on the first attempt. These variants
/// are the three bodies a test can ask the mock to serve.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BranchFailure {
    /// Answer normally.
    #[default]
    None,
    /// Answer every request with a non-JSON `500` body. The wrapper parse
    /// fails, which the client classifies as fatal: one attempt, no retry.
    NonJson,
    /// Answer the first `calls` requests with a JSON wrapper carrying a
    /// retryable `retcode`, then the normal branch answer. Use a value at or
    /// above the client's attempt count (five) to exercise exhaustion.
    Retryable { calls: u32 },
    /// Answer every request with a JSON wrapper carrying this terminal
    /// `retcode`. The client ends the call after the first answer.
    Terminal { retcode: i64 },
}

impl BranchFailure {
    /// A terminal failure carrying a `retcode` the client classifies as
    /// terminal.
    ///
    /// Panics on any other code: such a fixture would be a retryable failure
    /// wearing the terminal name.
    pub fn terminal(retcode: i64) -> Self {
        assert!(
            terminal_retcode(retcode),
            "retcode {retcode} is not terminal; expected -201, -202 or -1001"
        );
        Self::Terminal { retcode }
    }
}

#[derive(Default)]
pub struct MockOpts {
    /// The launcher this fixture describes. `None` serves `hk4e_global`.
    ///
    /// The response's `game` object carries this biz and game id, so a run for
    /// another launcher is not silently answered with the global identity.
    pub biz: Option<Biz>,
    pub deprecated_files: Vec<String>,
    pub scan_dir: String,
    pub res_dir: String,
    pub cache_dir: String,
    /// `getGameConfigs.res_category_dir` (a path under `game_dir`).
    pub res_category_dir: String,
    /// `getGameConfigs.blacklist_dir` (a path under `game_dir`).
    pub blacklist_dir: String,
    /// `getGameConfigs.enable_resource_blacklist`.
    pub enable_resource_blacklist: bool,
    /// `getGameConfigs.default_download_mode`. `None` serves the live
    /// `DOWNLOAD_MODE_CHUNK`; `Some(s)` serves `s` verbatim, so a test can send
    /// `Some("")` to model the API omitting the field.
    pub download_mode: Option<String>,
    /// Serve every chunk request as `500`. Models a reachable-but-broken origin,
    /// which is what exercises the retry/backoff paths (`--fail-fast`) without
    /// a dead host and its connection timeouts.
    pub fail_chunks: bool,
    /// How `getGameBranches` answers, while `getGameConfigs` still answers.
    /// Models the branch endpoint failing after the first wave has already
    /// been issued, so error precedence and the client's retry classification
    /// are testable.
    pub branch_failure: BranchFailure,
    /// Hold the `getGameConfigs` answer until `getGameBranches` has been served.
    ///
    /// The first wave issues both calls concurrently, so without this the
    /// request that is merely *issued* is not deterministic. Holding the config
    /// answer pins the branch as the first of the two to settle, which lets a
    /// test assert on a branch error that is already in hand rather than on
    /// polling order.
    pub config_after_branches: bool,
    /// Serve every chunk request as `404`. Models the blob routes' real answer
    /// to an id they do not have: the live CDN (`autopatchhk.yuanshen.com`,
    /// `Server: AliyunOSS`) answers `404` with an `application/xml` `NoSuchKey`
    /// body and never a `200`-with-a-code, and an id under the wrong build's
    /// prefix is a `404` too. The distinction from `fail_chunks` is the whole
    /// point: a 5xx is worth retrying and a 404 is not, so only one of the two
    /// can be told apart by how many times it was asked for.
    pub not_found_chunks: bool,
    /// Serve every manifest request as `404`, for the same reason. The id is
    /// published atomically with the build that serves it, so this models a
    /// build whose manifests are gone rather than a not-yet-propagated one.
    pub not_found_manifests: bool,
    /// Serve the first N chunk responses with valid-but-**wrong** bytes: the
    /// same uncompressed length and a well-formed body, so the chunk decodes
    /// fine and fails on its MD5. Models a corrupt or half-propagated origin,
    /// which is what makes a file retry — and therefore what distinguishes
    /// "bytes of promoted files" from "bytes that crossed the wire".
    pub corrupt_first_chunks: u32,
    /// Serve these chunk ids corrupt on their **first** request.
    ///
    /// `corrupt_first_chunks` is a counter over the requests, so it can only
    /// ever break the *first* chunk a file asks for. Naming the ids is what
    /// breaks a later one — the case where an attempt has already written a
    /// chunk that passed its gate before it fails, and the next attempt has to
    /// find that chunk still correct.
    pub corrupt_chunks: Vec<String>,
    /// Advertise a manifest checksum the blob does not have. The blob itself
    /// is served unchanged, so the only thing that can catch it is the
    /// checksum gate — the MD5 of the **decompressed** bytes.
    pub wrong_manifest_checksum: bool,
    /// Serve the first N chunk responses as a body that is **cut short**: the
    /// `Content-Length` promises the whole blob and only `truncate_bytes` of it
    /// are written, then the connection closes. Models a transport that dies
    /// mid-body — the case where bytes crossed the wire but no complete chunk
    /// arrived, so the download ceiling has to charge the partial body.
    pub truncate_first_chunks: u32,
    /// How many bytes of a truncated body to write before closing.
    pub truncate_bytes: usize,
    /// Called in the server task just before the **first** chunk body is sent.
    ///
    /// The request necessarily arrives after the plan phase has finished, so a
    /// test can use this to change the install between the plan scan and the
    /// repair loop without racing a timer.
    pub on_first_chunk: Option<Arc<dyn Fn() + Send + Sync>>,
}

pub struct MockServer {
    pub hyp_base: String,
    pub sophon_base: String,
    /// chunk ids fetched since start (proves reuse / no-download cases)
    pub chunk_requests: Arc<Mutex<Vec<String>>>,
    /// every request target seen, in order (proves query params, e.g. the
    /// per-launcher `channel`/`sub_channel` on `getGameDeprecatedFileConfigs`)
    pub targets: Arc<Mutex<Vec<String>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl MockServer {
    pub fn stop(&self) {
        self.handle.abort();
    }

    pub async fn start(fixture: GameFixture, opts: MockOpts) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let port = listener.local_addr().expect("local addr").port();
        let hyp_base = format!("http://127.0.0.1:{port}/hyp/hyp-connect/api");
        let sophon_base = format!("http://127.0.0.1:{port}/downloader/sophon_chunk/api");
        let manifest_prefix = format!("http://127.0.0.1:{port}/manifests/test_build");
        let chunk_prefix = format!("http://127.0.0.1:{port}/chunks/test_build");

        let manifest_len = fixture.manifest_zstd.len();
        let responses = Arc::new(build_responses(
            &fixture,
            &opts,
            &manifest_prefix,
            &chunk_prefix,
        ));
        let fixture = Arc::new(fixture);
        let chunk_requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let targets: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        let log = chunk_requests.clone();
        let seen = targets.clone();
        let handle = tokio::spawn(async move {
            loop {
                let (socket, _) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let responses = responses.clone();
                let fixture = fixture.clone();
                let log = log.clone();
                let seen = seen.clone();
                tokio::spawn(async move {
                    serve_one(socket, &responses, &fixture, &log, &seen).await;
                });
            }
        });

        // Hold lengths for the unused warning (manifest bytes live in fixture).
        let _ = manifest_len;
        Self {
            hyp_base,
            sophon_base,
            chunk_requests,
            targets,
            handle,
        }
    }

    pub fn chunk_hits(&self) -> Vec<String> {
        self.chunk_requests.lock().unwrap().clone()
    }

    /// Every request target the server has seen, in order.
    pub fn request_targets(&self) -> Vec<String> {
        self.targets.lock().unwrap().clone()
    }

    /// The query string of the first request whose target contains `needle`.
    pub fn first_target_with(&self, needle: &str) -> Option<String> {
        self.request_targets()
            .into_iter()
            .find(|t| t.contains(needle))
    }
}

struct Responses {
    configs: Vec<u8>,
    branches: Vec<u8>,
    deprecated: Vec<u8>,
    build_latest: Vec<u8>,
    build_not_found: Vec<u8>,
    /// The `getBuild` answer for a **local** tag, when the server serves one.
    /// The template's `data.tag` is replaced with the tag the request asked
    /// for, so the served build always matches the local version. `None` = the
    /// local tag is not served, which is the default.
    build_local: Option<serde_json::Value>,
    /// The local manifest's body, by id.
    local_manifest: Option<(String, Vec<u8>)>,
    /// Extra latest-build manifest bodies, by id. A manifest advertised by
    /// `getBuild` but absent here is a `404`.
    extra_manifests: Vec<(String, Vec<u8>)>,
    latest_tag: String,
    fail_chunks: bool,
    branch_failure: BranchFailure,
    /// Branch requests served so far, so `BranchFailure::Retryable` can answer
    /// the first `calls` of them with a retryable retcode.
    branch_calls: AtomicU64,
    /// Hold the config answer until the branch answer has been served.
    config_after_branches: bool,
    /// Signalled once the branch answer has been served.
    ///
    /// A watch (not a one-shot permit) so a *retried* config call still sees
    /// that the branch was already served instead of waiting forever for a
    /// permit that the first call consumed.
    branches_served: tokio::sync::watch::Sender<bool>,
    not_found_chunks: bool,
    not_found_manifests: bool,
    corrupt_first_chunks: u32,
    /// Chunk ids to serve corrupt on their **first** request, whatever the
    /// request order.
    ///
    /// `corrupt_first_chunks` is a counter over the requests, so it can only
    /// ever break the *first* chunk a file asks for. This one names the ids,
    /// which is what a test needs to break a *later* chunk — the case where an
    /// attempt has already written a chunk that passed its gate before it
    /// fails.
    corrupt_chunks: Vec<String>,
    /// Chunk responses to serve cut short, counted over the requests like
    /// `corrupt_first_chunks`: the first N responses promise a full body and
    /// deliver `truncate_bytes` of it.
    truncate_first_chunks: u32,
    truncate_bytes: usize,
    /// Runs once, before the first chunk body is sent.
    on_first_chunk: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Whether `on_first_chunk` has already run.
    first_chunk_seen: AtomicBool,
    /// The ids whose first response was already served corrupt. An id is in
    /// here from its first corrupt response onwards, so the second request for
    /// the same id gets the good bytes: a flaky origin, not a broken one.
    corrupt_served: Mutex<HashSet<String>>,
    /// Chunk responses served so far, so the corrupt-N counter is per-server
    /// and not per-connection.
    served: AtomicU64,
}

fn build_responses(
    fx: &GameFixture,
    opts: &MockOpts,
    manifest_prefix: &str,
    chunk_prefix: &str,
) -> Responses {
    // The variant stays constructible directly, so check the knob here as well
    // as in `BranchFailure::terminal`: a debug build must not silently serve a
    // "terminal" failure the client would retry. Release keeps the fixture
    // usable because this is only a self-check.
    if let BranchFailure::Terminal { retcode } = opts.branch_failure {
        debug_assert!(
            terminal_retcode(retcode),
            "retcode {retcode} is not terminal; expected -201, -202 or -1001"
        );
    }
    let biz = opts.biz.unwrap_or(Biz::Hk4eGlobal);
    let (_, _, game_id) = biz.endpoints();
    let game_biz = biz.as_str();
    let raw_manifest_len = zstd::stream::decode_all(std::io::Cursor::new(fx.manifest_zstd.clone()))
        .expect("decode fixture manifest")
        .len();
    let configs = serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "launch_configs": [{
                "game": {"id": game_id, "biz": game_biz},
                "exe_file_name": "GenshinImpact.exe",
                "audio_pkg_scan_dir": opts.scan_dir,
                "audio_pkg_res_dir": opts.res_dir,
                "audio_pkg_cache_dir": opts.cache_dir,
                "default_download_mode": opts
                    .download_mode
                    .clone()
                    .unwrap_or_else(|| "DOWNLOAD_MODE_CHUNK".to_string()),
                "res_category_dir": opts.res_category_dir,
                "blacklist_dir": opts.blacklist_dir,
                "enable_resource_blacklist": opts.enable_resource_blacklist
            }]
        }
    });
    let branches = serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "game_branches": [{
                "game": {"id": game_id, "biz": game_biz},
                "main": {
                    "package_id": "pkg_test",
                    "branch": "main",
                    "password": "",
                    "tag": fx.latest_tag,
                    "diff_tags": []
                },
                "pre_download": null
            }]
        }
    });
    let deprecated_files: Vec<serde_json::Value> = opts
        .deprecated_files
        .iter()
        .map(|n| serde_json::json!({"name": n}))
        .collect();
    let deprecated = serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "deprecated_file_configs": [{
                "game": {"id": game_id, "biz": game_biz},
                "deprecated_files": deprecated_files
            }]
        }
    });
    let mut latest_manifests = vec![serde_json::json!({
        "category_id": "10016",
        "category_name": "game-test",
        "matching_field": "game",
        "manifest": {
            "id": fx.manifest_id,
            "checksum": if opts.wrong_manifest_checksum {
                // A checksum no blob can have: the manifest must be
                // refused, and the run must fail as metadata.
                "00000000000000000000000000000000".to_string()
            } else {
                fx.manifest_checksum.clone()
            },
            "compressed_size": fx.manifest_zstd.len() as i64,
            "uncompressed_size": raw_manifest_len as i64
        },
        "chunk_download": {"url_prefix": chunk_prefix, "url_suffix": ""},
        "manifest_download": {"url_prefix": manifest_prefix, "url_suffix": ""}
    })];
    for m in &fx.extra_latest_manifests {
        let raw_len = zstd::stream::decode_all(std::io::Cursor::new(m.zstd.clone()))
            .expect("decode extra fixture manifest")
            .len();
        latest_manifests.push(serde_json::json!({
            "category_id": m.category_id,
            "category_name": m.matching_field,
            "matching_field": m.matching_field,
            "manifest": {
                "id": m.id,
                "checksum": m.checksum,
                "compressed_size": m.zstd.len() as i64,
                "uncompressed_size": raw_len as i64
            },
            "chunk_download": {"url_prefix": chunk_prefix, "url_suffix": ""},
            "manifest_download": {"url_prefix": manifest_prefix, "url_suffix": ""}
        }));
    }
    let build_latest = serde_json::json!({
        "retcode": 0,
        "message": "OK",
        "data": {
            "build_id": "build_test_001",
            "tag": fx.latest_tag,
            "manifests": latest_manifests
        }
    });
    // The local build, when the server serves one: a different manifest, and
    // so a different chunk layout for the same path. That is what makes the
    // repair move bytes inside the file.
    let build_local = fx.local_manifest.as_ref().map(|m| {
        serde_json::json!({
            "retcode": 0,
            "message": "OK",
            "data": {
                "build_id": "build_test_old",
                "tag": LOCAL_TAG,
                "manifests": [{
                    "category_id": "10016",
                    "category_name": "game-test",
                    "matching_field": "game",
                    "manifest": {
                        "id": m.id,
                        "checksum": m.checksum,
                        "compressed_size": m.zstd.len() as i64,
                        "uncompressed_size": 0
                    },
                    "chunk_download": {"url_prefix": chunk_prefix, "url_suffix": ""},
                    "manifest_download": {"url_prefix": manifest_prefix, "url_suffix": ""}
                }]
            }
        })
    });
    let local_manifest = fx
        .local_manifest
        .as_ref()
        .map(|m| (m.id.clone(), m.zstd.clone()));
    let extra_manifests = fx
        .extra_latest_manifests
        .iter()
        .filter(|m| m.served)
        .map(|m| (m.id.clone(), m.zstd.clone()))
        .collect();
    Responses {
        configs: serde_json::to_vec(&configs).unwrap(),
        branches: serde_json::to_vec(&branches).unwrap(),
        deprecated: serde_json::to_vec(&deprecated).unwrap(),
        build_latest: serde_json::to_vec(&build_latest).unwrap(),
        build_not_found: br#"{"retcode":-202,"message":"not found","data":null}"#.to_vec(),
        build_local,
        local_manifest,
        extra_manifests,
        latest_tag: fx.latest_tag.clone(),
        fail_chunks: opts.fail_chunks,
        branch_failure: opts.branch_failure,
        branch_calls: AtomicU64::new(0),
        config_after_branches: opts.config_after_branches,
        branches_served: tokio::sync::watch::channel(false).0,
        not_found_chunks: opts.not_found_chunks,
        not_found_manifests: opts.not_found_manifests,
        corrupt_first_chunks: opts.corrupt_first_chunks,
        corrupt_chunks: opts.corrupt_chunks.clone(),
        truncate_first_chunks: opts.truncate_first_chunks,
        truncate_bytes: opts.truncate_bytes,
        on_first_chunk: opts.on_first_chunk.clone(),
        first_chunk_seen: AtomicBool::new(false),
        corrupt_served: Mutex::new(HashSet::new()),
        served: AtomicU64::new(0),
    }
}

fn query_tag(target: &str) -> Option<String> {
    let q = target.split_once('?')?.1;
    for pair in q.split('&') {
        if let Some(v) = pair.strip_prefix("tag=") {
            return Some(v.to_string());
        }
    }
    None
}

async fn serve_one(
    socket: tokio::net::TcpStream,
    r: &Responses,
    fx: &GameFixture,
    log: &Arc<Mutex<Vec<String>>>,
    seen: &Arc<Mutex<Vec<String>>>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut sock = socket;
    let mut buf = vec![0u8; 65536];
    let mut got = 0usize;
    // Read until end of headers (GET has no body).
    loop {
        match sock.read(&mut buf[got..]).await {
            Ok(0) => break,
            Ok(n) => {
                got += n;
                if buf[..got].windows(4).any(|w| w == b"\r\n\r\n") || got >= buf.len() {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&buf[..got]).to_string();
    let target = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    seen.lock().unwrap().push(target.clone());

    // A body that is cut short is written in full-length header terms and then
    // abandoned, which is what a transport that dies mid-body looks like.
    let mut cut: Option<usize> = None;
    // Set when this response is the first successful chunk body.
    let mut first_chunk_body = false;
    let is_branch = target.contains("getGameBranches");
    let (status, ctype, body): (&str, &str, Vec<u8>) = if target.contains("getGameConfigs") {
        if r.config_after_branches {
            // `wait_for` returns immediately when the retained value is
            // already true, so a retried config call cannot hang.
            let mut served = r.branches_served.subscribe();
            let _ = served.wait_for(|served| *served).await;
        }
        ("200 OK", "application/json", r.configs.clone())
    } else if is_branch {
        let n = r.branch_calls.fetch_add(1, Ordering::Relaxed);
        match r.branch_failure {
            BranchFailure::NonJson => (
                "500 Internal Server Error",
                "text/plain",
                b"branch endpoint unavailable".to_vec(),
            ),
            BranchFailure::Retryable { calls } if n < u64::from(calls) => (
                "200 OK",
                "application/json",
                br#"{"retcode":-1,"message":"busy","data":null}"#.to_vec(),
            ),
            BranchFailure::Terminal { retcode } => (
                "200 OK",
                "application/json",
                serde_json::to_vec(&serde_json::json!({
                    "retcode": retcode,
                    "message": "invalid params",
                    "data": null
                }))
                .expect("serialize branch failure"),
            ),
            _ => ("200 OK", "application/json", r.branches.clone()),
        }
    } else if target.contains("getGameDeprecatedFileConfigs") {
        ("200 OK", "application/json", r.deprecated.clone())
    } else if target.contains("getBuild") {
        match query_tag(&target) {
            Some(t) if t != r.latest_tag => match &r.build_local {
                // The local build, when the server serves one. It is tagged
                // with the version the request asked for, so a test that
                // changes its local version gets a matching build instead of
                // silently falling back to a full fetch.
                Some(b) => {
                    let mut body = b.clone();
                    body["data"]["tag"] = serde_json::Value::String(t);
                    (
                        "200 OK",
                        "application/json",
                        serde_json::to_vec(&body).expect("serialize local build"),
                    )
                }
                None => ("200 OK", "application/json", r.build_not_found.clone()),
            },
            _ => ("200 OK", "application/json", r.build_latest.clone()),
        }
    } else if target.contains("/manifests/") {
        let id = target
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("");
        if r.not_found_manifests {
            ("404 Not Found", "text/plain", b"no such manifest".to_vec())
        } else if id == fx.manifest_id {
            (
                "200 OK",
                "application/octet-stream",
                fx.manifest_zstd.clone(),
            )
        } else if let Some((local_id, body)) = &r.local_manifest
            && id == local_id
        {
            ("200 OK", "application/octet-stream", body.clone())
        } else if let Some((_, body)) = r.extra_manifests.iter().find(|(mid, _)| mid == id) {
            ("200 OK", "application/octet-stream", body.clone())
        } else {
            ("404 Not Found", "text/plain", b"no such manifest".to_vec())
        }
    } else if target.contains("/chunks/") {
        let id = target
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("");
        if r.not_found_chunks || r.fail_chunks {
            // Still logged, on both branches: "was this chunk even asked for,
            // and how many times" is the assertion that tells a 404 (asked once)
            // from a 5xx (asked five), so a 404 has to reach this log too.
            log.lock().unwrap().push(id.to_string());
            if r.not_found_chunks {
                // The shape the live CDN uses, so a test cannot accidentally
                // depend on a content-type the origin never sends.
                (
                    "404 Not Found",
                    "application/xml",
                    b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".to_vec(),
                )
            } else {
                (
                    "500 Internal Server Error",
                    "text/plain",
                    b"chunk store is down".to_vec(),
                )
            }
        } else {
            match fx.chunk_store.get(id) {
                Some(bytes) => {
                    first_chunk_body = true;
                    log.lock().unwrap().push(id.to_string());
                    let n = r.served.fetch_add(1, Ordering::Relaxed);
                    // The named ids are corrupt on their **first** request
                    // only: the second one gets the good bytes, which is what
                    // a flaky origin looks like.
                    let first_time = {
                        let mut served = r.corrupt_served.lock().unwrap();
                        r.corrupt_chunks.iter().any(|c| c == id) && served.insert(id.to_string())
                    };
                    if n < r.corrupt_first_chunks as u64 || first_time {
                        // Well-formed, same uncompressed length, wrong MD5.
                        let bad = fx
                            .corrupt_chunk_store
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| bytes.clone());
                        ("200 OK", "application/octet-stream", bad)
                    } else {
                        if n < r.truncate_first_chunks as u64 {
                            cut = Some(r.truncate_bytes.min(bytes.len()));
                        }
                        ("200 OK", "application/octet-stream", bytes.clone())
                    }
                }
                None => {
                    // An id the store has never heard of is the same event as
                    // `not_found_chunks`, and it is logged for the same reason.
                    log.lock().unwrap().push(id.to_string());
                    (
                        "404 Not Found",
                        "application/xml",
                        b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".to_vec(),
                    )
                }
            }
        }
    } else {
        ("404 Not Found", "text/plain", b"unknown route".to_vec())
    };

    // The branch answer is chosen; let a held config answer proceed. The
    // request itself was already logged above, so a test can assert it was
    // issued regardless of when the client reads either response.
    if is_branch {
        // `send_replace` updates even when no config request is waiting yet:
        // a one-shot `send` would fail with no receiver, leaving a later
        // config call waiting forever.
        r.branches_served.send_replace(true);
    }

    // The plan phase is over by the time a chunk is requested, so this is the
    // deterministic seam for "the install changed between planning and
    // repair". It runs before the body, so the change is visible when the
    // repair loop reaches the next file.
    if first_chunk_body
        && let Some(hook) = &r.on_first_chunk
        && !r.first_chunk_seen.swap(true, Ordering::Relaxed)
    {
        hook();
    }

    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = sock.write_all(header.as_bytes()).await;
    let _ = sock.write_all(&body[..cut.unwrap_or(body.len())]).await;
    let _ = sock.flush().await;
}
