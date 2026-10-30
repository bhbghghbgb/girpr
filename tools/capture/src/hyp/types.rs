//! The response types, including the fields a minimal client throws away.
//!
//! **M2.** `stats`, `compression`, `encryption`, `password`, `categories[]` and
//! `deduplicated_stats` are modelled even though nothing here acts on all of
//! them: a capture that drops them cannot answer the question they were kept
//! for, which is what the captured API was doing at that moment.
//!
//! ## The numbers are strings
//!
//! *Measured 2026-10-14:* **every** numeric field in a `getBuild` body arrives as
//! a JSON **string** — `manifest.compressed_size` is `"9007187"`, and so are
//! `stats.file_count` and `stats.chunk_count`. A client that writes `size: u64`
//! and a client that writes `size: String` both appear to work until one of them
//! meets the other shape. [`IntOrStr`] accepts both and preserves whichever
//! arrived, so a re-serialisation is byte-comparable with the original.
//!
//! ## Forward-compatible by construction
//!
//! No type here carries `deny_unknown_fields`. HoYo adds fields without warning,
//! and a capture that refused to parse the day they did would be a tool that
//! stops working for a reason nobody chose. What *is* strict is the set of fields
//! the run acts on: those are required, and their absence is an error naming the
//! field.

use serde::{Deserialize, Serialize};

use crate::http::ApiError;

/// The only `default_download_mode` this tool can build a capture from.
///
/// `DOWNLOAD_MODE_FILE` and `DOWNLOAD_MODE_LDIFF` are whole-archive deliveries;
/// they need the 7z and hdiff paths, and a manifest captured under either would
/// describe an install that never chunk-repairs. Asserted at capture time — see
/// [`GameConfigs::require_chunk_mode`].
pub const DOWNLOAD_MODE_CHUNK: &str = "DOWNLOAD_MODE_CHUNK";

/// A number the API may send as an integer or as a decimal string.
///
/// Serialises back as whichever variant it was parsed from, so a typed value
/// round-trips to the same bytes the API sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IntOrStr {
    /// An actual JSON number.
    Int(i64),
    /// A JSON string holding a decimal integer. *Measured* for every size in
    /// `getBuild`.
    Str(String),
}

impl IntOrStr {
    /// The value as `u64`, or `None` when it is negative or not a number.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            IntOrStr::Int(i) => u64::try_from(*i).ok(),
            IntOrStr::Str(s) => s.trim().parse().ok(),
        }
    }

    /// The value as `i64`, or `None` when it is not a number.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            IntOrStr::Int(i) => Some(*i),
            IntOrStr::Str(s) => s.trim().parse().ok(),
        }
    }
}

impl std::fmt::Display for IntOrStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntOrStr::Int(i) => write!(f, "{i}"),
            IntOrStr::Str(s) => f.write_str(s),
        }
    }
}

impl From<u64> for IntOrStr {
    fn from(v: u64) -> Self {
        IntOrStr::Int(v as i64)
    }
}

impl From<i64> for IntOrStr {
    fn from(v: i64) -> Self {
        IntOrStr::Int(v)
    }
}

/// The API sends these as strings, and so does a caller writing a fixture — the
/// string form is the *faithful* one and gets no coercion on the way in.
impl From<&str> for IntOrStr {
    fn from(v: &str) -> Self {
        IntOrStr::Str(v.to_string())
    }
}

impl From<String> for IntOrStr {
    fn from(v: String) -> Self {
        IntOrStr::Str(v)
    }
}

/// `{"retcode":…,"message":…,"data":{…}}`, the wrapper every endpoint returns.
///
/// `retcode` is checked by [`crate::http::Fetcher::get_json`] before any of this
/// is reached, so a body that arrives here has already been shown to be `0`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wrapper<T> {
    /// `0` on success. `-202` on `getBuild` means *no such tag*.
    #[serde(default)]
    pub retcode: i64,
    /// `"OK"`, or the reason.
    #[serde(default)]
    pub message: String,
    /// The payload.
    pub data: T,
}

/// A required field that was absent.
///
/// Named precisely because "could not deserialize LaunchConfig" does not tell a
/// reader which of thirty fields changed.
fn missing(field: &'static str) -> ApiError {
    ApiError::shape(field, "the API response does not carry this field")
}

/// `data.game.id` / `data.game.biz` — present on nearly everything.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameRef {
    /// The game id, e.g. `gopR6Cufr3`.
    #[serde(default)]
    pub id: String,
    /// The biz name, e.g. `hk4e_global`.
    #[serde(default)]
    pub biz: String,
}

// ------------------------------------------------------------ getGameConfigs

/// `data.launch_configs[]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameConfigs {
    /// One entry per launcher the game is installed under.
    #[serde(default)]
    pub launch_configs: Vec<LaunchConfig>,
}

impl GameConfigs {
    /// The launcher's own configuration.
    ///
    /// There is exactly one for the game id this tool asked about; picking the
    /// first would silently take whichever the API listed first.
    pub fn launch(&self) -> Result<&LaunchConfig, ApiError> {
        self.launch_configs
            .first()
            .ok_or_else(|| ApiError::shape("getGameConfigs", "data.launch_configs is empty"))
    }

