//! Manifests: fetch → zstd → verify → decode → dump → diff.
//!
//! **M3** brings [`proto`] (the wire types, opaque field 6 preserved verbatim),
//! [`dump`] (the full decode plus its `paths.tsv` sidecar) and this facade, which
//! is the fetch-and-verify pipeline. **M4** adds [`diff`]: the path diff, the
//! reuse-pool measurement, and the reconciliation against `getBuild.stats`.
//!
//! Verification is not optional here. A blob that decodes to something is not
//! evidence that it decodes to the *right* something: the decompressed MD5 is
//! checked against `checksum`, and `checksum` against the manifest id's 32-hex
//! suffix. The id's 16-hex prefix is recorded as `opaque` and never checked —
//! it is not a hash of anything observable (spec §3.7), and the field saying so
//! exists so a later reader does not repeat the mistake.
//!
//! ## Structure violations are recorded, not fatal
//!
//! Spec §3.6 lists six structural properties that hold for the live manifests
//! today. A future version violating one must still produce a capture, with
//! `selfcheck.json` naming what broke — so nothing here *rejects* a non-gapless
//! or out-of-order chunk list. The counters are measured and stored, and M8
//! decides what they mean.

use serde::Serialize;

use crate::failure::{Failure, assertion};
use crate::hash;
use crate::hash::Encoding;
use crate::http::{ApiError, Fetched, Fetcher, Kind, RequestLog};
use crate::hyp::GetBuild;

pub mod diff;
pub mod dump;
pub mod fetch;
pub mod proto;

pub use dump::StoredAs;
pub use proto::{SophonChunk, SophonChunkFile, SophonChunkManifest};

/// What the manifest id's two halves mean.
///
/// Split out because they mean *different* things, and conflating them is the
/// mistake spec §3.7 exists to prevent.
#[derive(Debug, Clone, Serialize)]
pub struct IdParts {
    /// The leading half, e.g. `manifest_671e1a92a6cf53ff`.
    pub prefix_16hex: String,
    /// Always `"opaque"`.
    ///
    /// *Measured* 2026-10-14: the 16-hex prefix is not xxh64 of the compressed
    /// blob, the decompressed blob, the checksum, the id, the URL, or
    /// `build_id + id`. Collapse's cache validator assumes it is an xxh64, so
    /// for Genshin that validation can never pass. It is recorded, never checked.
    pub prefix_semantics: &'static str,
    /// The trailing 32 hex, which *is* the archive's MD5.
    pub suffix_32hex: String,
    /// Whether the suffix equals `manifest.checksum` (assertion A4).
    pub suffix_equals_checksum: bool,
}

/// Where the archive was stored and what it hashed to.
#[derive(Debug, Clone, Serialize)]
pub struct Stored {
    /// Artifact-relative path.
    pub path: String,
    /// Bytes on disk.
    pub bytes: u64,
    /// `"zstd"` — what the bytes are, not what they were compressed from.
    pub container: &'static str,
    /// MD5 of the blob as served, which is the CDN's `ETag` and `Content-MD5`.
    pub md5_compressed: String,
    /// SHA-256 of the blob as served.
    pub sha256_compressed: String,
    /// MD5 of the decompressed archive, which is `manifest.checksum`.
    pub md5_decompressed: String,
    /// Decompressed length.
    pub bytes_decompressed: u64,
    /// How the archive itself is stored. Always `identity` — the bytes are kept
    /// exactly as served so a reader can hash them and compare with the CDN.
    pub encoding: Encoding,
}

/// The three checks this milestone asserts.
#[derive(Debug, Clone, Serialize)]
pub struct Verified {
    /// **A3**: `md5(decompressed) == manifest.checksum`.
    pub decompressed_md5_matches_checksum: bool,
    /// **A4**: `manifest.checksum == id 32-hex suffix`.
    pub checksum_matches_id_suffix: bool,
    /// Always `false`, and expected to be. Present so the artifact *states* the
    /// finding rather than implying the prefix went unchecked.
    pub id_prefix_verified: bool,
}

