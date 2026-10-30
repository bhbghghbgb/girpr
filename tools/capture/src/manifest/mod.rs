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
mod tests {
    use super::*;

    fn chunk(offset: i64, usize_: i64) -> SophonChunk {
        SophonChunk {
            id: format!("id{offset}"),
            uncompressed_md5: format!("{offset:032}"),
            offset,
            compressed_size: 100,
            uncompressed_size: usize_,
            unknown_field6: 7,
            compressed_md5: "c".into(),
        }
    }

    fn file(path: &str, chunks: Vec<SophonChunk>, size: i64) -> SophonChunkFile {
        SophonChunkFile {
            file: path.into(),
            chunks,
            is_folder: false,
            size,
            md5: "m".into(),
        }
    }

    /// A manifest is data, and every number in it came off the wire as an
    /// `i64`. `decode_counts` must therefore *measure* a hostile one rather than
    /// abort on it — it is the function that exists to tell a reader what a
    /// manifest says, and it cannot do that by dying.
    #[test]
    fn a_manifest_whose_arithmetic_overflows_is_measured_not_fatal() {
        // `offset = i64::MAX` then `offset + uncompressed_size` overflowed: an
        // abort in debug and test builds, so this line alone could kill
        // `cargo nextest run`, and a silent wrap in release — which would report
        // the file as tiled when nothing had been checked.
        let entries = vec![file("huge", vec![chunk(i64::MAX, 4096)], 4096)];
        let d = decode_counts(&entries);
        assert_eq!(
            d.structure.non_gapless_files, 1,
            "an unaddable offset is not a tiling"
        );
        assert_eq!(d.total_chunks, 1, "and the entry is still counted");

        // And the reverse: a huge declared size must not become 18.4 quintillion
        // bytes in the artifact. `as u64` alone turned -1 into 2^64-1.
        let negative = vec![file("neg", vec![], -1)];
        let d = decode_counts(&negative);
        assert_eq!(d.total_file_bytes, 0, "a file of -1 B contributes 0 B");

        // Same for the field the doc calls "what bounds the tool's whole-chunk
        // buffering" — a negative compressed size must not record as ~1.8e19.
        let mut c = chunk(0, 10);
        c.compressed_size = -1;
        let d = decode_counts(&[file("neg", vec![c], 10)]);
        assert_eq!(
            d.max_chunk_compressed_bytes, 0,
            "and -1 B is not the maximum"
        );

        // A positive measurement still works, which is the half of this that has
        // to keep working.
        let ok = decode_counts(&[file("ok", vec![chunk(0, 10)], 10)]);
        assert_eq!(ok.max_chunk_compressed_bytes, 100);
        assert_eq!(ok.total_file_bytes, 10);
    }