    /// `default_download_mode`.
    pub fn download_mode(&self) -> Result<&str, ApiError> {
        self.launch()?
            .default_download_mode
            .as_deref()
            .ok_or_else(|| missing("default_download_mode"))
    }

    /// Assert that the launcher is on chunk delivery.
    ///
    /// **This is the one place a launcher-side change can stop a capture**, and
    /// it is a hard failure rather than a warning on purpose. Under
    /// `DOWNLOAD_MODE_FILE` or `DOWNLOAD_MODE_LDIFF` the install is delivered as
    /// whole archives and never chunk-repairs, so every manifest captured against
    /// it would describe a patch that the game never performs. Producing such a
    /// bundle anyway would be worse than producing none: it would be internally
    /// consistent, pass every hash check, and be useless.
    ///
    /// If a future launcher defaults to a mode that *still* publishes manifests,
    /// this is the line to reconsider — and the reconsideration should be
    /// recorded here rather than made silently.
    pub fn require_chunk_mode(&self) -> Result<&str, ApiError> {
        let mode = self.download_mode()?;
        if mode != DOWNLOAD_MODE_CHUNK {
            return Err(ApiError::shape(
                "default_download_mode",
                format!(
                    "the launcher reports {mode:?}, not {DOWNLOAD_MODE_CHUNK:?}; a capture \
                     built from it would describe an install that never chunk-repairs"
                ),
            ));
        }
        Ok(mode)
    }

    /// `audio_pkg_scan_dir` — where the audio scan file lives inside the install.
    ///
    /// Never hardcoded: it is the launcher's path, and the fixture has to write
    /// the file where the tool under test looks for it.
    pub fn audio_pkg_scan_dir(&self) -> Result<&str, ApiError> {
        let dir = self
            .launch()?
            .audio_pkg_scan_dir
            .as_deref()
            .ok_or_else(|| missing("audio_pkg_scan_dir"))?;
        // The fixture writes this under `game/`, so it is a document-supplied
        // path on the same boundary as `--files` and a manifest entry. The
        // launcher is a trusted source only until `--hyp-base` points at a test
        // or proxy server, which this tool explicitly supports.
        if let Some(problem) = crate::paths::relative_path_problem(dir) {
            return Err(ApiError::shape("audio_pkg_scan_dir", problem));
        }
        Ok(dir)
    }

    /// `installation_dir` — the default install directory name.
    pub fn installation_dir(&self) -> Result<&str, ApiError> {
        self.launch()?
            .installation_dir
            .as_deref()
            .ok_or_else(|| missing("installation_dir"))
    }

    /// `exe_file_name`.
    pub fn exe_file_name(&self) -> Result<&str, ApiError> {
        self.launch()?
            .exe_file_name
            .as_deref()
            .ok_or_else(|| missing("exe_file_name"))
    }
}

/// One `launch_configs[]` entry.
///
/// Only the fields this tool acts on are required; the rest of the launcher's
/// configuration is captured in the raw body and not modelled here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchConfig {
    /// The game this configuration is for.
    #[serde(default)]
    pub game: GameRef,
    /// The executable to launch.
    #[serde(default)]
    pub exe_file_name: Option<String>,
    /// Default install directory name.
    #[serde(default)]
    pub installation_dir: Option<String>,
    /// Where the audio scan file is written, relative to the install.
    #[serde(default)]
    pub audio_pkg_scan_dir: Option<String>,
    /// Where resolved audio assets live.
    #[serde(default)]
    pub audio_pkg_res_dir: Option<String>,
    /// `DOWNLOAD_MODE_CHUNK` or not. *Measured* `DOWNLOAD_MODE_CHUNK`.
    #[serde(default)]
    pub default_download_mode: Option<String>,
    /// Whether voice packs can be managed at all.
    #[serde(default)]
    pub enable_audio_pkg_mgmt: Option<bool>,
    /// Whether the launcher offers hdiff patches. *Measured* `true`.
    #[serde(default)]
    pub enable_ldiff: Option<bool>,
}

// ----------------------------------------------------------- getGameBranches

/// `data.game_branches[]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameBranches {
    /// One entry per branch family.
    #[serde(default)]
    pub game_branches: Vec<GameBranch>,
}

impl GameBranches {
    /// The `main` branch.
    pub fn main(&self) -> Result<&Branch, ApiError> {
        self.game_branches
            .iter()
            .find(|b| b.main.as_ref().is_some_and(|m| m.branch == "main"))
            .and_then(|b| b.main.as_ref())
            .ok_or_else(|| {
                ApiError::shape(
                    "getGameBranches",
                    "no entry with main.branch == \"main\"; the branch the downloader \
                     serves is the one named `main` and nothing else will do",
                )
            })
    }
}

/// One `game_branches[]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameBranch {
    /// The game this branch belongs to.
    #[serde(default)]
    pub game: GameRef,
    /// The current branch.
    #[serde(default)]
    pub main: Option<Branch>,
}

/// The `main` branch.
///
/// These five fields are everything the downloader needs, and they are exactly
/// what a hardcoded client would have had to guess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// The downloader package. *Measured* `ScSYQBFhu9` for `hk4e_global`.
    pub package_id: String,
    /// Always `main` in practice.
    pub branch: String,
    /// The downloader password.
    pub password: String,
    /// The version the branch currently serves. *Measured* `7.1.0`.
    pub tag: String,
    /// The patch-support list. **Not** a retention list — see the spec §3.4.
    #[serde(default)]
    pub diff_tags: Vec<String>,
    /// The manifest categories this branch publishes.
    #[serde(default)]
    pub categories: Vec<Category>,
}

