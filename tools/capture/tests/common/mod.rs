//! A local mock of the whole API surface, built by the test.
//!
//! **M10.** No dependency, no fixture file, and nothing borrowed from the
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
//! property M10 is asserting, so it is enforced by construction rather than by
//! remembering to pass `--hyp-base`.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------- the world

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
        raw,
        body,
    }
}

/// One file's entry: its path and the chunks, in order.
#[derive(Debug, Clone)]
pub struct File {
    /// The manifest-relative path.
    pub path: String,
    /// Its chunks, left to right.
    pub chunks: Vec<Blob>,
}

impl File {
    /// A file of `n` chunks derived from `seeds`, each `len` bytes.
    pub fn new(path: &str, seeds: &[u64], len: usize) -> File {
        File {
            path: path.to_string(),
            chunks: seeds.iter().map(|s| blob(*s, len)).collect(),
        }
    }

    /// The bytes a reader would get back.
    pub fn bytes(&self) -> Vec<u8> {
        self.chunks.iter().flat_map(|c| c.body.clone()).collect()
    }

    /// The declared size, which is `Σ uncompressed_size`.
    pub fn size(&self) -> i64 {
        self.chunks.iter().map(|c| c.uncompressed_size).sum()
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
                let chunk = gircap::manifest::proto::SophonChunk {
                    id: c.id.clone(),
                    uncompressed_md5: c.uncompressed_md5.clone(),
                    offset: cursor,
                    compressed_size: c.compressed_size,
                    uncompressed_size: c.uncompressed_size,
                    unknown_field6: 0,
                    compressed_md5: c.compressed_md5.clone(),
                };
                cursor += c.uncompressed_size;
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
}

impl World {
    /// A world whose `game` field exercises five of the nine classes.
    ///
    /// | file                 | old → new                            | class                |
    /// |----------------------|--------------------------------------|----------------------|
    /// | `browser_assets`     | same chunks, same order              | `identical`          |
    /// | `libNew.dll`         | absent → one new chunk              | `added`              |
    /// | `old.blk`            | one old chunk → absent              | `removed`            |
    /// | `zero.blk`           | all chunks replaced                 | `zero_reuse_multi`   |
    /// | `HoYoChannel.dll`    | first chunk replaced, rest shared   | `partial_same_offset`|
    ///
    /// The remaining four are `unavailable`, which is the honest answer for a
    /// synthetic world and matches the live 7.0.0 → 7.1.0 finding that
    /// `full_reuse_reordered` has no candidate at all.
    ///
    /// **`en-us` is published too**, because `--audio` defaults to `en-us` and
    /// the capture then fetches that manifest from *both* builds. A world that
    /// published only `game` would not be a narrower fixture — it would be a
    /// capture that cannot finish. The audio file is byte-identical across the
    /// pair so it does not take a class away from `game`.
    pub fn five_classes() -> World {
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
                        File::new("GenshinImpact_Data/StreamingAssets/blocks/old.blk", &[91], 1024),
                        File::new("GenshinImpact_Data/StreamingAssets/blocks/zero.blk", &[70, 71], 512),
                        File::new("GenshinImpact_Data/Plugins/HoYoChannel.dll", &[10, 11, 12], 1024),
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
                        File::new("GenshinImpact_Data/StreamingAssets/blocks/zero.blk", &[80, 81], 512),
                        File::new("GenshinImpact_Data/Plugins/HoYoChannel.dll", &[99, 11, 12], 1024),
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
        }
    }

