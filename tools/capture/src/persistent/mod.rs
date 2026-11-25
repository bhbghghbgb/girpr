//! The optional `dispatch/` and `persistent/` records.
//!
//! These are a **conditional part of schema 4**: a bundle whose three record
//! rows are honestly `unavailable` has neither directory and still verifies,
//! while a bundle that carries them has every byte checked exactly like every
//! other bundle file (`MANIFEST.json → files[]` and `verify/checksums.json` are
//! generated from what the writers return), and a bundle that marks a record
//! class `captured` without them fails verification.
//!
//! The dispatcher envelope is a byte-exact response body; the record beside it
//! states the key fingerprint, seed and version it was queried with, and the
//! values the decoder derived from it. The three manifests and the full-file
//! blobs are raw bytes: no re-serialisation, because the bridge replays them.

pub mod capture;

use serde::{Deserialize, Serialize};

/// One mirrored manifest's descriptor, as the dispatcher published it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    /// `res`, `data` or `silence_data`.
    pub kind: String,
    /// The CDN file name, from the descriptor NDJSON.
    pub remote_name: String,
    /// The URL the descriptor resolves to, **as captured** (the replay server
    /// rewrites only its scheme and authority).
    pub url: String,
    pub md5: String,
    pub file_size: u64,
}

/// The decoded facts a replay needs, kept beside the ciphertext so a consumer
/// never has to guess a revision or a CDN path from a base64 string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decoded {
    pub game_version: String,
    pub channel_name: String,
    pub codename: String,
    pub res_revision: u64,
    pub data_revision: u64,
    pub silence_revision: u64,
    /// Derived from the second `base_revision` GET, not from the envelope.
    pub audio_revision: u64,
    pub res_hash: String,
    pub data_hash: String,
    pub silence_hash: String,
    /// Also derived from the second GET.
    pub audio_hash: String,
    pub repo_res_version_url: String,
    pub repo_design_data_url: String,
    /// The body of the second `base_revision` GET (`"<revision> <hash>"`).
    pub base_revision: String,
    /// The three manifests, in dispatcher descriptor order.
    pub descriptors: [Descriptor; 3],
}

/// `dispatch/record.json` — what was asked, with what key and seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchRecord {
    pub schema_version: u32,
    /// The game version used in the query (`meta.latest`).
    pub version: String,
    /// The full query string's version parameter, e.g. `OSRELWin9.9.0`.
    pub query_version: String,
    /// The full dispatcher URL that produced `envelope`, as queried.
    pub dispatch_url: String,
    /// SHA-256 of the **public** key's DER form, never a private key.
    pub key_fingerprint: String,
    /// Bundle-relative path of the private key material. A live key belongs
    /// only in a gitignored artifact directory; tests use a synthetic key.
    pub key_file: String,
    pub seed: String,
    /// Bundle-relative path of the byte-exact dispatcher envelope.
    pub envelope: String,
    pub envelope_md5: String,
    pub envelope_sha256: String,
    pub ciphertext_bytes: u64,
    pub decoded: Decoded,
}

/// One raw manifest byte body retained under `persistent/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub kind: String,
    pub path: String,
    pub bytes: u64,
    pub md5: String,
}

/// One full-file blob retained under `persistent/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobFile {
    /// The dispatcher's `remoteName`, which decides the install-side folder.
    pub remote_name: String,
    /// The mirrored manifest kind: `res`, `data` or `silence_data`.
    pub source_kind: String,
    /// The capture id that admitted this blob (`persistent_res+1` for extras).
    pub capture_id: String,
    pub path: String,
    pub bytes: u64,
    pub md5: String,
}

/// `persistent/index.json` — the inventory of the raw byte bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentIndex {
    pub schema_version: u32,
    pub manifests: [ManifestFile; 3],
    pub blobs: Vec<BlobFile>,
}

/// Digest a body once, into the pair every record uses.
pub fn digest(bytes: &[u8]) -> (u64, String, String) {
    (
        bytes.len() as u64,
        crate::hash::md5_hex(bytes),
        crate::hash::sha256_hex(bytes),
    )
}