/// One `categories[]` entry: which `matching_field` a manifest is published for.
///
/// *Measured* five per branch: `game`, `zh-cn`, `en-us`, `ja-jp`, `ko-kr`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    /// `10016` for `game`, `10018` for `en-us`, and so on.
    #[serde(default)]
    pub category_id: String,
    /// The field a file entry is matched on — `game` or an audio code.
    pub matching_field: String,
    /// `CATEGORY_TYPE_RESOURCE` or `CATEGORY_TYPE_AUDIO`.
    #[serde(default, rename = "type")]
    pub kind: String,
    /// The scenarios the category is published under.
    #[serde(default)]
    pub scenarios: Vec<String>,
}

// ----------------------------------------------------- getGameDeprecatedFiles

/// `data.deprecated_file_configs[]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecatedFiles {
    /// One entry per game.
    #[serde(default)]
    pub deprecated_file_configs: Vec<DeprecatedConfig>,
}

impl DeprecatedFiles {
    /// The first entry's file list.
    ///
    /// As with the other list-shaped responses there is exactly one entry for
    /// the game asked about, and taking the first is the only reading that does
    /// not need a cross-check against the request.
    pub fn files(&self) -> Result<&[DeprecatedFile], ApiError> {
        self.deprecated_file_configs
            .first()
            .map(|c| c.deprecated_files.as_slice())
            .ok_or_else(|| {
                ApiError::shape(
                    "getGameDeprecatedFileConfigs",
                    "data.deprecated_file_configs is empty",
                )
            })
    }
}

/// One `deprecated_file_configs[]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecatedConfig {
    /// The game these files belong to.
    #[serde(default)]
    pub game: GameRef,
    /// The files the current version no longer ships.
    #[serde(default)]
    pub deprecated_files: Vec<DeprecatedFile>,
}

/// One `deprecated_files[]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecatedFile {
    /// The manifest-relative path.
    pub name: String,
}

// --------------------------------------------------------- getGamePackages

/// `data.game_packages[]`. **Captured as dated staleness evidence only.**
///
/// *Measured 2026-10-14:* `major.version` is `5.5.0` with patches `5.4.0` and
/// `5.3.0`, frozen at 2025-03-14 — about two major versions behind a `7.1.0`
/// branch. That is the whole reason the legacy endpoint is irrelevant to the
/// capture, and the bundle should be able to show that rather than assert it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamePackages {
    /// One entry per game.
    #[serde(default)]
    pub game_packages: Vec<GamePackage>,
}

impl GamePackages {
    /// The package list for the game asked about.
    pub fn package(&self) -> Result<&GamePackage, ApiError> {
        self.game_packages
            .first()
            .ok_or_else(|| ApiError::shape("getGamePackages", "data.game_packages is empty"))
    }

    /// The newest version the legacy path knows about.
    pub fn major_version(&self) -> Result<&str, ApiError> {
        self.package()?
            .main
            .as_ref()
            .map(|m| m.major.version.as_str())
            .ok_or_else(|| missing("main.major.version"))
    }

    /// The patches the legacy path advertises, oldest last.
    pub fn patch_versions(&self) -> Vec<String> {
        self.package()
            .ok()
            .and_then(|p| p.main.as_ref())
            .map(|m| m.patches.iter().map(|p| p.version.clone()).collect())
            .unwrap_or_default()
    }

    /// How far behind the branch this endpoint is, in version terms.
    ///
    /// Returns `(major_behind, patch_names)`, or `None` when the comparison
    /// itself is not meaningful — the branch is *behind* the legacy endpoint, or
    /// either version does not start with a number. Only the leading component
    /// is compared: `5` against `7` is a two-release gap, but the number of
    /// releases between `5.9.0` and `7.1.0` cannot be derived from two strings,
    /// and guessing a middle would put a fabricated number in the bundle.
    pub fn staleness(&self, branch_tag: &str) -> Option<(u64, Vec<String>)> {
        let major = self
            .major_version()
            .ok()?
            .split('.')
            .next()?
            .parse::<u64>()
            .ok()?;
        let live = branch_tag.split('.').next()?.parse::<u64>().ok()?;
        // `checked_sub`, not `then_some(live - major)`: `then_some` evaluates its
        // argument eagerly, so the branch being behind would overflow before the
        // comparison could refuse it. A panic in a measurement is worse than a
        // missing one.
        let behind = live.checked_sub(major)?;
        Some((behind, self.patch_versions()))
    }
}

/// One `game_packages[]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GamePackage {
    /// The game these packages are for.
    #[serde(default)]
    pub game: GameRef,
    /// The legacy package description.
    #[serde(default)]
    pub main: Option<LegacyMain>,
}

/// `main`: the legacy zip and hdiff listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyMain {
    /// The newest whole-game package set.
    #[serde(default)]
    pub major: LegacyRelease,
    /// Advertised patches, newest first.
    #[serde(default)]
    pub patches: Vec<LegacyRelease>,
}