    /// [`World::five_classes`] with no audio field at all.
    ///
    /// Only usable with `--audio none`: with the default, the capture asks for an
    /// `en-us` manifest this world does not publish, and refuses. That refusal is
    /// correct and is not what this world is for.
    pub fn game_only() -> World {
        let mut w = World::five_classes();
        w.old.fields.retain(|(f, _)| f == "game");
        w.new.fields.retain(|(f, _)| f == "game");
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
        let prefix = md5_hex(format!("manifest/{}/{}", build.tag, field).as_bytes())[..16].to_string();
        Ok(Archive {
            id: format!("manifest_{prefix}_{checksum}"),
            checksum,
            raw,
            plain_len: plain.len() as i64,
            file_count: files.len() as i64,
            chunk_count: files.iter().map(|f| f.chunks.len() as i64).sum(),
            uncompressed_size: files.iter().map(|f| f.size()).sum(),
            compressed_size: files.iter().map(|f| f.chunks.iter().map(|c| c.compressed_size).sum::<i64>()).sum(),
        })
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
    pub uncompressed_size: i64,
    /// `Σ chunk compressed_size`.
    pub compressed_size: i64,
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
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let table = Arc::clone(&table);
                let log = Arc::clone(&thread_log);
                std::thread::spawn(move || {
                    let _ = serve(stream, &table, &log);
                });
            }
        });

        Mock { base, stop, log }
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
        self.log.lock().expect("the request log is not poisoned").clone()
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
    chunks: BTreeMap<String, Blob>,
}

/// Every `matching_field` any build publishes, in a stable order.
///
/// **Derived from the builds, never declared separately.** The first version
/// hardcoded `categories` to `[game, en-us]` while `five_classes()` published
/// only `game`, and the mock lied: the branch claimed to publish a manifest the
/// build did not carry, and the capture failed at the join. A mock that can
/// disagree with itself is a mock that will one day hide a real bug behind a
/// fabricated one.
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
///
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
                            J::s("GenshinImpact_Data/Persistent"),
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
    let mut chunks: BTreeMap<String, Blob> = BTreeMap::new();
    for build in [&world.new, &world.old] {
        let mut manifests = Vec::new();
        for (field, files) in &build.fields {
            let archive = world.archive(build, field).expect("the world is self-consistent");
            for f in files {
                for c in &f.chunks {
                    chunks.entry(c.id.clone()).or_insert_with(|| c.clone());
                }
            }
            archives.insert((build.build_id.clone(), archive.id.clone()), archive.clone());
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
                        ("compressed_size".into(), J::s(archive.compressed_size.to_string())),
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

    Table { hyp, builds, archives, chunks }
}

/// Answer one request.
fn serve(mut stream: TcpStream, table: &Table, log: &Mutex<Vec<Recorded>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    // Drain the headers; this client always sends `Connection: close`-compatible
    // requests and never negotiates a body it would not send.
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
    }

    let target = request_line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    let tag = query_param(&query, "tag");
    log.lock().expect("log").push(Recorded { path: path.clone(), query: query.clone(), tag: tag.clone() });

    let Some(bytes) = respond(table, &path, tag.as_deref()) else {
        return write_response(
            &mut stream,
            404,
            "text/plain",
            format!("no route for {path}").as_bytes(),
        );
    };
    write_response(&mut stream, 200, content_type(&path), &bytes)
}

fn content_type(path: &str) -> &'static str {
    if path.ends_with(".json") || path.contains("getBuild") || path.contains("getGame") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}

fn respond(table: &Table, path: &str, tag: Option<&str>) -> Option<Vec<u8>> {
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segs.as_slice() {
        ["hyp", endpoint] => table.hyp.get(*endpoint).cloned(),
        ["sophon", "getBuild"] => {
            let tag = tag?;
            match table.builds.get(tag) {
                Some(b) => Some(b.clone()),
                // The measured way of saying "no such tag": HTTP 200, retcode -202.
                None => Some(
                    serde_json::to_vec(&serde_json::json!({
                        "retcode": -202,
                        "message": "build not found",
                        "data": serde_json::Value::Null
                    }))
                    .expect("render json"),
                ),
            }
        }
        ["cdn", _build, "manifest", id] => {
            table.archives.values().find(|a| a.id == *id).map(|a| a.raw.clone())
        }
        ["cdn", _build, "chunk", id] => table.chunks.get(*id).map(|c| c.raw.clone()),
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
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
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
