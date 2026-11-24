//! The value types of a persistent plan. No behaviour beyond the spellings
//! the other modules need; `docs/09` §4.1 pins the shapes.

use girpr_dispatch::ManifestKind;

/// Which base tree a row lands in.
///
/// `Persistent` means either a genuine hotfix (the base manifest has the same
/// rel with a different MD5) or a row explicitly forced there; `StreamingAssets`
/// is the default for new paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BaseTree {
    StreamingAssets,
    Persistent,
}

impl BaseTree {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StreamingAssets => "StreamingAssets",
            Self::Persistent => "Persistent",
        }
    }
}

/// The suffix-derived folder under the base tree. `Root` spells as `""` and
/// adds no path segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Folder {
    AudioAssets,
    AssetBundles,
    VideoAssets,
    Root,
}

impl Folder {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AudioAssets => "AudioAssets",
            Self::AssetBundles => "AssetBundles",
            Self::VideoAssets => "VideoAssets",
            Self::Root => "",
        }
    }
}

/// One recognised NDJSON line of a persistent manifest.
///
/// `isPatch`, `category` and `localName` are deliberately absent. The server
/// does send them, but Collapse discards them and so do we (docs/08 §4.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentRow {
    pub remote_name: String,
    pub md5: String,
    /// XXH64 from the manifest's `hash` hex field, when present.
    pub hash: Option<u64>,
    pub file_size: u64,
}

impl PersistentRow {
    /// The digest a download must satisfy: XXH64 wins when present, MD5 is
    /// the fallback. Kept here as a convenience over the pure rule in
    /// [`crate::digest`].
    pub fn expected_digest(&self) -> crate::digest::ExpectedDigest {
        crate::digest::expected_digest(self)
    }
}

/// One file the execute half may download. All fields are decided here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentAsset {
    /// Cleaned, `/`-separated rel, rooted at the game directory.
    pub rel: String,
    pub placement: BaseTree,
    pub size: u64,
    pub md5: String,
    pub hash: Option<u64>,
    pub primary_url: String,
    pub alternate_url: String,
    pub source: ManifestKind,
}

/// A marker file the post phase publishes: plain text, **no trailing newline**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerSet {
    pub path: String,
    pub value: String,
}

/// The complete, read-only result of the plan half.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistentPlan {
    /// Sorted and deduped by cleaned rel (the first row for a rel wins).
    pub assets: Vec<PersistentAsset>,
    /// Raw, already-verified manifest bytes paired with their mirror rel.
    /// Sorted by rel. The plan half never writes them.
    pub mirrors: Vec<(String, Vec<u8>)>,
    /// Cleaned rels of every marker file the post phase writes.
    pub marker_paths: Vec<String>,
    /// Assets + mirrors + marker paths, for the purge keep-set. Sorted, deduped.
    pub expected_rels: Vec<String>,
    /// Sum of the deduplicated asset sizes. Mirrors and markers are not counted:
    /// they are fetched/verified separately from the download ceilings.
    pub total_bytes: u64,
}

impl PersistentPlan {
    /// Cleaned rels currently expected to exist under the game directory.
    pub fn rels(&self) -> impl Iterator<Item = &str> {
        self.expected_rels.iter().map(String::as_str)
    }
}
