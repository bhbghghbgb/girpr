//! The loopback replay server behind `tests/artifact_bridge.rs`.
//!
//! Spec: [`docs/06-artifact-bridge-spec.md`](../../../docs/06-artifact-bridge-spec.md)
//! §4. This module is reached from that one test file and from nowhere else:
//! `mod server;` in `repair_offline.rs` / `cli_contract.rs` would put a
//! several-hundred-line replay server into both of those compilation units for
//! tests that never use it.
//!
//! **What it does.** Opens one `gircap` artifact bundle **read-only** and serves
//! its bytes on ephemeral `127.0.0.1` ports, so the real `girpr` binary can be
//! pointed at it with `--hyp-base` / `--sophon-base` and patch a scratch copy of
//! the bundle's `game/` fixture.
//!
//! **Four listeners, one route family each** (`docs/06` §4 calls them "four
//! families, six listeners"; the count that matters is that *manifests and
//! chunks are on different ports*, which is what makes a crossed prefix
//! impossible rather than merely unlikely):
//!
//! | listener | request | served from |
//! |---|---|---|
//! | hyp | `GET <hyp>/getGameConfigs` etc. | `api/get*.json`, verbatim |
//! | build | `GET <sophon>/getBuild[?tag=]` | `api/getBuild.<tag>.json`, **rewritten** |
//! | manifest | `GET <manifest-prefix>/<manifest-id>` | `manifests/<tag>/<field>.manifest.zst` |
//! | chunk | `GET <chunk-prefix>/<chunk-id>` | `chunks/blobs/<chunk-id>.zst` |
//!
//! **The rewrite (`docs/06` §4.1) is the whole trick.** `--hyp-base` and
//! `--sophon-base` redirect the metadata calls only; manifest and chunk URLs are
//! built by `sophon::join_url` from a `url_prefix` read out of the *response
//! body*, so a body served verbatim sends `girpr` to the production CDN no
//! matter what the flags say. So each `manifests[].chunk_download.url_prefix`
//! and `manifests[].manifest_download.url_prefix` has its **scheme and
//! authority** swapped for the listener's own, keeping the path byte-for-byte.
//! The bundle on disk is never written — the ports are only known after the
//! listeners bind, and the bundle has to still pass `gircap verify`
//! afterwards.
//!
//! Two body fields are load-bearing and must survive the rewrite untouched:
//! `manifest.checksum` (MD5 of the **decompressed** manifest, verified by
//! `sophon::fetch_manifest` before the protobuf is parsed — altering it kills
//! the run at metadata with exit `2`) and `manifest.id` (it is the URL path
//! segment). `rewrite_changes_only_the_two_url_prefixes` pins that.
//!
//! **The `build_id` invariant (`docs/06` §4.1).** Every blob request carries
//! `<build_id>` as the second-to-last path segment, and it must name the build
//! that declared the id. That is enforced strictly for manifests (the map is
//! built while rewriting, `docs/06` §4.2). For chunks it is enforced as "the
//! latest build's id, or `blob_wrong_build`", because that is every request
//! `girpr` can make — chunk URLs come from the *latest* build's
//! `chunk_download.url_prefix` (`repair::manifest::fetch_latest`), and the
//! bundle does not record a per-build chunk declaration to attribute against
//! (`chunks/index.json` records which captures referenced a blob, not which
//! build's manifest declares it).
//!
//! **Nothing here invents a byte.** Every served body comes out of a file the
//! bundle vouches for; the only synthesised content is the failure shapes, and
//! those are replayed from `api/error_shapes.json` (`docs/06` §4.4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

// ---------------------------------------------------------------------------
// request log (`docs/06` §4.7)
// ---------------------------------------------------------------------------

/// Which route family a request arrived on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HitKind {
    /// `getGameConfigs` / `getGameBranches` / `getGameDeprecatedFileConfigs`.
    Api,
    /// Sophon `getBuild`.
    Build,
    /// A manifest blob.
    Manifest,
    /// A chunk blob.
    Chunk,
}

impl HitKind {
    fn label(self) -> &'static str {
        match self {
            HitKind::Api => "api",
            HitKind::Build => "build",
            HitKind::Manifest => "manifest",
            HitKind::Chunk => "chunk",
        }
    }
}