impl Verified {
    /// Whether every check this milestone asserts held.
    pub fn all_green(&self) -> bool {
        self.decompressed_md5_matches_checksum && self.checksum_matches_id_suffix
    }
}

/// Counts of the six structural properties in spec §3.6.
///
/// All zero on the live manifests as measured. Non-zero is not fatal here; M8
/// turns a non-zero into a failed selfcheck row, which is the right place for
/// the judgement because by then the whole bundle exists.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Structure {
    /// Paths appearing more than once.
    pub duplicate_paths: u64,
    /// Entries with an empty path.
    pub empty_paths: u64,
    /// Paths containing a backslash.
    pub backslash_paths: u64,
    /// Paths containing a non-ASCII byte.
    pub non_ascii_paths: u64,
    /// Files whose chunk offsets are not strictly ascending.
    pub unsorted_chunk_offsets: u64,
    /// Files whose chunks do not cover `0..size` without a gap or overlap.
    pub non_gapless_files: u64,
    /// Files where `Σ chunk.uncompressed_size != size`.
    pub size_mismatch_files: u64,
}

impl Structure {
    /// Whether every property holds.
    pub fn clean(&self) -> bool {
        self.duplicate_paths == 0
            && self.empty_paths == 0
            && self.backslash_paths == 0
            && self.non_ascii_paths == 0
            && self.unsorted_chunk_offsets == 0
            && self.non_gapless_files == 0
            && self.size_mismatch_files == 0
    }

    /// The names of the properties that do not hold, for a one-line message.
    pub fn violations(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.duplicate_paths > 0 {
            v.push("duplicate_paths");
        }
        if self.empty_paths > 0 {
            v.push("empty_paths");
        }
        if self.backslash_paths > 0 {
            v.push("backslash_paths");
        }
        if self.non_ascii_paths > 0 {
            v.push("non_ascii_paths");
        }
        if self.unsorted_chunk_offsets > 0 {
            v.push("unsorted_chunk_offsets");
        }
        if self.non_gapless_files > 0 {
            v.push("non_gapless_files");
        }
        if self.size_mismatch_files > 0 {
            v.push("size_mismatch_files");
        }
        v
    }
}

/// Totals of the decoded manifest.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Decoded {
    /// Entries of any kind.
    pub entries: u64,
    /// Entries that are files.
    pub files: u64,
    /// Entries that are folders.
    pub folders: u64,
    /// Chunks across all entries.
    pub total_chunks: u64,
    /// The largest single chunk's compressed size.
    ///
    /// This is what bounds the main tool's whole-chunk buffering, so it is
    /// recorded rather than derived later: a reader can see the ceiling without
    /// re-decoding 16 MB of protobuf.
    pub max_chunk_compressed_bytes: u64,
    /// `Σ size` over file entries.
    pub total_file_bytes: u64,
    /// The six structural counters.
    pub structure: Structure,
}

/// `manifests/<tag>/<field>.manifest.meta.json` — everything known about one blob.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestMeta {
    /// The bundle schema version.
    pub schema_version: u32,
    /// The version this manifest belongs to.
    pub tag: String,
    /// The build it was published under.
    pub build_id: String,
    /// `game`, or an audio code.
    pub matching_field: String,
    /// `10016` for `game`.
    pub category_id: String,
    /// The launcher's own name for the category, non-ASCII and preserved.
    pub category_name: String,
    /// Identity and declared sizes, as the API sent them.
    pub manifest: DeclaredManifest,
    /// What the API declared, including the fields a minimal client drops.
    pub declared: Declared,
    /// The id's two halves.
    pub id_parts: IdParts,
    /// Where the bytes went and what they hashed to.
    pub stored: Stored,
    /// The three checks.
    pub verified: Verified,
    /// Totals of the decode.
    pub decoded: Decoded,
    /// Whether the full JSON decode was written beside the archive.
    ///
    /// Recorded rather than inferred: under `--manifest-detail paths-only` the
    /// archive is still on disk and still decodable, so a reader cannot tell
    /// from the tree alone that the convenience copy is missing. It is also not
    /// a loss — the protobuf round-trips from the archive — so it must not read
    /// as damage.
    pub full_decode_written: bool,
}