    /// A manifest nobody can build on is refused by name, with exit `3`.
    ///
    /// This is the check that has to run *before* a blob is fetched, and it was
    /// unreachable from a unit test until it moved out of the command
    /// dispatcher: a manifest that fails A3 has no good decode to compare
    /// against, no `chunks/`, and no bundle to capture against.
    #[test]
    fn a_manifest_that_cannot_be_built_on_is_refused_by_name() {
        let good = |decoded: Decoded, stats: Option<crate::hyp::Stats>| CapturedManifest {
            ..captured(green(), decoded, stats)
        };

        // 1. A3/A4 failed: named as the verification it is, before any
        //    reconciliation, and the message carries both booleans.
        let mut v = green();
        v.decompressed_md5_matches_checksum = false;
        let e = verify(&captured(v, Decoded::default(), None), "7.0.0", "game").unwrap_err();
        assert_eq!(e.code, crate::exit::ExitCode::AssertionFailed, "{e:?}");
        assert!(
            e.detail.starts_with("verify the 7.0.0 game manifest:"),
            "{e:?}"
        );
        assert!(
            e.detail
                .contains("decompressed md5 matches checksum: false"),
            "{e}"
        );

        // 2. A5 failed: every disagreeing stat named with both numbers.
        let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
        let wrong = Some(crate::hyp::Stats {
            compressed_size: Some("100".into()),
            uncompressed_size: Some("999".into()),
            file_count: Some("1".into()),
            chunk_count: Some("1".into()),
        });
        let e = verify(&good(d, wrong), "7.0.0", "game").unwrap_err();
        assert_eq!(e.code, crate::exit::ExitCode::AssertionFailed, "{e:?}");
        assert!(
            e.detail.starts_with("reconcile the 7.0.0 game manifest:"),
            "{e}"
        );
        assert!(
            e.detail
                .contains("uncompressed_size: declared Some(999), decoded 150"),
            "{e}"
        );
        assert!(
            !e.detail.contains("chunk_count"),
            "and only the one that disagrees: {e}"
        );

        // 3. Everything green passes, and a structural violation does not stop
        //    it — spec §3.6 says the capture still happens and M8 makes it a row.
        let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
        let ok = Some(crate::hyp::Stats {
            compressed_size: Some("100".into()),
            uncompressed_size: Some("150".into()),
            file_count: Some("1".into()),
            chunk_count: Some("1".into()),
        });
        assert!(verify(&good(d.clone(), ok.clone()), "7.0.0", "game").is_ok());

        // A gap between the chunks, which reconciles byte-for-byte anyway: the
        // chunks still total 140, so A5 is satisfied and only §3.6's structural
        // property is broken.
        let holed = decode_counts(&[file("a", vec![chunk(0, 50), chunk(60, 90)], 140)]);
        let holed_stats = Some(crate::hyp::Stats {
            compressed_size: Some("100".into()),
            uncompressed_size: Some("140".into()),
            file_count: Some("1".into()),
            chunk_count: Some("2".into()),
        });
        assert!(
            verify(&good(holed, holed_stats), "7.0.0", "game").is_ok(),
            "a structural violation is recorded, not fatal"
        );
    }

    #[test]
    fn a_clean_manifest_reports_every_structure_counter_zero() {
        let entries = vec![
            file("a", vec![chunk(0, 100), chunk(100, 50)], 150),
            file("b/c", vec![], 0),
        ];
        let d = decode_counts(&entries);
        assert_eq!(
            (d.entries, d.files, d.folders, d.total_chunks),
            (2, 2, 0, 2)
        );
        assert_eq!(d.total_file_bytes, 150);
        assert_eq!(d.max_chunk_compressed_bytes, 100);
        assert!(d.structure.clean(), "{:?}", d.structure);
        assert!(d.structure.violations().is_empty());
    }

    /// The verification a manifest that reconciles would carry.
    fn green() -> Verified {
        Verified {
            decompressed_md5_matches_checksum: true,
            checksum_matches_id_suffix: true,
            id_prefix_verified: false,
        }
    }