/// One legacy release: a version and its archives.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LegacyRelease {
    /// The version.
    #[serde(default)]
    pub version: String,
    /// Whole-game archives. Modelled as counts; the URLs are in the raw body.
    #[serde(default)]
    pub game_pkgs: Vec<LegacyArchive>,
    /// Voice-pack archives, one per language.
    #[serde(default)]
    pub audio_pkgs: Vec<LegacyAudio>,
}

/// One legacy game archive part.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyArchive {
    /// The part URL.
    #[serde(default)]
    pub url: String,
    /// The archive MD5.
    #[serde(default)]
    pub md5: String,
    /// Compressed size. *Measured* a string.
    #[serde(default)]
    pub size: Option<IntOrStr>,
    /// Uncompressed size. *Measured* a string.
    #[serde(default)]
    pub decompressed_size: Option<IntOrStr>,
}

/// One legacy voice-pack archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyAudio {
    /// The language code, e.g. `en-us`.
    #[serde(default)]
    pub language: String,
    /// The archive URL.
    #[serde(default)]
    pub url: String,
    /// Compressed size. *Measured* a string.
    #[serde(default)]
    pub size: Option<IntOrStr>,
}

// ------------------------------------------------------------------ getBuild

/// `data` of a `getBuild` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBuild {
    /// The build identifier. Chunk URLs are namespaced per build id, so a chunk
    /// id fetched under the wrong one is a `404`.
    pub build_id: String,
    /// The tag this build serves. *Measured* `7.1.0`.
    pub tag: String,
    /// One entry per published manifest. *Measured* five for `hk4e_global`.
    #[serde(default)]
    pub manifests: Vec<BuildManifest>,
}

impl GetBuild {
    /// The manifest published for `matching_field`.
    ///
    /// There is exactly one per field, and the field is the join key the whole
    /// selection pool is built on, so a miss here is a structural problem rather
    /// than a lookup that happened to come up empty.
    pub fn manifest_for(&self, matching_field: &str) -> Result<&BuildManifest, ApiError> {
        self.manifests
            .iter()
            .find(|m| m.matching_field == matching_field)
            .ok_or_else(|| {
                ApiError::shape(
                    "getBuild",
                    format!(
                        "no manifest for matching_field {matching_field:?}; published: {:?}",
                        self.manifests
                            .iter()
                            .map(|m| &m.matching_field)
                            .collect::<Vec<_>>()
                    ),
                )
            })
    }
}

/// One `manifests[]` entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildManifest {
    /// `10016` for `game`.
    #[serde(default)]
    pub category_id: String,
    /// A human name. *Measured* non-ASCII; preserved verbatim.
    #[serde(default)]
    pub category_name: String,
    /// Identity and checksum of the archive blob.
    pub manifest: ManifestRef,
    /// Where its chunks live and how they are stored.
    #[serde(default)]
    pub chunk_download: Download,
    /// Where the archive itself lives.
    #[serde(default)]
    pub manifest_download: Download,
    /// `game`, or an audio code.
    pub matching_field: String,
    /// Totals the archive should reconcile against.
    #[serde(default)]
    pub stats: Option<Stats>,
    /// Totals after cross-build deduplication.
    ///
    /// A field a minimal client discards. It is kept because it is the
    /// difference between "this build has 113 610 chunks" and "113 610 of them
    /// are needed", which is a question a reader of a 128 GB capture will ask.
    #[serde(default)]
    pub deduplicated_stats: Option<Stats>,
}

impl BuildManifest {
    /// The archive's URL: `{url_prefix}/{url_suffix}` with the id appended.
    ///
    /// `{url_prefix}/{id}{url_suffix}` is the measured join order, and the suffix
    /// is empty in every response seen so far — it is modelled anyway, because a
    /// join that ignores a field the API sends is a join that breaks the day the
    /// field is used.
    pub fn archive_url(&self) -> String {
        join(&self.manifest_download, &self.manifest.id)
    }

    /// The URL of one chunk of this manifest.
    pub fn chunk_url(&self, chunk_id: &str) -> String {
        join(&self.chunk_download, chunk_id)
    }
}

/// `{url_prefix}/{id}{url_suffix}`.
fn join(d: &Download, id: &str) -> String {
    format!("{}/{id}{}", d.url_prefix, d.url_suffix)
}

/// The manifest archive's identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestRef {
    /// `manifest_<16 hex>_<32 hex>`.
    ///
    /// The 32-hex suffix is the archive's MD5. The 16-hex prefix is **opaque**:
    /// *measured*, it is not a hash of the blob, the decompressed blob, the
    /// checksum, the id, the URL, or `build_id + id`. Recorded, never asserted.
    pub id: String,
    /// `md5(decompressed archive)`. Verified in M3.
    pub checksum: String,
    /// Compressed size. *Measured* a string.
    #[serde(default)]
    pub compressed_size: Option<IntOrStr>,
    /// Uncompressed size. *Measured* a string.
    #[serde(default)]
    pub uncompressed_size: Option<IntOrStr>,
}

impl ManifestRef {
    /// The 32-hex checksum suffix of the id.
    pub fn id_suffix(&self) -> Option<&str> {
        self.id.rsplit_once('_').map(|(_, s)| s)
    }