/// The `manifest` sub-object of the API's entry, carried verbatim.
#[derive(Debug, Clone, Serialize)]
pub struct DeclaredManifest {
    /// The archive id.
    pub id: String,
    /// The archive's decompressed MD5.
    pub checksum: String,
    /// Declared compressed size. A JSON *string* upstream.
    pub compressed_size: String,
    /// Declared uncompressed size. A JSON *string* upstream.
    pub uncompressed_size: String,
}

/// The `stats` / download declarations, carried so a reader can reconcile.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Declared {
    /// `stats`, as strings.
    pub stats: Option<crate::hyp::Stats>,
    /// `deduplicated_stats`, as strings.
    pub deduplicated_stats: Option<crate::hyp::Stats>,
    /// The chunk download declaration.
    pub chunk_download: crate::hyp::types::Download,
    /// The manifest download declaration.
    pub manifest_download: crate::hyp::types::Download,
}

/// One captured manifest: the bytes, the decode, and the verification.
#[derive(Debug, Clone)]
pub struct CapturedManifest {
    /// The tag it belongs to.
    pub tag: String,
    /// The build it was published under.
    pub build_id: String,
    /// `game`, or an audio code.
    pub matching_field: String,
    /// The request's sequence number.
    pub seq: u64,
    /// The archive exactly as served.
    pub raw: Vec<u8>,
    /// The decoded entries.
    pub entries: Vec<SophonChunkFile>,
    /// The verification and structure report.
    pub meta: ManifestMeta,
}

impl CapturedManifest {
    /// The decoded entries.
    pub fn entries(&self) -> &[SophonChunkFile] {
        &self.entries
    }

    /// A manifest with no bytes behind it — for a caller that already holds the
    /// decoded entries.
    ///
    /// Exists for the offline test corpus in `tests/`, where assembling a full
    /// [`ManifestMeta`] would be 200 lines of boilerplate around a pure
    /// function. **Not** for the capture path: nothing this tool fetches skips
    /// the archive, and a `CapturedManifest` that claims otherwise would be a
    /// lie a later stage could act on.
    pub fn empty(tag: &str, matching_field: &str, entries: Vec<SophonChunkFile>) -> Self {
        CapturedManifest {
            tag: tag.to_string(),
            build_id: String::new(),
            matching_field: matching_field.to_string(),
            seq: 0,
            raw: Vec::new(),
            meta: ManifestMeta {
                schema_version: crate::SCHEMA_VERSION,
                tag: tag.to_string(),
                build_id: String::new(),
                matching_field: matching_field.to_string(),
                category_id: String::new(),
                category_name: String::new(),
                manifest: DeclaredManifest {
                    id: String::new(),
                    checksum: String::new(),
                    compressed_size: String::new(),
                    uncompressed_size: String::new(),
                },
                declared: Declared::default(),
                id_parts: IdParts {
                    prefix_16hex: String::new(),
                    prefix_semantics: "opaque",
                    suffix_32hex: String::new(),
                    suffix_equals_checksum: false,
                },
                stored: Stored {
                    path: String::new(),
                    bytes: 0,
                    container: "zstd",
                    md5_compressed: String::new(),
                    sha256_compressed: String::new(),
                    md5_decompressed: String::new(),
                    bytes_decompressed: 0,
                    encoding: Encoding::Identity,
                },
                verified: Verified {
                    decompressed_md5_matches_checksum: false,
                    checksum_matches_id_suffix: false,
                    id_prefix_verified: false,
                },
                decoded: Decoded::default(),
                full_decode_written: false,
            },
            entries,
        }
    }

    /// Recompute the decode counters over the current entries.
    pub fn recompute_decoded(&mut self) {
        self.meta.decoded = decode_counts(&self.entries);
    }

    /// The archive URL it came from.
    pub fn url(&self) -> String {
        self.meta.declared.manifest_download.url_prefix.to_owned()
            + "/"
            + &self.meta.manifest.id
            + &self.meta.declared.manifest_download.url_suffix
    }