/// One request, as the server recorded it.
///
/// The log — not `SUMMARY`, and not the trace log — is the source for
/// `expectations.must_not_fetch`. A `download_bytes` total cannot see a chunk
/// that was fetched when it should have been reused *locally*; a substring
/// search over `--log-file` cannot either, because a reused chunk is logged
/// too, and matching the bare id false-positives on the "reused it" line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub kind: HitKind,
    /// The API name, or the manifest / chunk id.
    pub id: String,
    /// The `<build_id>` path segment; `None` on the JSON routes.
    pub build: Option<String>,
}

impl std::fmt::Display for Hit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.build {
            Some(b) => write!(f, "{} {} ({b})", self.kind.label(), self.id),
            None => write!(f, "{} {}", self.kind.label(), self.id),
        }
    }
}

// ---------------------------------------------------------------------------
// the bundle, opened read-only
// ---------------------------------------------------------------------------

/// One entry of the capture matrix: a `patchmaps/<id>.json` plus the identity
/// its driver needs.
#[derive(Clone, Debug)]
pub struct Capture {
    /// The `capture_id`, which is also the `patchmaps/<id>.json` stem.
    pub id: String,
    /// The fixture path, as `/`-separated and relative to `--game-path`.
    pub path: String,
    pub classes: Vec<String>,
    /// Size of the file on disk in the bundle's `game/` fixture.
    pub old_size: u64,
    pub new_size: u64,
    /// `Σ compressed_size` over the chunks with no old-side match, or `None`
    /// for a `removed` patchmap (`totals: null` — there is nothing to patch).
    pub download_bytes: Option<u64>,
    /// Ground truth: the patched file's bytes live at this bundle-relative path.
    pub result_path: String,
    pub result_md5: String,
    /// `true` when the file must be **gone** afterwards.
    pub absent_after: bool,
    /// Chunk ids that must never be requested (`docs/06` §6 assertion 3).
    pub must_not_fetch: Vec<String>,
}

impl Capture {
    /// `removed` is the one class with no repair to do, so it gets the
    /// `--purge-only` run and the purge counters instead of a byte total.
    pub fn is_purge_only(&self) -> bool {
        self.download_bytes.is_none()
    }
}

/// A `gircap` artifact bundle, opened read-only.
///
/// Nothing in this struct is a cache of a live server: every path points into
/// the bundle, and every rule is derived from what the bundle recorded. The
/// bridge never writes here — `docs/06` §6.4: the bundle is the *input*, a
/// scratch copy is the working tree.
#[derive(Clone, Debug)]
pub struct Bundle {
    pub root: PathBuf,
    /// `MANIFEST.json → biz.id`, which is what `--biz` must be given.
    pub biz: String,
    pub old_tag: String,
    pub new_tag: String,
    pub old_build: String,
    pub new_build: String,
    /// The capture matrix, ordered by capture id so runs are comparable.
    pub captures: Vec<Capture>,
    /// `MANIFEST.json → capture.id`.
    pub capture_id: String,
}

impl Bundle {
    /// Open a bundle. Panics carry the path, because every failure here is
    /// "this directory is not a bundle", which is a human mistake.
    pub fn open(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join("MANIFEST.json");
        let manifest: Value = read_json(&manifest_path);
        let versions = &manifest["versions"];
        let old_tag = str_field(&versions["old"]["tag"], &manifest_path);
        let new_tag = str_field(&versions["new"]["tag"], &manifest_path);
        let old_build = str_field(&versions["old"]["build_id"], &manifest_path);
        let new_build = str_field(&versions["new"]["build_id"], &manifest_path);
        let biz = str_field(&manifest["biz"]["id"], &manifest_path);
        let capture_id = str_field(&manifest["capture"]["id"], &manifest_path);

        let mut captures: Vec<Capture> = std::fs::read_dir(root.join("patchmaps"))
            .unwrap_or_else(|e| panic!("read patchmaps/ in {}: {e}", root.display()))
            .map(|e| {
                let path = e.expect("read patchmaps entry").path();
                if path.extension().is_some_and(|x| x == "json") {
                    Some(read_capture(&path))
                } else {
                    None
                }
            })
            .collect::<Option<Vec<_>>>()
            .unwrap_or_default();
        captures.sort_by(|a, b| a.id.cmp(&b.id));

        Self {
            root,
            biz,
            old_tag,
            new_tag,
            old_build,
            new_build,
            captures,
            capture_id,
        }
    }

