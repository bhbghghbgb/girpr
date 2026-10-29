//! A local mock of the whole API surface, built by the test.
//!
//! No dependency, no fixture file, and nothing borrowed from the
//! repair binary's own mock: the synthetic manifests, the chunk bodies, the
//! `getBuild` bodies and the four HoYoPlay bodies are all *constructed here*,
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
//! | `/hyp`   | the four HoYoPlay endpoints                        |
//! | `/sophon`| `getBuild`                                          |
//! | `/cdn`   | manifest archives and chunk blobs                  |
//!
//! All three live on `127.0.0.1` with an ephemeral port, so **no test can reach
//! the network even by accident** — there is nothing else to reach. That is the
//! property the suite asserts, so it is enforced by construction rather than by
//! remembering to pass `--hyp-base`.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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

/// A chunk whose bytes are a function of `seed`, and nothing else.
///
/// Deterministic so a test's expectations are stable, and distinct per `seed` so
/// two files cannot collide by accident.
pub fn blob(seed: u64, len: usize) -> Blob {
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
    let uncompressed_md5 = md5_hex(&body);
    // The 16-hex prefix is deliberately *not* a function of anything a reader
    // could check, because upstream's is not either (docs/00-spec.md §3.7).
    let prefix = md5_hex(format!("prefix/{seed}").as_bytes())[..16].to_string();
    Blob {
        id: format!("{prefix}_{uncompressed_md5}"),
        compressed_md5: md5_hex(&raw),
        compressed_size: raw.len() as i64,
        uncompressed_md5,
        uncompressed_size: body.len() as i64,
        declared_uncompressed_size: None,
        served: None,
        raw,
        body,
    }
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
        md5_hex(&self.bytes())
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

/// The whole synthetic world: the builds, and what the four HoYoPlay endpoints
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
                        // to A21-A23.
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
    /// is the case that used to give both the id `added`: the second patchmap
    /// replaced the first while the run still reported two captures. Nothing else
    /// in the world changes, so every other class keeps its ordinary candidate.
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
        let checksum = md5_hex(&plain);
        let prefix =
            md5_hex(format!("manifest/{}/{}", build.tag, field).as_bytes())[..16].to_string();
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
            table,
        }
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
        // One connect-and-close is enough to unblock `incoming()`.
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
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

fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == key
        {
            return Some(percent_decode(v));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'%'));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

fn md5_hex(bytes: &[u8]) -> String {
    let mut c = md5::Context::new();
    c.consume(bytes);
    format!("{:x}", c.compute())
}