    /// The base its **chunk** blobs are served from.
    ///
    /// Separate from [`CapturedManifest::url`] because the two point at
    /// different CDN trees — `.../manifests/<build_id>` and
    /// `.../chunks/<build_id>` — and a capture that built chunk URLs from the
    /// archive URL would 404 on every one of them.
    pub fn chunk_url_prefix(&self) -> &str {
        &self.meta.declared.chunk_download.url_prefix
    }

    /// The URL of one of its chunks.
    pub fn chunk_url(&self, chunk_id: &str) -> String {
        self.meta.declared.chunk_download.url_prefix.clone()
            + "/"
            + chunk_id
            + &self.meta.declared.chunk_download.url_suffix
    }
}

/// Fetch one manifest, verify it, and decode it.
///
/// The order is load-bearing: the archive is hashed **as served** before it is
/// decompressed, because a blob that is not the one the CDN claims would
/// otherwise be decompressed and diffed before anyone noticed, and a manifest
/// that decodes cleanly is exactly the shape of thing that can be subtly wrong.
pub async fn capture(
    fetcher: &Fetcher,
    build: &GetBuild,
    tag: &str,
    matching_field: &str,
    max_decompressed_bytes: u64,
    rejections: &crate::rejections::Rejections,
) -> Result<CapturedManifest, ApiError> {
    // `matching_field` becomes `manifests/<tag>/<field>.manifest…` and travels
    // into the meta document, so it has to be one file-name component. The flag
    // validates its own values; this is the check every other caller (and a
    // future one) passes through.
    if let Some(problem) = crate::paths::field_component_problem(matching_field) {
        return Err(ApiError::shape(
            "matching_field",
            format!("manifest field {problem}"),
        ));
    }
    let declared = build.manifest_for(matching_field)?;
    // Storage is asserted before a byte is fetched, not after: `zstd::decode` on
    // an unencrypted raw blob does not reliably fail, it returns garbage.
    declared
        .manifest_download
        .require_zstd_plaintext(&format!("{tag} {matching_field} manifest_download"))?;

    let url = declared.archive_url();
    let fetched = fetch_archive(fetcher, &url).await?;

    if !hash::is_zstd(&fetched.body) {
        return Err(ApiError::shape(
            &url,
            format!(
                "{} bytes do not begin with the zstd magic {:?}, though the API declared \
                 compression=1",
                fetched.body.len(),
                &fetched.body[..fetched.body.len().min(4)],
            ),
        ));
    }
    // The declared sizes, as strings, beside what was actually fetched.
    let declared_compressed = size_of(declared.manifest.compressed_size.as_ref());
    let declared_uncompressed = size_of(declared.manifest.uncompressed_size.as_ref());
    // **The declared size bounds the decode, not just the later comparison.**
    // `zstd::decode_all` expanded the whole frame before this check, so a small
    // compressed bomb could abort the process (O4). The limit is the declaration
    // plus one byte (enough to detect an over-long frame) and is itself capped
    // by the run's `--max-decompressed-bytes`; an absent declaration gets that
    // ceiling. The effective value is recorded in `MANIFEST.json`.
    let decompressed = hash::zstd_decompress(
        &fetched.body,
        &url,
        hash::decompression_limit(declared_uncompressed, max_decompressed_bytes),
    )
    .map_err(|e| ApiError::shape(&url, e))?;
    if let Some(want) = declared_compressed
        && want != fetched.body.len() as u64
    {
        // Not fatal on its own: the CDN could have re-compressed. It is recorded
        // and asserted because a mismatch means one of the two numbers in the
        // bundle is wrong, and a reader cannot tell which.
        return Err(ApiError::shape(
            &url,
            format!(
                "declared compressed_size is {want} but {n} bytes arrived",
                n = fetched.body.len()
            ),
        ));
    }
    if let Some(want) = declared_uncompressed
        && want != decompressed.len() as u64
    {
        return Err(ApiError::shape(
            &url,
            format!(
                "declared uncompressed_size is {want} but the archive decompresses to {n} bytes",
                n = decompressed.len()
            ),
        ));
    }

    let md5_decompressed = hash::md5_hex(&decompressed);
    let checksum = &declared.manifest.checksum;
    let suffix = declared.manifest.id_suffix().unwrap_or("").to_string();
    let verified = Verified {
        decompressed_md5_matches_checksum: md5_decompressed.eq_ignore_ascii_case(checksum),
        checksum_matches_id_suffix: !suffix.is_empty() && suffix.eq_ignore_ascii_case(checksum),
        id_prefix_verified: false,
    };

    let entries = {
        use prost::Message as _;
        let mut m = proto::SophonChunkManifest::decode(decompressed.as_slice())
            .map_err(|e| ApiError::shape(&url, format!("protobuf decode: {e}")))?;
        // A digest is an identity, not a spelling, and `girpr` compares them
        // case-insensitively. Normalising here — the one place a manifest
        // enters the crate — makes every later comparison agree with the tool
        // by construction. See `proto::SophonChunk::normalise`.
        m.normalise();
        // **The one boundary every manifest path crosses.** `chuncks[].file`
        // becomes `<artifact>/game/<file>` and `expected/<file>`, and
        // `chunks[].id` becomes both a CDN URL segment and
        // `chunks/blobs/<id>.zst`. A decode that admitted `..\..\x` would let a
        // hostile or compromised CDN write outside `--out`; no later check
        // exists on either sink, so refuse the document here, before the tree is
        // created or a chunk is fetched.
        validate_entry_paths(&format!("{tag}/{matching_field}"), &m.chuncks, rejections)?;
        // The same boundary, for the numbers: `offset`, `compressed_size` and
        // `uncompressed_size` are raw `int64`s on the wire. A negative cost or
        // an offset that cannot be added is refused here, before a selection or
        // an assembly can carry it into the artifact.
        for entry in &m.chuncks {
            entry
                .validate_arithmetic()
                .map_err(|why| ApiError::shape(&url, format!("{}: {why}", entry.file)))?;
        }
        m.chuncks
    };
    let decoded = decode_counts(&entries);
    // One pass over the archive for both digests it records.
    let stored_digests = hash::md5_sha256(&fetched.body);

    let meta = ManifestMeta {
        schema_version: crate::SCHEMA_VERSION,
        tag: tag.to_string(),
        build_id: build.build_id.clone(),
        matching_field: matching_field.to_string(),
        category_id: declared.category_id.clone(),
        category_name: declared.category_name.clone(),
        manifest: DeclaredManifest {
            id: declared.manifest.id.clone(),
            checksum: checksum.clone(),
            compressed_size: render_size(declared.manifest.compressed_size.as_ref()),
            uncompressed_size: render_size(declared.manifest.uncompressed_size.as_ref()),
        },
        declared: Declared {
            stats: declared.stats.clone(),
            deduplicated_stats: declared.deduplicated_stats.clone(),
            chunk_download: declared.chunk_download.clone(),
            manifest_download: declared.manifest_download.clone(),
        },
        id_parts: IdParts {
            prefix_16hex: declared.manifest.id_prefix().unwrap_or("").to_string(),
            prefix_semantics: "opaque",
            suffix_32hex: suffix,
            suffix_equals_checksum: verified.checksum_matches_id_suffix,
        },
        stored: Stored {
            path: String::new(),
            bytes: fetched.body.len() as u64,
            container: "zstd",
            md5_compressed: stored_digests.md5,
            sha256_compressed: stored_digests.sha256,
            md5_decompressed: md5_decompressed.clone(),
            bytes_decompressed: decompressed.len() as u64,
            encoding: Encoding::Identity,
        },
        verified: verified.clone(),
        decoded,
        full_decode_written: true,
    };

    tracing::info!(
        tag,
        matching_field,
        entries = meta.decoded.entries,
        chunks = meta.decoded.total_chunks,
        compressed_bytes = meta.stored.bytes,
        decompressed_bytes = meta.stored.bytes_decompressed,
        max_chunk = meta.decoded.max_chunk_compressed_bytes,
        "manifest decoded"
    );

    Ok(CapturedManifest {
        tag: tag.to_string(),
        build_id: build.build_id.clone(),
        matching_field: matching_field.to_string(),
        seq: fetched.seq,
        raw: fetched.body,
        entries,
        meta,
    })
}