    /// The four `api/*.json` bodies served verbatim by the hyp listener.
    pub fn api_body(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.root.join("api").join(name))
            .unwrap_or_else(|e| panic!("read api/{name} in {}: {e}", self.root.display()))
    }

    /// The recorded `api/getBuild.<tag>.json`, verbatim, before any rewrite.
    pub fn recorded_build_body(&self, tag: &str) -> Vec<u8> {
        self.api_body(&format!("getBuild.{tag}.json"))
    }

    /// `manifests/<tag>/<field>.manifest.zst`, if the bundle carries it.
    pub fn manifest_archive(&self, tag: &str, field: &str) -> Option<PathBuf> {
        let p = self
            .root
            .join("manifests")
            .join(tag)
            .join(format!("{field}.manifest.zst"));
        p.is_file().then_some(p)
    }

    /// `chunks/blobs/<id>.zst`, if the bundle carries it.
    pub fn chunk_blob(&self, id: &str) -> Option<PathBuf> {
        // The id reaches here straight out of a URL path segment, so refuse
        // anything that could climb out of the bundle before touching the disk.
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            return None;
        }
        let p = self
            .root
            .join("chunks")
            .join("blobs")
            .join(format!("{id}.zst"));
        p.is_file().then_some(p)
    }
}

fn read_capture(path: &Path) -> Capture {
    let v = read_json(path);
    let ex = &v["expectations"];
    // `totals: null` is the `removed` class's shape: nothing to patch, so there
    // is no byte total to compare against and the capture is driven by the
    // purge counters instead (`docs/06` §5.1).
    let download_bytes = v["totals"]["download_bytes"].as_u64();
    Capture {
        id: str_field(&v["capture_id"], path),
        path: str_field(&v["path"], path),
        classes: v["classes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        old_size: v["old"]["size"].as_u64().unwrap_or(0),
        new_size: v["new"]["size"].as_u64().unwrap_or(0),
        download_bytes,
        result_path: ex["result_path"].as_str().unwrap_or_default().to_string(),
        result_md5: ex["result_md5"].as_str().unwrap_or_default().to_string(),
        absent_after: ex["absent_after"].as_bool().unwrap_or(false),
        must_not_fetch: ex["must_not_fetch"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn read_json(path: &Path) -> Value {
    let raw = std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn str_field(v: &Value, path: &Path) -> String {
    v.as_str()
        .unwrap_or_else(|| panic!("{} is missing a string field", path.display()))
        .to_string()
}

// ---------------------------------------------------------------------------
// the bridge: four listeners over one bundle
// ---------------------------------------------------------------------------

/// The four origins a `girpr` run is pointed at.
#[derive(Clone, Debug)]
pub struct Origins {
    pub hyp_base: String,
    pub sophon_base: String,
    /// Not a `--flag` target: these two exist only inside rewritten bodies, as
    /// the `url_prefix` `girpr` will build manifest and chunk URLs from.
    pub manifest_origin: String,
    pub chunk_origin: String,
}

/// One manifest id, and where the archive behind it lives.
///
/// Built **while rewriting the `getBuild` body** (`docs/06` §4.2): the loop
/// over `manifests[]` is where every id in the bundle passes through, and a
/// filesystem scan over `manifests/*/*.meta.json` would answer a different
/// question — "is this archive in the bundle" rather than "did *this build*
/// declare it", which is exactly the `build_id` distinction.
#[derive(Clone, Debug)]
struct ManifestEntry {
    id: String,
    field: String,
    tag: String,
    build_id: String,
    /// `manifests/<tag>/<field>.manifest.zst`, if the bundle carries it. A
    /// `getBuild` body declares five manifests and the bundle stores two, so
    /// this is `None` more often than not — and that is the `404` a
    /// `--audio` widening would hit (`docs/06` §4.5).
    archive: Option<PathBuf>,
}

/// Everything the listeners serve from. One instance per `Bridge`.
struct Served {
    bundle: Bundle,
    /// Rewritten `getBuild` bodies, keyed by the tag they answer for.
    builds: HashMap<String, Vec<u8>>,
    manifest_index: HashMap<String, ManifestEntry>,
    /// `api/error_shapes.json`, when the bundle has one.
    shapes: Option<HashMap<String, Shape>>,
    /// `true` when the bundle predates the file and the not-found shapes are
    /// being synthesized rather than replayed. The bridge logs this rather than
    /// passing the not-found assertions silently (`docs/06` §4.4.2).
    shapes_synthesized: bool,
    log: Mutex<Vec<Hit>>,
}

/// A measured failure shape, replayed verbatim except for the fields the
/// document marks volatile (`docs/06` §4.4.1).
struct Shape {
    status: u16,
    content_type: String,
    body: String,
}

/// The running bridge. Dropping it stops the listeners.
pub struct Bridge {
    served: Arc<Served>,
    handles: Vec<tokio::task::JoinHandle<()>>,
    origins: Origins,
}

impl Bridge {
    /// Bind the four listeners, then build the served state, then serve.
    ///
    /// The order is forced: the rewrite needs the ports, and the ports only
    /// exist once the listeners are bound. Binding before loading also means a
    /// bundle that cannot be loaded fails after four sockets have been opened,
    /// which the drop below cleans up.
    pub async fn start(bundle: Bundle) -> Self {
        let hyp = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind hyp listener");
        let build = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind sophon getBuild listener");
        let manifest = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind manifest listener");
        let chunk = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind chunk listener");

        let origin = |l: &tokio::net::TcpListener| {
            format!(
                "http://127.0.0.1:{}",
                l.local_addr().expect("local addr").port()
            )
        };
        let served = Arc::new(Served::load(bundle, &origin(&manifest), &origin(&chunk)));

        let origins = Origins {
            hyp_base: format!("{}/hyp/hyp-connect/api", origin(&hyp)),
            sophon_base: format!("{}/downloader/sophon_chunk/api", origin(&build)),
            manifest_origin: origin(&manifest),
            chunk_origin: origin(&chunk),
        };

        let mut handles = Vec::new();
        for (listener, route) in [
            (hyp, Route::Hyp),
            (build, Route::Build),
            (manifest, Route::Manifest),
            (chunk, Route::Chunk),
        ] {
            let served = served.clone();
            handles.push(tokio::spawn(async move {
                accept_loop(listener, route, served).await;
            }));
        }

        Self {
            served,
            handles,
            origins,
        }
    }

    /// The `--hyp-base` / `--sophon-base` values for one run.
    pub fn origins(&self) -> &Origins {
        &self.origins
    }

    pub fn bundle(&self) -> &Bundle {
        &self.served.bundle
    }

    /// Everything the server has been asked for, in order.
    pub fn hits(&self) -> Vec<Hit> {
        self.served.log.lock().expect("request log").clone()
    }

    /// The chunk ids requested so far — the set `must_not_fetch` is checked
    /// against. The log records the *request*, whether or not it was served,
    /// which is what makes "asked once and 404'd" visible rather than "never
    /// asked".
    pub fn chunk_ids(&self) -> Vec<String> {
        self.hits()
            .into_iter()
            .filter(|h| h.kind == HitKind::Chunk)
            .map(|h| h.id)
            .collect()
    }

    /// The `<build_id>` each chunk request carried, paired with its id. The
    /// `build_id` invariant (`docs/06` §4.1) is assertable from this.
    pub fn chunk_requests(&self) -> Vec<(String, String)> {
        self.hits()
            .into_iter()
            .filter(|h| h.kind == HitKind::Chunk)
            .map(|h| (h.build.unwrap_or_default(), h.id))
            .collect()
    }

    /// `true` when the bundle carried no `api/error_shapes.json`, so the
    /// not-found shapes are synthesized from the measured live behaviour rather
    /// than replayed from evidence.
    pub fn shapes_synthesized(&self) -> bool {
        self.served.shapes_synthesized
    }

    /// The recorded shape for an id, for tests that want to compare the
    /// replay against the document.
    pub fn recorded_shape(&self, id: &str) -> Option<(u16, String, String)> {
        self.served
            .shapes
            .as_ref()
            .and_then(|s| s.get(id))
            .map(|s| (s.status, s.content_type.clone(), s.body.clone()))
    }

    /// One capture, by its id.
    pub fn capture(&self, id: &str) -> Option<&Capture> {
        self.served.bundle.captures.iter().find(|c| c.id == id)
    }

    /// The rewritten `getBuild` body for a tag, as it is served.
    pub fn served_build(&self, tag: &str) -> Option<Vec<u8>> {
        self.served.builds.get(tag).cloned()
    }

    /// Where a manifest id resolves: `(tag, field, declaring build_id)`, and
    /// whether the bundle actually carries that field's archive.
    ///
    /// The last element is the `docs/06` §4.5 fact stated as data: a `getBuild`
    /// body declares five manifests and the bundle stores two, so three ids
    /// resolve to an archive that does not exist — and that is exactly what a
    /// widened `--audio` would ask for.
    pub fn manifest_resolution(&self, id: &str) -> Option<(String, String, String, bool)> {
        self.served.manifest_index.get(id).map(|e| {
            (
                e.tag.clone(),
                e.field.clone(),
                e.build_id.clone(),
                e.archive.is_some(),
            )
        })
    }

    /// Every manifest id the served `getBuild` bodies declare.
    pub fn declared_manifest_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.served.manifest_index.keys().cloned().collect();
        ids.sort();
        ids
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        for h in &self.handles {
            h.abort();
        }
    }
}

/// `Served::load` needs the manifest and chunk origins, which only exist after
/// the listeners bind — hence the split from `Bridge::start`.
impl Served {
    fn load(bundle: Bundle, manifest_origin: &str, chunk_origin: &str) -> Self {
        let mut builds = HashMap::new();
        let mut manifest_index: HashMap<String, ManifestEntry> = HashMap::new();

        // The rewrite loop, over every tag the bundle recorded.
        for tag in [&bundle.old_tag, &bundle.new_tag] {
            let recorded = bundle.recorded_build_body(tag);
            let mut body: Value = serde_json::from_slice(&recorded)
                .unwrap_or_else(|e| panic!("parse api/getBuild.{tag}.json: {e}"));
            let build_id = body["data"]["build_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let manifests = body["data"]["manifests"]
                .as_array_mut()
                .unwrap_or_else(|| panic!("api/getBuild.{tag}.json declares no manifests"));

            // The same loop does both jobs `docs/06` §4.2 asks for: rewrite the
            // prefix, and index the id. Every id in the bundle passes through
            // here exactly once.
            let mut declared: Vec<(String, String)> = Vec::with_capacity(manifests.len());
            for m in manifests.iter_mut() {
                // ONLY these two fields change, and only their scheme and
                // authority. `manifest.checksum` and `manifest.id` are
                // load-bearing and must come through byte-identical — the
                // checksum is MD5 of the *decompressed* manifest and
                // `sophon::fetch_manifest` verifies it before parsing.
                for (field, origin) in [
                    ("manifest_download", manifest_origin),
                    ("chunk_download", chunk_origin),
                ] {
                    let recorded_prefix = m[field]["url_prefix"]
                        .as_str()
                        .unwrap_or_else(|| panic!("{field}.url_prefix in api/getBuild.{tag}.json"));
                    let rewritten = swap_authority(recorded_prefix, origin)
                        .unwrap_or_else(|e| panic!("rewrite {field} url_prefix: {e}"));
                    m[field]["url_prefix"] = Value::String(rewritten);
                }
                declared.push((
                    m["manifest"]["id"]
                        .as_str()
                        .expect("manifest.id")
                        .to_string(),
                    m["matching_field"]
                        .as_str()
                        .expect("matching_field")
                        .to_string(),
                ));
            }

            for (id, field) in declared {
                manifest_index.insert(
                    id.clone(),
                    ManifestEntry {
                        id,
                        // A `getBuild` body declares five manifests and the
                        // bundle stores the two the capture kept, so `archive`
                        // is `None` more often than not. That is the `404` a
                        // widened `--audio` hits (`docs/06` §4.5).
                        archive: bundle.manifest_archive(tag, &field),
                        field,
                        tag: tag.to_string(),
                        build_id: build_id.clone(),
                    },
                );
            }
            builds.insert(
                tag.to_string(),
                serde_json::to_vec(&body).expect("serialize"),
            );
        }

        let shapes = bundle
            .root
            .join("api")
            .join("error_shapes.json")
            .is_file()
            .then(|| {
                let v = read_json(&bundle.root.join("api").join("error_shapes.json"));
                v["shapes"]
                    .as_array()
                    .expect("error_shapes.json shapes[]")
                    .iter()
                    .map(|s| {
                        (
                            s["id"].as_str().expect("shape id").to_string(),
                            Shape {
                                status: s["status"].as_u64().expect("shape status") as u16,
                                content_type: s["content_type"]
                                    .as_str()
                                    .expect("shape content_type")
                                    .to_string(),
                                body: s["body"].as_str().expect("shape body").to_string(),
                            },
                        )
                    })
                    .collect()
            });

        Self {
            shapes_synthesized: shapes.is_none(),
            shapes,
            builds,
            manifest_index,
            bundle,
            log: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, hit: Hit) {
        self.log.lock().expect("request log").push(hit);
    }

    /// The `getBuild` body for a tag, or the recorded `-202` for one the
    /// bundle does not carry.
    ///
    /// A tagless request serves `versions.new.tag`. Getting this wrong is the
    /// worst failure mode in `docs/06` §4.3, because it is **green**: the run
    /// falls back to an empty reuse map, downloads everything, and still exits
    /// `0`. That is why the driver asserts both the byte figure and
    /// `local_manifest` (`docs/06` §6).
    fn build_response(&self, tag: Option<&str>) -> Response {
        let tag = tag.unwrap_or(&self.bundle.new_tag);
        let Some(body) = self.builds.get(tag) else {
            return self.error_shape("getbuild_unknown_tag", "", None);
        };
        Response::ok_json(body.clone())
    }

    fn manifest_response(&self, path: &str) -> Response {
        let Some((build_id, id)) = last_two(path) else {
            return self.error_shape("blob_missing", path, None);
        };
        match self.manifest_index.get(&id) {
            None => self.error_shape("blob_missing", path, None),
            // `docs/06` §4.1: the id exists, but not under *this* build. The
            // same id under the other build's prefix is a 404 in production,
            // and that is the whole reason the two namespaces are separate
            // ports.
            Some(e) if e.build_id != build_id => self.error_shape("blob_wrong_build", path, None),
            Some(e) => match &e.archive {
                Some(archive) => Response::octet_stream(manifest_bytes(archive)),
                // Declared by the build, absent from the bundle.
                None => self.error_shape("blob_missing", path, None),
            },
        }
    }

    fn chunk_response(&self, path: &str) -> Response {
        let Some((build_id, id)) = last_two(path) else {
            return self.error_shape("chunk_missing", path, None);
        };
        // Every chunk download goes to the latest build's prefix
        // (`repair::manifest::fetch_latest`), so any other build id is wrong by
        // construction — and a shim that crossed the two blob prefixes would
        // produce exactly that.
        if build_id != self.bundle.new_build {
            return self.error_shape("blob_wrong_build", path, None);
        }
        match self.bundle.chunk_blob(&id) {
            Some(blob) => Response::chunk(std::fs::read(blob).expect("read chunk blob")),
            None => self.error_shape("chunk_missing", path, None),
        }
    }

    /// Replay a recorded failure shape, synthesizing only the fields
    /// `docs/06` §4.4.1 marks volatile: `RequestId`, `HostId` and `Key` (and
    /// the body length that follows from `Key`). `girpr` calls
    /// `.error_for_status()` and never reads the body, so **status and
    /// content-type are the contract** and the body is fidelity, for a human
    /// reading this server's log.
    ///
    /// With no `api/error_shapes.json` the measured live shape is used
    /// instead, and [`Bridge::shapes_synthesized`] says so, so the bridge can
    /// report that those assertions were skipped for lack of evidence rather
    /// than inventing a status it has no record of.
    fn error_shape(&self, id: &str, path: &str, extra: Option<&str>) -> Response {
        let key = path.trim_start_matches('/');
        let Some(shape) = self.shapes.as_ref().and_then(|s| s.get(id)) else {
            return Response {
                status: 404,
                content_type: "application/xml".to_string(),
                body: Arc::new(no_such_key_body(key)),
            };
        };
        let request_id = synthetic_request_id(key);
        let mut body = shape.body.clone();
        for (tag, value) in [("Key", key), ("RequestId", request_id.as_str())] {
            body = replace_element(&body, tag, value);
        }
        let _ = extra;
        Response {
            status: shape.status,
            content_type: shape.content_type.clone(),
            body: Arc::new(body.into_bytes()),
        }
    }
}

/// Replace `<tag>…</tag>`'s contents. The recorded bodies are single-line XML
/// and single-line JSON, so a textual replace of the element's span is enough
/// and avoids pulling in an XML parser for four static bodies.
fn replace_element(body: &str, tag: &str, value: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    match (body.find(&open), body.find(&close)) {
        (Some(a), Some(b)) if b > a + open.len() => {
            let mut out = String::with_capacity(body.len() + value.len());
            out.push_str(&body[..a + open.len()]);
            out.push_str(value);
            out.push_str(&body[b..]);
            out
        }
        // `HostId` is recorded empty, and `<HostId></HostId>` is the shape.
        _ => body.to_string(),
    }
}

/// The AliyunOSS `NoSuchKey` body, used only when the bundle has no recorded
/// shape to replay. `docs/06` §4.4 measured it; nothing here is invented.
fn no_such_key_body(key: &str) -> Vec<u8> {
    let mut b = String::new();
    b.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Error>\n  <Code>NoSuchKey</Code>\n");
    b.push_str("  <Message>The specified key does not exist.</Message>\n");
    b.push_str(&format!(
        "  <RequestId>{}</RequestId>\n",
        synthetic_request_id(key)
    ));
    b.push_str("  <HostId></HostId>\n");
    b.push_str(&format!("  <Key>{}</Key>\n", xml_escape(key)));
    b.push_str("  <EC>0026-00000001</EC>\n  <RecommendDoc></RecommendDoc>\n</Error>\n");
    b.into_bytes()
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A `RequestId` shaped like the recorded one (24 uppercase hex) but derived
/// from the key, so a given request replays identically on every run.
fn synthetic_request_id(key: &str) -> String {
    format!("{:x}", md5::compute(key.as_bytes())).to_uppercase()
}

/// Replace a prefix's **scheme and authority**, keeping the path.
///
/// Replacing the whole prefix with one origin would collapse the manifest and
/// chunk namespaces into a single root and stop them being distinguishable;
/// keeping the path is what lets a blob route be "take the last two segments
/// and ignore the rest", exactly as production's is (`docs/06` §4.1).
fn swap_authority(prefix: &str, origin: &str) -> Result<String, String> {
    let Some((_scheme, rest)) = prefix.split_once("://") else {
        return Err(format!("{prefix:?} has no scheme://authority"));
    };
    let path = match rest.find('/') {
        Some(i) => &rest[i..],
        None => "",
    };
    Ok(format!("{origin}{path}"))
}

/// The last two path segments: `([build_id], [id])`.
///
/// Every blob route is `<prefix>/<build_id>/<id>` and the route is
/// indistinguishable from production's, which is the point: the bridge still
/// proves `girpr` builds URLs from `url_prefix` + `id` alone.
fn last_two(path: &str) -> Option<(String, String)> {
    let mut it = path
        .trim_start_matches('/')
        .split('/')
        .rev()
        .filter(|s| !s.is_empty());
    let id = it.next()?.to_string();
    let build_id = it.next()?.to_string();
    Some((build_id, id))
}

// ---------------------------------------------------------------------------
// served-manifest bytes cache (`docs/06` §4.6)
// ---------------------------------------------------------------------------

/// Manifest archives are immutable per bundle and ~40 MB of a ~130 MB bundle,
/// so every run re-reading them is ~320 MB across eight runs for nothing.
///
/// The cache is **outside the listeners and shared by all of them** — including
/// across the eight per-capture bridges — because it holds bytes that are a
/// function of the bundle alone, so nothing about per-capture isolation
/// (`docs/06` §6.3) is lost by sharing it, and every capture pays the disk read
/// once instead of eight times.
static MANIFEST_CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<u8>>>>> = OnceLock::new();

fn manifest_bytes(path: &Path) -> Arc<Vec<u8>> {
    let cache = MANIFEST_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().expect("manifest cache");
    if let Some(b) = guard.get(path) {
        return b.clone();
    }
    let bytes = Arc::new(std::fs::read(path).expect("read manifest archive"));
    guard.insert(path.to_path_buf(), bytes.clone());
    bytes
}

// ---------------------------------------------------------------------------
// the HTTP layer
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Route {
    Hyp,
    Build,
    Manifest,
    Chunk,
}

struct Response {
    status: u16,
    content_type: String,
    /// `Arc` so a manifest served from the shared cache is not memcpy'd on
    /// every request (`docs/06` §4.6).
    body: Arc<Vec<u8>>,
}

impl Response {
    fn ok_json(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            content_type: "application/json".to_string(),
            body: Arc::new(body),
        }
    }
    fn octet_stream(body: Arc<Vec<u8>>) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream".to_string(),
            body,
        }
    }
    fn chunk(bytes: Vec<u8>) -> Self {
        Self::octet_stream(Arc::new(bytes))
    }
    fn text(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type: "text/plain".to_string(),
            body: Arc::new(body),
        }
    }
}

async fn accept_loop(listener: tokio::net::TcpListener, route: Route, served: Arc<Served>) {
    loop {
        let Ok((socket, _)) = listener.accept().await else {
            continue;
        };
        let served = served.clone();
        tokio::spawn(async move { serve_one(socket, route, served).await });
    }
}

async fn serve_one(mut socket: tokio::net::TcpStream, route: Route, served: Arc<Served>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buf = vec![0u8; 65536];
    let mut got = 0usize;
    // GET has no body, so the header terminator is the whole request.
    loop {
        match socket.read(&mut buf[got..]).await {
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
    let path = target.split('?').next().unwrap_or("/").to_string();

    let response = match route {
        Route::Hyp => hyp_response(&served, &path),
        Route::Build => {
            let tag = query_value(&target, "tag");
            served.record(Hit {
                kind: HitKind::Build,
                id: tag.clone().unwrap_or_else(|| "latest".to_string()),
                build: None,
            });
            served.build_response(tag.as_deref())
        }
        Route::Manifest => {
            if let Some((build_id, id)) = last_two(&path) {
                served.record(Hit {
                    kind: HitKind::Manifest,
                    id,
                    build: Some(build_id),
                });
            }
            served.manifest_response(&path)
        }
        Route::Chunk => {
            if let Some((build_id, id)) = last_two(&path) {
                served.record(Hit {
                    kind: HitKind::Chunk,
                    id,
                    build: Some(build_id),
                });
            }
            served.chunk_response(&path)
        }
    };

    let reason = match response.status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason,
        response.content_type,
        response.body.len()
    );
    let _ = socket.write_all(head.as_bytes()).await;
    let _ = socket.write_all(&response.body).await;
    let _ = socket.flush().await;
    let _ = socket.shutdown().await;
}

/// The three HoYoPlay calls, served verbatim: nothing in them carries a
/// `url_prefix`, so there is nothing to rewrite.
fn hyp_response(served: &Served, path: &str) -> Response {
    let name = path
        .trim_start_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let body = match name {
        "getGameConfigs" => served.bundle.api_body("getGameConfigs.json"),
        "getGameBranches" => served.bundle.api_body("getGameBranches.json"),
        "getGameDeprecatedFileConfigs" => {
            served.bundle.api_body("getGameDeprecatedFileConfigs.json")
        }
        _ => {
            served.record(Hit {
                kind: HitKind::Api,
                id: name.to_string(),
                build: None,
            });
            return Response::text(404, b"unknown hyp api".to_vec());
        }
    };
    served.record(Hit {
        kind: HitKind::Api,
        id: name.to_string(),
        build: None,
    });
    Response::ok_json(body)
}

fn query_value(target: &str, key: &str) -> Option<String> {
    let q = target.split_once('?')?.1;
    q.split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{key}=")).map(urldecode))
}

/// `girpr` percent-encodes every non-alphanumeric byte (`hyp::urlencode`), so
/// a tag comes back as `%2E`-style escapes only if it has any; handle them
/// rather than assume.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