    /// A captured manifest, assembled field by field.
    ///
    /// Deliberately not a `Default`-derived fixture: three of the six structs on
    /// the way here have no honest empty state (`IdParts.prefix_semantics` and
    /// `Stored.container` are `&'static str` that carry real claims), and adding
    /// a `Default` to production types purely to shorten a test is the wrong
    /// trade. What `verify` reads is `meta.verified`, `meta.decoded`,
    /// `meta.declared` and `meta.decoded.structure`; the rest is spelled out so
    /// that stays visible.
    fn captured(
        verified: Verified,
        decoded: Decoded,
        stats: Option<crate::hyp::Stats>,
    ) -> CapturedManifest {
        CapturedManifest {
            tag: "7.0.0".into(),
            build_id: "3XnPviMfoILi".into(),
            matching_field: "game".into(),
            seq: 8,
            raw: Vec::new(),
            entries: Vec::new(),
            meta: ManifestMeta {
                schema_version: crate::SCHEMA_VERSION,
                tag: "7.0.0".into(),
                build_id: "3XnPviMfoILi".into(),
                matching_field: "game".into(),
                category_id: "10016".into(),
                category_name: "Genshin Impact".into(),
                manifest: DeclaredManifest {
                    id: "manifest_abc".into(),
                    checksum: "c".into(),
                    compressed_size: "100".into(),
                    uncompressed_size: "150".into(),
                },
                declared: Declared {
                    stats,
                    ..Declared::default()
                },
                id_parts: IdParts {
                    prefix_16hex: "abc".into(),
                    prefix_semantics: "opaque",
                    suffix_32hex: "c".into(),
                    suffix_equals_checksum: true,
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
                verified,
                decoded,
                full_decode_written: true,
            },
        }
    }

    /// Each input breaks exactly one structural counter.
    ///
    /// Spec §3.6: a violation must be *counted*, not fatal — the capture still
    /// works and the selfcheck says which property broke.
    ///
    /// Asserting *containment* rather than equality is deliberate, and is the honest
    /// form: the six properties are not independent. Walking a chunk list in the
    /// order it is stored, any out-of-order offsets are *also* non-gapless — there
    /// is no way to present them out of order and still cover `0..size` in one pass.
    /// A test demanding that one broken property be the only broken one would be
    /// demanding the counters be wrong.
    #[test]
    fn each_input_breaks_the_property_it_is_named_for() {
        let cases: Vec<(&str, SophonChunkFile, &str)> = vec![
            ("empty_paths", file("", vec![], 0), "empty_paths"),
            (
                "backslash_paths",
                file("a\\b", vec![], 0),
                "backslash_paths",
            ),
            (
                "non_ascii_paths",
                file("a/\u{4e2d}\u{6587}", vec![], 0),
                "non_ascii_paths",
            ),
            (
                "unsorted_chunk_offsets",
                file("x", vec![chunk(100, 50), chunk(0, 100)], 150),
                "unsorted_chunk_offsets",
            ),
            (
                "non_gapless_files",
                file("x", vec![chunk(0, 50), chunk(100, 50)], 150),
                "non_gapless_files",
            ),
            (
                "size_mismatch_files",
                file("x", vec![chunk(0, 50), chunk(50, 50)], 200),
                "size_mismatch_files",
            ),
        ];
        for (name, entry, prop) in cases {
            let d = decode_counts(&[entry]);
            assert!(!d.structure.clean(), "{name} should not be clean");
            assert!(
                d.structure.violations().contains(&prop),
                "{name}: expected {prop} among {:?}",
                d.structure.violations()
            );
        }
    }

    /// The properties that *can* be isolated: ascending-but-gapped, and
    /// gapless-but-wrong-length, are each a single violation.
    #[test]
    fn a_gap_and_a_length_disagreement_are_each_reported_alone() {
        // Offsets 0 and 60 with lengths 50 and 90 tile 0..150 in total but leave
        // a hole at 50..60, so `Σ == size` holds and only the gap is reported.
        let gap = decode_counts(&[file("x", vec![chunk(0, 50), chunk(60, 90)], 140)]);
        assert_eq!(gap.structure.violations(), vec!["non_gapless_files"]);
        assert_eq!(
            gap.structure.unsorted_chunk_offsets, 0,
            "offsets are still ascending"
        );
        assert_eq!(gap.structure.size_mismatch_files, 0);

        // Chunks covering 0..100 exactly, against a declared size of 200: the
        // coverage is fine and only the length disagrees.
        let len = decode_counts(&[file("x", vec![chunk(0, 50), chunk(50, 50)], 200)]);
        assert_eq!(len.structure.violations(), vec!["size_mismatch_files"]);
        assert_eq!(len.structure.non_gapless_files, 0);
    }

    /// An overlap is a gap in the wrong direction, and is counted as one.
    #[test]
    fn an_overlapping_chunk_is_non_gapless_too() {
        // 0..100 then 50..150: total 150 matches the declared size, offsets
        // ascend, and the two ranges overlap.
        let d = decode_counts(&[file("x", vec![chunk(0, 100), chunk(50, 50)], 150)]);
        assert_eq!(d.structure.violations(), vec!["non_gapless_files"]);
    }

    #[test]
    fn a_duplicate_path_is_counted_per_extra_occurrence() {
        let d = decode_counts(&[
            file("a", vec![], 0),
            file("a", vec![], 0),
            file("a", vec![], 0),
        ]);
        assert_eq!(d.structure.duplicate_paths, 2);
        assert_eq!(d.structure.violations(), vec!["duplicate_paths"]);
    }

    /// A folder carries no bytes, so reporting it as a gap would make a single
    /// folder entry look like six violations.
    #[test]
    fn a_folder_entry_is_not_counted_as_a_byte_level_violation() {
        let folder = SophonChunkFile {
            file: "GenshinImpact_Data/StreamingAssets".into(),
            chunks: vec![],
            is_folder: true,
            size: 0,
            md5: String::new(),
        };
        let d = decode_counts(&[folder]);
        assert_eq!((d.files, d.folders), (0, 1));
        assert!(d.structure.clean(), "{:?}", d.structure);
    }

    #[test]
    fn the_largest_chunk_is_recorded_across_every_entry() {
        let mut big = chunk(0, 10);
        big.compressed_size = 3_689_476;
        let d = decode_counts(&[file("a", vec![big, chunk(10, 10)], 20)]);
        // The measured ceiling for 7.0.0 and 7.1.0, spec §3.6.
        assert_eq!(d.max_chunk_compressed_bytes, 3_689_476);
    }

    // ------------------------------------------------------------ A5

    #[test]
    fn a_reconciling_manifest_reports_nothing() {
        let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
        let stats = crate::hyp::Stats {
            compressed_size: Some("100".into()),
            uncompressed_size: Some("150".into()),
            file_count: Some("1".into()),
            chunk_count: Some("1".into()),
        };
        assert!(reconcile(&d, Some(&stats)).is_empty());
        // No declared stats is not a violation; it is simply nothing to check.
        assert!(reconcile(&d, None).is_empty());
    }

    #[test]
    fn each_disagreeing_stat_is_named_with_both_numbers() {
        let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
        let stats = crate::hyp::Stats {
            compressed_size: Some("100".into()),
            uncompressed_size: Some("999".into()),
            file_count: Some("7".into()),
            chunk_count: Some("1".into()),
        };
        let bad = reconcile(&d, Some(&stats));
        let names: Vec<&str> = bad.iter().map(|(n, _, _)| *n).collect();
        assert_eq!(names, vec!["file_count", "uncompressed_size"]);
        assert_eq!(bad[0], ("file_count", Some(7), 1));
        assert_eq!(bad[1], ("uncompressed_size", Some(999), 150));
    }

    #[test]
    fn an_unparsable_declared_size_is_skipped_rather_than_called_a_violation() {
        let d = decode_counts(&[file("a", vec![chunk(0, 150)], 150)]);
        let stats = crate::hyp::Stats {
            compressed_size: None,
            uncompressed_size: Some("not a number".into()),
            file_count: Some("1".into()),
            chunk_count: None,
        };
        assert!(reconcile(&d, Some(&stats)).is_empty());
    }

    /// A manifest entry is data from the wire, and the guard runs before `--out`
    /// is created. `a/b.` and `a/b` are one file on Windows — and `a/./b` is a
    /// third spelling of it — so accepting the shapes would make the last write
    /// win wherever the tree is materialised. This pins that `manifest::capture`
    /// consults the shared rule, not only `--files`.
    ///
    /// **And that it collects every offending name before refusing.** Stopping at
    /// the first would make a second bad path discoverable only by editing the
    /// first out of a downloaded manifest.
    #[test]
    fn a_manifest_entry_windows_folds_onto_another_is_a_shape_error() {
        for (bad, rule) in [
            ("a/b.", "path.windows-fold"),
            ("a/b ", "path.windows-fold"),
            ("a/./b", "path.dot-component"),
        ] {
            let entries = vec![file(bad, vec![], 10)];
            let rejections = crate::rejections::Rejections::new();
            let err = validate_entry_paths("7.1.0/game", &entries, &rejections)
                .expect_err("a folded name must stop the run");
            let ApiError::Shape { subject, detail } = err else {
                panic!("{bad:?} must be a shape error");
            };
            assert_eq!(subject, "7.1.0/game");
            assert!(detail.contains("1 refused name"), "{bad:?}: {detail}");
            let rows = rejections.snapshot();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].value, bad);
            assert_eq!(rows[0].rule, rule);
            assert_eq!(rows[0].source, "manifest 7.1.0/game entry 0 file");
        }
        // An ordinary dotted name is still legal.
        assert!(
            validate_entry_paths(
                "u",
                &[file("a/b.c", vec![], 10)],
                &crate::rejections::Rejections::new()
            )
            .is_ok(),
            "a dot inside a name is not a folded trailing dot"
        );
    }

    /// One refusal does not hide the next: every offending entry and chunk id is
    /// named in the failure and recorded in the rejection document. A device
    /// name is **not** one of them — it is accepted and escaped on disk.
    #[test]
    fn every_hostile_name_in_one_manifest_is_reported_together() {
        let mut bad_chunk = chunk(0, 1);
        bad_chunk.id = "not-a-chunk-id".into();
        let entries = vec![
            file("a/b.", vec![], 1),
            file("CON", vec![], 1),
            file("ok/file", vec![bad_chunk], 1),
        ];
        let rejections = crate::rejections::Rejections::new();
        let err = validate_entry_paths("7.1.0/game", &entries, &rejections)
            .expect_err("two bad names must stop the run");
        let ApiError::Shape { detail, .. } = err else {
            panic!("a shape error");
        };
        assert!(detail.contains("2 refused names"), "{detail}");
        let rows = rejections.snapshot();
        let rules: Vec<&str> = rows.iter().map(|r| r.rule.as_str()).collect();
        assert_eq!(
            rules,
            vec!["path.windows-fold", "chunk-id.shape"],
            "one row per offending name, with its own rule"
        );
        let values: Vec<&str> = rows.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(values, vec!["a/b.", "not-a-chunk-id"]);
        assert!(
            !rows.iter().any(|r| r.value == "CON"),
            "a device name is kept, not refused; the writer escapes it"
        );
    }

    // ------------------------------------------------------------ A4

    #[test]
    fn the_id_parts_are_split_and_the_prefix_is_labelled_opaque() {
        let m = crate::hyp::ManifestRef {
            id: "manifest_671e1a92a6cf53ff_8d4dfb34d2ee2cf64aae45a9b1ecf58d".into(),
            checksum: "8d4dfb34d2ee2cf64aae45a9b1ecf58d".into(),
            compressed_size: Some("9007187".into()),
            uncompressed_size: Some("16827424".into()),
        };
        assert_eq!(m.id_prefix(), Some("manifest_671e1a92a6cf53ff"));
        assert_eq!(m.id_suffix(), Some(m.checksum.as_str()));
        let parts = IdParts {
            prefix_16hex: m.id_prefix().unwrap().to_string(),
            prefix_semantics: "opaque",
            suffix_32hex: m.id_suffix().unwrap().to_string(),
            suffix_equals_checksum: true,
        };
        // The field exists so the artifact *states* the finding rather than
        // implying the prefix went unchecked.
        assert_eq!(parts.prefix_semantics, "opaque");
        assert!(parts.suffix_equals_checksum);
        assert_eq!(parts.suffix_32hex.len(), 32);
    }

    #[test]
    fn a_verified_report_is_green_only_when_both_checks_hold() {
        let ok = Verified {
            decompressed_md5_matches_checksum: true,
            checksum_matches_id_suffix: true,
            id_prefix_verified: false,
        };
        assert!(ok.all_green());
        for bad in [
            Verified {
                decompressed_md5_matches_checksum: false,
                ..ok.clone()
            },
            Verified {
                checksum_matches_id_suffix: false,
                ..ok.clone()
            },
        ] {
            assert!(!bad.all_green());
        }
        // The prefix is never verified and that is not a failure.
        assert!(ok.all_green() && !ok.id_prefix_verified);
    }

    #[test]
    fn a_declared_size_is_carried_as_the_api_sent_it() {
        assert_eq!(render_size(Some(&"9007187".into())), "9007187");
        assert_eq!(render_size(Some(&crate::hyp::IntOrStr::Int(42))), "42");
        assert_eq!(render_size(None), "");
        assert_eq!(size_of(Some(&"9007187".into())), Some(9_007_187));
        assert_eq!(size_of(None), None);
    }
}