/// Refuse a decoded manifest entry whose path or chunk id cannot be joined onto
/// the bundle without leaving it.
///
/// **This is not one of §3.6's structural counters.** A non-gapless chunk list
/// is a fact worth recording and still captureable; a path that escapes `--out`
/// is an arbitrary write, so it is a shape error and stops the run before the
/// artifact directory exists. The rule and its rationale live in
/// [`crate::paths`], shared with `--files`, the bundle reader and the launcher's
/// scan directory.
fn validate_entry_paths(
    context: &str,
    entries: &[SophonChunkFile],
    rejections: &crate::rejections::Rejections,
) -> Result<(), ApiError> {
    // **Every offending name, not the first.** H3: a manifest with two
    // Windows-folded paths says more about the API than one with a single path,
    // and stopping at the first would make the second discoverable only by
    // editing the first out. Each one is recorded in the rejection document as
    // it is found; the message lists them so an operator does not have to open
    // the bundle to learn what was wrong.
    let mut problems: Vec<String> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if let Some((rule, problem)) = crate::paths::logical_path_rule(&entry.file) {
            let row = rejections.record(
                format!("manifest {context} entry {i} file"),
                &entry.file,
                rule,
            );
            problems.push(format!(
                "entry {i} {:?}: {}",
                row.value,
                brief(&problem, 240)
            ));
        }
        for (j, chunk) in entry.chunks.iter().enumerate() {
            if let Some((rule, problem)) = crate::paths::chunk_id_rule(&chunk.id) {
                rejections.record(
                    format!("manifest {context} entry {i} chunk {j} id"),
                    &chunk.id,
                    rule,
                );
                problems.push(format!(
                    "entry {i} {:?} chunk {j}: {}",
                    brief(&entry.file, 120),
                    brief(&problem, 240)
                ));
            }
        }
    }
    if problems.is_empty() {
        return Ok(());
    }
    // The message is for an operator, not an archive: name enough to act on and
    // count the rest. The full list, up to the document's own cap, is in the
    // rejection document.
    const SHOWN: usize = 8;
    let mut message = problems
        .iter()
        .take(SHOWN)
        .map(|p| brief(p, 240))
        .collect::<Vec<_>>()
        .join("; ");
    if problems.len() > SHOWN {
        message.push_str(&format!("; and {} more", problems.len() - SHOWN));
    }
    Err(ApiError::shape(
        context,
        format!(
            "{} refused name{}: {message}",
            problems.len(),
            if problems.len() == 1 { "" } else { "s" }
        ),
    ))
}