    /// The 16-hex opaque prefix of the id.
    pub fn id_prefix(&self) -> Option<&str> {
        self.id.rsplit_once('_').map(|(p, _)| p)
    }
}

/// How a download family is addressed and stored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Download {
    /// `0` everywhere measured. Non-zero would mean the blobs are encrypted, and
    /// this tool does not implement that.
    #[serde(default)]
    pub encryption: i64,
    /// Empty everywhere measured.
    #[serde(default)]
    pub password: String,
    /// `1` = zstd. The only value this tool handles.
    #[serde(default)]
    pub compression: i64,
    /// The CDN base, including the `build_id` as its last segment.
    #[serde(default)]
    pub url_prefix: String,
    /// Appended after the id. Empty everywhere measured.
    #[serde(default)]
    pub url_suffix: String,
}

impl Download {
    /// Assert that the blobs can be stored the way this tool stores them.
    ///
    /// Compression `1` is zstd and encryption `0` is plaintext. Anything else is
    /// a *silent* failure waiting to happen — `zstd::decode` on an unencrypted
    /// raw blob produces garbage rather than an error in some cases — so it is
    /// checked here, once, before a single byte is fetched.
    pub fn require_zstd_plaintext(&self, what: &str) -> Result<(), ApiError> {
        if self.encryption != 0 {
            return Err(ApiError::shape(
                what,
                format!(
                    "encryption is {}, not 0; encrypted blobs are not supported",
                    self.encryption
                ),
            ));
        }
        if self.compression != 1 {
            return Err(ApiError::shape(
                what,
                format!(
                    "compression is {}, not 1; only zstd blobs can be decoded",
                    self.compression
                ),
            ));
        }
        Ok(())
    }
}

/// Totals an archive should reconcile against (assertion A5).
///
/// *Measured* as four JSON **strings**.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    /// Sum of chunk compressed sizes.
    #[serde(default)]
    pub compressed_size: Option<IntOrStr>,
    /// Sum of file sizes.
    #[serde(default)]
    pub uncompressed_size: Option<IntOrStr>,
    /// File entries in the archive.
    #[serde(default)]
    pub file_count: Option<IntOrStr>,
    /// Chunk entries across all files.
    #[serde(default)]
    pub chunk_count: Option<IntOrStr>,
}