/// A bounded rendering of an untrusted string for a one-line failure.
fn brief(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max).collect();
    out.push('…');
    out
}

async fn fetch_archive(fetcher: &Fetcher, url: &str) -> Result<Fetched, ApiError> {
    fetcher.get(url, Kind::Manifest, None).await
}

/// The declared size of an archive, read from the API's string-or-int form.
fn size_of(v: Option<&crate::hyp::IntOrStr>) -> Option<u64> {
    v.and_then(|n| n.as_u64())
}

/// The declared size, rendered as it arrived.
///
/// Carried as a string because that is what the API sent, and a bundle that
/// quietly normalised `"9007187"` to `9007187` would no longer match the body it
/// was captured from.
fn render_size(v: Option<&crate::hyp::IntOrStr>) -> String {
    v.map(|n| n.to_string()).unwrap_or_default()
}

/// Count the decode: totals, the largest chunk, and the six structural counters.
///
/// **One pass, and an O(1) duplicate test.** Every number this returns is
/// accumulated in the loop below. Re-walking the same entries five more
/// times for `entries`/`files`/`folders`/`total_chunks`/`total_file_bytes`,
/// and calling `chunk_bytes()` for a sixth — a sum the chunk loop had already
/// computed as its `cursor` — would repeat work already done. And the
/// duplicate check is a `HashSet`, not `Vec::contains`, which is 2 744² / 2 ≈
/// 3.8 M string comparisons for the `game` field alone, on a value this
/// module's own `diff` says must never be quadratic.
pub fn decode_counts(entries: &[SophonChunkFile]) -> Decoded {
    let mut s = Structure::default();
    let mut seen: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(entries.len());
    let mut max_chunk = 0_u64;
    let mut files = 0_u64;
    let mut folders = 0_u64;
    let mut total_chunks = 0_u64;
    let mut total_file_bytes = 0_u64;

    for e in entries {
        if e.file.is_empty() {
            s.empty_paths += 1;
        }
        if e.file.contains('\\') {
            s.backslash_paths += 1;
        }
        if !e.file.is_ascii() {
            s.non_ascii_paths += 1;
        }
        if !seen.insert(e.file.as_str()) {
            s.duplicate_paths += 1;
        }

        if e.is_folder {
            folders += 1;
        } else {
            files += 1;
            // `.max(0) as u64`, which is the convention `diff.rs` already uses
            // for every byte it sums. A negative `size` is a manifest claiming a
            // file of -1 bytes, and `as u64` alone turns that into 18.4
            // quintillion — which then lands in `total_file_bytes` in the
            // artifact and in `reconcile` as a mismatch against the chunk sum.
            total_file_bytes = total_file_bytes.saturating_add(e.size.max(0) as u64);
        }
        total_chunks += e.chunks.len() as u64;

        let mut ascending = true;
        let mut gapless = true;
        let mut cursor = 0_i64;
        // `Σ uncompressed_size`, which is what `chunk_bytes()` recomputed on a
        // second walk. For a gapless file it is also `cursor`, so the two agree;
        // for a gapful one only this sum is meaningful, which is why it is
        // accumulated rather than read off the cursor.
        let mut chunk_bytes = 0_i64;
        for (i, c) in e.chunks.iter().enumerate() {
            // Same convention as `total_file_bytes` above, and the same reason:
            // this is the number recorded as `max_chunk_compressed_bytes`, the
            // field documented as what bounds the tool's whole-chunk buffering.
            let compressed = c.compressed_size.max(0) as u64;
            if compressed > max_chunk {
                max_chunk = compressed;
            }
            if i > 0 && c.offset <= e.chunks[i - 1].offset {
                ascending = false;
            }
            if c.offset != cursor {
                gapless = false;
            }
            // `checked_add`, because this counter is a *measurement* and must not
            // panic on the thing it measures. A manifest declaring
            // `offset = i64::MAX` overflowed here: an abort in debug and test
            // builds — so `cargo nextest run` itself would die — and a silent wrap
            // in release, which reports a file as tiled when nothing was checked.
            // An offset that cannot be added to is a file larger than any file,
            // so the tiling is broken by definition.
            cursor = match c.offset.checked_add(c.uncompressed_size) {
                Some(next) => next,
                None => {
                    gapless = false;
                    i64::MAX
                }
            };
            chunk_bytes = chunk_bytes.saturating_add(c.uncompressed_size);
        }
        // A folder carries no bytes, so none of the byte-level properties apply
        // to it; counting one would report a violation on every folder entry and
        // mean nothing.
        //
        // `gapless` deliberately asks only whether the chunks *tile* — each one
        // starting where the last ended. Whether they then reach `size` is
        // `size_mismatch_files`, and folding it in here would double-count every
        // disagreement and blur two properties into one counter. It is in fact
        // redundant: with no gap, the cursor equals `Σ uncompressed_size`, so
        // the extra clause could only ever fire when the length already did.
        if e.is_folder || e.chunks.is_empty() {
            continue;
        }
        if !ascending {
            s.unsorted_chunk_offsets += 1;
        }
        if !gapless {
            s.non_gapless_files += 1;
        }
        if chunk_bytes != e.size {
            s.size_mismatch_files += 1;
        }
    }

    Decoded {
        entries: entries.len() as u64,
        files,
        folders,
        total_chunks,
        max_chunk_compressed_bytes: max_chunk,
        total_file_bytes,
        structure: s,
    }
}