impl Stats {
    /// `file_count` as a number, when present and well-formed.
    pub fn files(&self) -> Option<u64> {
        self.file_count.as_ref().and_then(IntOrStr::as_u64)
    }
    /// `chunk_count` as a number, when present and well-formed.
    pub fn chunks(&self) -> Option<u64> {
        self.chunk_count.as_ref().and_then(IntOrStr::as_u64)
    }
    /// `uncompressed_size` as a number, when present and well-formed.
    pub fn uncompressed_bytes(&self) -> Option<u64> {
        self.uncompressed_size.as_ref().and_then(IntOrStr::as_u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------- IntOrStr, the measured shape

    /// Every size in a `getBuild` body is a string. A `u64` field would parse
    /// this as null and quietly produce a manifest with no sizes.
    #[test]
    fn a_string_number_parses_and_keeps_its_wire_form() {
        let v: IntOrStr = serde_json::from_str(r#""9007187""#).unwrap();
        assert_eq!(v, IntOrStr::Str("9007187".into()));
        assert_eq!(v.as_u64(), Some(9_007_187));
        assert_eq!(serde_json::to_string(&v).unwrap(), r#""9007187""#);
    }

    #[test]
    fn an_integer_number_parses_and_keeps_its_wire_form() {
        let v: IntOrStr = serde_json::from_str("42").unwrap();
        assert_eq!(v, IntOrStr::Int(42));
        assert_eq!(v.as_u64(), Some(42));
        assert_eq!(serde_json::to_string(&v).unwrap(), "42");
    }

    #[test]
    fn neither_form_is_coerced_into_the_other_on_the_way_out() {
        // The whole reason the type exists: re-serialising must not change the
        // bytes, or a diffed capture stops matching a fresh request.
        assert_ne!(
            serde_json::to_string(&IntOrStr::Str("1".into())).unwrap(),
            serde_json::to_string(&IntOrStr::Int(1)).unwrap()
        );
    }

    #[test]
    fn a_negative_or_unparsable_value_has_no_unsigned_form() {
        assert_eq!(IntOrStr::Int(-1).as_u64(), None);
        assert_eq!(IntOrStr::Int(-1).as_i64(), Some(-1));
        assert_eq!(IntOrStr::Str("nope".into()).as_u64(), None);
        assert_eq!(IntOrStr::Str("".into()).as_u64(), None);
        assert_eq!(IntOrStr::Str(" 42 ".into()).as_u64(), Some(42));
    }

    #[test]
    fn display_is_the_original_text_either_way() {
        assert_eq!(IntOrStr::Str("007".into()).to_string(), "007");
        assert_eq!(IntOrStr::Int(7).to_string(), "7");
    }

    // --------------------------------------------------- the measured 7.1.0 body

    /// Verbatim from the live API, sizes and all.
    const BUILD_7_1_0: &str = r#"{"retcode":0,"message":"OK","data":{"build_id":"sMXGW2ll3Fuu","tag":"7.1.0","manifests":[{"category_id":"10016","category_name":"游戏本体","manifest":{"id":"manifest_671e1a92a6cf53ff_8d4dfb34d2ee2cf64aae45a9b1ecf58d","checksum":"8d4dfb34d2ee2cf64aae45a9b1ecf58d","compressed_size":"9007187","uncompressed_size":"16827424"},"chunk_download":{"encryption":0,"password":"","compression":1,"url_prefix":"https://autopatchhk.yuanshen.com/client_app/sophon/chunks/cxhpq4g4rgg0/sMXGW2ll3Fuu","url_suffix":""},"manifest_download":{"encryption":0,"password":"","compression":1,"url_prefix":"https://autopatchhk.yuanshen.com/client_app/sophon/manifests/cxhpq4g4rgg0/sMXGW2ll3Fuu","url_suffix":""},"matching_field":"game","stats":{"compressed_size":"128310073410","uncompressed_size":"131981846376","file_count":"2744","chunk_count":"113610"},"deduplicated_stats":{"compressed_size":"128174445744","uncompressed_size":"131776619976","file_count":"2744","chunk_count":"113441"}}]}}"#;

    fn build() -> GetBuild {
        let w: Wrapper<GetBuild> = serde_json::from_str(BUILD_7_1_0).expect("measured body parses");
        assert_eq!(w.retcode, 0);
        w.data
    }

    #[test]
    fn the_measured_build_parses_with_every_field_it_declares() {
        let b = build();
        assert_eq!(b.build_id, "sMXGW2ll3Fuu");
        assert_eq!(b.tag, "7.1.0");
        let m = b
            .manifest_for("game")
            .expect("the game manifest is present");
        assert_eq!(m.category_id, "10016");
        assert_eq!(m.category_name, "游戏本体", "non-ASCII survives");
        assert_eq!(
            m.manifest.id,
            "manifest_671e1a92a6cf53ff_8d4dfb34d2ee2cf64aae45a9b1ecf58d"
        );
        assert_eq!(m.stats.as_ref().unwrap().files(), Some(2744));
    }

    /// The id's two halves mean different things, and conflating them is the
    /// mistake the spec is at pains to prevent.
    #[test]
    fn the_manifest_id_splits_into_an_opaque_prefix_and_the_checksum_suffix() {
        let b = build();
        let m = b.manifest_for("game").unwrap();
        assert_eq!(m.manifest.id_prefix(), Some("manifest_671e1a92a6cf53ff"));
        assert_eq!(
            m.manifest.id_suffix(),
            Some("8d4dfb34d2ee2cf64aae45a9b1ecf58d")
        );
        assert_eq!(m.manifest.id_suffix(), Some(m.manifest.checksum.as_str()));
        assert_eq!(
            m.manifest.id_prefix().unwrap().len(),
            "manifest_".len() + 16
        );
    }

    #[test]
    fn urls_are_joined_prefix_id_suffix() {
        let b = build();
        let m = b.manifest_for("game").unwrap();
        assert_eq!(
            m.archive_url(),
            "https://autopatchhk.yuanshen.com/client_app/sophon/manifests/cxhpq4g4rgg0/sMXGW2ll3Fuu/manifest_671e1a92a6cf53ff_8d4dfb34d2ee2cf64aae45a9b1ecf58d"
        );
        assert_eq!(
            m.chunk_url("ddb31a9f"),
            "https://autopatchhk.yuanshen.com/client_app/sophon/chunks/cxhpq4g4rgg0/sMXGW2ll3Fuu/ddb31a9f"
        );
    }

    /// `url_suffix` is empty in every response seen, which is exactly why a join
    /// that ignored it would look correct right up until it stopped being.
    #[test]
    fn a_url_suffix_is_appended_rather_than_dropped() {
        let m = BuildManifest {
            category_id: "1".into(),
            category_name: String::new(),
            manifest: ManifestRef {
                id: "manifest_x_deadbeef".into(),
                checksum: "deadbeef".into(),
                compressed_size: None,
                uncompressed_size: None,
            },
            chunk_download: Download {
                encryption: 0,
                password: String::new(),
                compression: 1,
                url_prefix: "https://c/x".into(),
                url_suffix: ".zst".into(),
            },
            manifest_download: Download {
                encryption: 0,
                password: String::new(),
                compression: 1,
                url_prefix: "https://c/m".into(),
                url_suffix: ".zst".into(),
            },
            matching_field: "game".into(),
            stats: None,
            deduplicated_stats: None,
        };
        assert!(m.archive_url().ends_with("/manifest_x_deadbeef.zst"));
        assert!(m.chunk_url("abc").ends_with("/abc.zst"));
    }

    #[test]
    fn a_missing_matching_field_names_what_was_published() {
        let b = build();
        let e = b.manifest_for("ko-kr").unwrap_err();
        assert!(e.to_string().contains("ko-kr"), "{e}");
        assert!(e.to_string().contains("game"), "{e}");
        assert_eq!(b.manifests.len(), 1);
    }

    /// The field a minimal client drops, and the one that decides whether a blob
    /// can be decoded at all.
    #[test]
    fn storage_declined_is_refused_before_a_byte_is_fetched() {
        let b = build();
        let m = b.manifest_for("game").unwrap();
        m.chunk_download
            .require_zstd_plaintext("chunk_download")
            .unwrap();
        m.manifest_download
            .require_zstd_plaintext("manifest_download")
            .unwrap();

        let bad = Download {
            encryption: 1,
            password: String::new(),
            compression: 1,
            url_prefix: String::new(),
            url_suffix: String::new(),
        };
        assert!(
            bad.require_zstd_plaintext("chunk_download")
                .unwrap_err()
                .to_string()
                .contains("encryption")
        );
        let bad = Download {
            encryption: 0,
            password: String::new(),
            compression: 0,
            url_prefix: String::new(),
            url_suffix: String::new(),
        };
        assert!(
            bad.require_zstd_plaintext("chunk_download")
                .unwrap_err()
                .to_string()
                .contains("compression")
        );
    }

    #[test]
    fn deduplicated_stats_are_kept_and_readable() {
        let b = build();
        let m = b.manifest_for("game").unwrap();
        let d = m.deduplicated_stats.as_ref().expect("kept");
        assert_eq!(d.files(), Some(2744));
        assert_eq!(d.chunks(), Some(113_441));
        assert_ne!(d.chunks(), m.stats.as_ref().unwrap().chunks());
    }

    // ------------------------------------------------------------- configs

    /// Verbatim from the live API.
    const CONFIGS: &str = r#"{"retcode":0,"message":"OK","data":{"launch_configs":[{"game":{"id":"gopR6Cufr3","biz":"hk4e_global"},"exe_file_name":"GenshinImpact.exe","installation_dir":"Genshin Impact game","audio_pkg_scan_dir":"GenshinImpact_Data/Persistent/audio_lang_14","audio_pkg_res_dir":"GenshinImpact_Data/StreamingAssets/AudioAssets","default_download_mode":"DOWNLOAD_MODE_CHUNK","enable_audio_pkg_mgmt":false,"enable_ldiff":true}]}}"#;

    fn configs() -> GameConfigs {
        serde_json::from_str::<Wrapper<GameConfigs>>(CONFIGS)
            .expect("measured body parses")
            .data
    }

    #[test]
    fn the_fixture_paths_come_from_the_launcher_not_from_hardcoding() {
        let c = configs();
        assert_eq!(
            c.audio_pkg_scan_dir().unwrap(),
            "GenshinImpact_Data/Persistent/audio_lang_14"
        );
        assert_eq!(c.installation_dir().unwrap(), "Genshin Impact game");
        assert_eq!(c.exe_file_name().unwrap(), "GenshinImpact.exe");
    }

    /// The scan directory is a document-supplied path: the fixture joins it
    /// under `game/`, so an escaping value has to be refused where it is read.
    #[test]
    fn an_escaping_audio_scan_dir_is_refused() {
        for value in ["../../escape", "a\\b", "C:/Windows", ""] {
            let body = CONFIGS.replace("GenshinImpact_Data/Persistent/audio_lang_14", value);
            let c: GameConfigs = serde_json::from_str::<Wrapper<GameConfigs>>(&body)
                .unwrap()
                .data;
            let e = c.audio_pkg_scan_dir().unwrap_err();
            let text = e.to_string();
            assert!(
                text.contains("audio_pkg_scan_dir"),
                "{value:?}: the error names the field: {text}"
            );
        }
    }

    #[test]
    fn chunk_mode_is_required_and_a_different_mode_is_named() {
        assert_eq!(configs().require_chunk_mode().unwrap(), DOWNLOAD_MODE_CHUNK);

        let other = CONFIGS.replace("DOWNLOAD_MODE_CHUNK", "DOWNLOAD_MODE_LDIFF");
        let c: GameConfigs = serde_json::from_str::<Wrapper<GameConfigs>>(&other)
            .unwrap()
            .data;
        let e = c.require_chunk_mode().unwrap_err();
        assert!(e.to_string().contains("DOWNLOAD_MODE_LDIFF"), "{e}");
        assert!(e.to_string().contains("DOWNLOAD_MODE_CHUNK"), "{e}");
    }

    #[test]
    fn a_missing_required_config_field_is_named_rather_than_defaulted() {
        let bare = r#"{"retcode":0,"data":{"launch_configs":[{}]}}"#;
        let c: GameConfigs = serde_json::from_str::<Wrapper<GameConfigs>>(bare)
            .unwrap()
            .data;
        for f in [
            c.download_mode().unwrap_err().to_string(),
            c.audio_pkg_scan_dir().unwrap_err().to_string(),
            c.installation_dir().unwrap_err().to_string(),
        ] {
            assert!(f.contains("response does not carry this field"), "{f}");
        }
        let empty = r#"{"retcode":0,"data":{"launch_configs":[]}}"#;
        let c: GameConfigs = serde_json::from_str::<Wrapper<GameConfigs>>(empty)
            .unwrap()
            .data;
        assert!(c.launch().unwrap_err().to_string().contains("empty"));
    }

    /// A launcher adding a field must not stop a capture. It is captured in the
    /// raw body regardless, and refusing to parse would be refusing to record it.
    #[test]
    fn an_unknown_field_does_not_stop_a_parse() {
        let extra = CONFIGS.replace(
            "\"enable_ldiff\":true",
            "\"enable_ldiff\":true,\"brand_new_field\":42",
        );
        let c: GameConfigs = serde_json::from_str::<Wrapper<GameConfigs>>(&extra)
            .unwrap()
            .data;
        assert_eq!(c.require_chunk_mode().unwrap(), DOWNLOAD_MODE_CHUNK);
    }

    // ------------------------------------------------------------ branches

    /// Verbatim from the live API.
    const BRANCHES: &str = r#"{"retcode":0,"message":"OK","data":{"game_branches":[{"game":{"id":"gopR6Cufr3","biz":"hk4e_global"},"main":{"package_id":"ScSYQBFhu9","branch":"main","password":"bDL4JUHL625x","tag":"7.1.0","diff_tags":["7.0.0","6.7.0"],"categories":[{"category_id":"10016","matching_field":"game","type":"CATEGORY_TYPE_RESOURCE","scenarios":["CATEGORY_SCENARIO_FULL"]},{"category_id":"10018","matching_field":"en-us","type":"CATEGORY_TYPE_AUDIO","scenarios":["CATEGORY_SCENARIO_FULL"]}]},"pre_download":null}]}}"#;

    #[test]
    fn the_branch_carries_the_downloader_parameters_and_the_categories() {
        let b: GameBranches = serde_json::from_str::<Wrapper<GameBranches>>(BRANCHES)
            .unwrap()
            .data;
        let m = b.main().unwrap();
        assert_eq!(m.package_id, "ScSYQBFhu9");
        assert_eq!(m.password, "bDL4JUHL625x");
        assert_eq!(m.tag, "7.1.0");
        assert_eq!(m.diff_tags, vec!["7.0.0", "6.7.0"]);
        assert_eq!(m.categories.len(), 2);
        assert_eq!(m.categories[0].kind, "CATEGORY_TYPE_RESOURCE");
        assert_eq!(m.categories[1].kind, "CATEGORY_TYPE_AUDIO");
        assert_eq!(m.categories[1].matching_field, "en-us");
    }

    #[test]
    fn a_branch_with_no_main_entry_is_an_error() {
        let b = r#"{"retcode":0,"data":{"game_branches":[{"main":{"branch":"beta","package_id":"x","password":"y","tag":"z"}}]}}"#;
        let b: GameBranches = serde_json::from_str::<Wrapper<GameBranches>>(b)
            .unwrap()
            .data;
        assert!(b.main().unwrap_err().to_string().contains("main"));
    }

    // ---------------------------------------------------- deprecated + legacy

    #[test]
    fn deprecated_files_parse_to_manifest_relative_paths() {
        let body = r#"{"retcode":0,"data":{"deprecated_file_configs":[{"game":{"id":"g","biz":"b"},"deprecated_files":[{"name":"UnityPlayer.dll"},{"name":"GenshinImpact_Data/Plugins/hdiffz.dll"}]}]}}"#;
        let d: DeprecatedFiles = serde_json::from_str::<Wrapper<DeprecatedFiles>>(body)
            .unwrap()
            .data;
        let files = d.files().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].name, "GenshinImpact_Data/Plugins/hdiffz.dll");
    }

    /// The claim the legacy endpoint is captured to support: it is frozen, and
    /// the bundle should be able to *show* that rather than assert it.
    #[test]
    fn the_legacy_endpoint_reports_its_own_staleness() {
        let body = r#"{"retcode":0,"data":{"game_packages":[{"game":{"id":"g","biz":"b"},"main":{"major":{"version":"5.5.0","game_pkgs":[{"url":"u","md5":"m","size":"10737418240","decompressed_size":"21485322240"}],"audio_pkgs":[{"language":"en-us","url":"a","size":"18412580831"}]},"patches":[{"version":"5.4.0"},{"version":"5.3.0"}]}}]}}"#;
        let p: GamePackages = serde_json::from_str::<Wrapper<GamePackages>>(body)
            .unwrap()
            .data;
        assert_eq!(p.major_version().unwrap(), "5.5.0");
        assert_eq!(p.patch_versions(), vec!["5.4.0", "5.3.0"]);

        let (behind, patches) = p
            .staleness("7.1.0")
            .expect("a 7.x branch is ahead of 5.5.0");
        assert_eq!(behind, 2, "5.5.0 -> 7.1.0 is two major versions");
        assert_eq!(patches.len(), 2);
        // The url contains parentheses in the real body; they must not need
        // escaping to be held.
        assert_eq!(
            p.package().unwrap().main.as_ref().unwrap().major.audio_pkgs[0].language,
            "en-us"
        );
    }

    #[test]
    fn staleness_is_zero_when_the_branch_is_not_ahead() {
        let body = r#"{"retcode":0,"data":{"game_packages":[{"main":{"major":{"version":"7.1.0"}},"patches":[]}]}}"#;
        let p: GamePackages = serde_json::from_str::<Wrapper<GamePackages>>(body)
            .unwrap()
            .data;
        // Zero is an answer — "not stale" — and is reported as such. `None` is
        // reserved for "could not be computed", so the two are distinguishable.
        assert_eq!(p.staleness("7.1.0"), Some((0, Vec::<String>::new())));
        // A branch *behind* the legacy endpoint means the comparison itself is
        // wrong, and is also refused rather than reported as negative staleness.
        assert_eq!(p.staleness("6.7.0"), None);
    }
}