/// Reconcile the decode against `getBuild.stats` (assertion A5).
///
/// Returns `(field, declared, decoded)` for each of the three that disagree.
/// A violation is reported, not raised: the decode is still true, the declared
/// number is still true, and M8 decides which one is the interesting fact.
pub fn reconcile(
    decoded: &Decoded,
    stats: Option<&crate::hyp::Stats>,
) -> Vec<(&'static str, Option<u64>, u64)> {
    let mut out = Vec::new();
    let mut check = |name, declared: Option<u64>, actual: u64| {
        if let Some(d) = declared
            && d != actual
        {
            out.push((name, Some(d), actual));
        }
    };
    check("file_count", stats.and_then(|s| s.files()), decoded.files);
    check(
        "chunk_count",
        stats.and_then(|s| s.chunks()),
        decoded.total_chunks,
    );
    check(
        "uncompressed_size",
        stats.and_then(|s| s.uncompressed_bytes()),
        decoded.total_file_bytes,
    );
    out
}

/// The `matching_field` every run scores first, and the one every later stage
/// needs whatever class a capture lands in.
///
/// `game` is the one mandatory field: `--audio` widens the *selection pool* but
/// never makes the game field optional. Naming it here rather than inlining it
/// at each call site gives the rule "game is never optional" one address, and
/// `build_manifest` — which stamps the name into `MANIFEST.json -> field` — needs
/// the same value the capture selected on.
///
/// It lived in `run.rs`, where `build_manifest` could reach it because it was
/// there too.
pub const MANIFEST_FIELD: &str = "game";

/// The two things a decoded manifest must satisfy before any byte of it is
/// trusted: its decompressed MD5 matches the declared checksum, and the decoded
/// counts reconcile against the declared stats.
///
/// Both are exit `3`. The bytes arrived intact or they did not, and either way
/// nothing here is the network's fault — a checksum that fails is a fact about
/// what was published, and the capture cannot be built on top of it.
///
/// **Lived in `run.rs`, next to nothing that could be tested without a capture.**
/// It is [`reconcile`] plus the verification that has to precede it, and it was
/// unable to move until `Failure` left the command dispatcher.
pub fn verify(m: &CapturedManifest, tag: &str, field: &str) -> Result<(), Failure> {
    if !m.meta.verified.all_green() {
        return Err(assertion(
            &format!("verify the {tag} {field} manifest"),
            &ApiError::shape(
                tag,
                format!(
                    "decompressed md5 matches checksum: {}, checksum matches id suffix: {}",
                    m.meta.verified.decompressed_md5_matches_checksum,
                    m.meta.verified.checksum_matches_id_suffix
                ),
            ),
        ));
    }
    let v = reconcile(&m.meta.decoded, m.meta.declared.stats.as_ref());
    if !v.is_empty() {
        return Err(assertion(
            &format!("reconcile the {tag} {field} manifest"),
            &ApiError::shape(
                tag,
                v.iter()
                    .map(|(n, d, a)| format!("{n}: declared {d:?}, decoded {a}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        ));
    }
    dump::note_reconciliation(m);
    if !m.meta.decoded.structure.clean() {
        // Recorded, not fatal: spec §3.6 says a violation must still produce
        // a capture with the selfcheck naming what broke. M8 makes it a row.
        tracing::warn!(
            tag,
            field,
            violations = ?m.meta.decoded.structure.violations(),
            "manifest violates a structural invariant; recorded, not fatal"
        );
    }
    Ok(())
}

/// Mark the bodies in a set as stored, by request sequence number.
pub fn mark_stored(log: &RequestLog, m: &mut CapturedManifest, path: &str) {
    let rel = path.to_string();
    m.meta.stored.path = rel.clone();
    log.set_stored_as(m.seq, rel);
}

#[cfg(test)]
mod tests;
