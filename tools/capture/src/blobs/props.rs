//! Blob properties: container sniff, digests, and the recorded A6 results.
//!
//! **M6.** Both digests are kept because both are *asserted* — the compressed
//! MD5 is what the CDN's `ETag` claims, the decompressed one is what the
//! manifest claims, and a blob that passes only one of them is evidence of
//! something.
//!
//! ## Why measurement never fails
//!
//! A6 is four checks: the raw length, the compressed MD5, the decompressed MD5
//! and the decoded length. [`BlobProps::measure`] returns all four as data and
//! returns `Ok` even when they disagree, because a capture that hit a
//! truncated CDN response is still worth keeping — the selfcheck is where that
//! gets said out loud, one row per chunk instance, and dropping the blob would
//! destroy the evidence. What *is* an error is a blob that is not zstd at all:
//! there is nothing to measure, and guessing would be worse.
//!
//! The decoded bytes are produced and dropped rather than cached. The store
//! measures here and assembles in M7, and holding a second copy of every chunk
//! between those two steps would be tens of megabytes of peak RSS to avoid a
//! second decompress.

use serde::{Deserialize, Serialize};

use crate::hash;
use crate::http::ApiError;
use crate::manifest::proto::SophonChunk;

/// The four A6 checks, kept apart so a failing capture can name which one
/// broke rather than only that something did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verified {
    /// `len(raw) == compressed_size` — the blob arrived whole.
    pub raw_size_matches: bool,
    /// `md5(raw) == compressed_md5` — it arrived *intact*, not merely complete.
    pub md5_compressed_matches: bool,
    /// `md5(zstd(raw)) == uncompressed_md5` — it is the chunk the manifest named.
    pub md5_decompressed_matches: bool,
    /// `len(decoded) == uncompressed_size`.
    pub decoded_size_matches: bool,
}

impl Verified {
    /// Whether all four hold.
    pub fn all_green(&self) -> bool {
        self.raw_size_matches
            && self.md5_compressed_matches
            && self.md5_decompressed_matches
            && self.decoded_size_matches
    }

    /// The individual A6 checks that failed, in the spec's order.
    ///
    /// Empty when green. This is what turns "selfcheck failed" into "the decoded
    /// MD5 of this blob does not match what the manifest said", which is the
    /// difference between a bisectable report and a shrug.
    pub fn failures(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.raw_size_matches {
            out.push("len(raw blob) == compressed_size");
        }
        if !self.md5_compressed_matches {
            out.push("md5(raw blob) == compressed_md5");
        }
        if !self.md5_decompressed_matches {
            out.push("md5(zstd(raw blob)) == uncompressed_md5");
        }
        if !self.decoded_size_matches {
            out.push("len(decoded) == uncompressed_size");
        }
        out
    }
}

impl std::fmt::Display for Verified {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.failures().as_slice() {
            [] => f.write_str("all four A6 checks hold"),
            [only] => write!(f, "{only} does not hold"),
            many => write!(
                f,
                "{} of the four A6 checks do not hold: {}",
                many.len(),
                many.join(", ")
            ),
        }
    }
}

/// One stored blob and everything known about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobProps {
    /// The content address: the manifest's `chunk.id`.
    pub chunk_id: String,
    /// Bundle-relative path, or `null` when `--no-chunk-blobs` skipped it.
    pub stored_path: Option<String>,
    /// `len(raw)`, which is what the CDN sent and what the file holds.
    pub bytes: u64,
    /// The sniff result. `"zstd"` or `"identity"`, never a guess.
    ///
    /// An owned `String` rather than a `&'static str`: M9's `verify` reads this
    /// field back out of `chunks/index.json`, and a value that arrived from a
    /// document is not `'static`.
    pub container: String,
    /// MD5 of the bytes as served.
    pub md5_compressed: String,
    /// MD5 of the bytes after zstd.
    pub md5_decompressed: String,
    /// SHA-256 of the bytes as served, so a reader can check with a stock tool.
    pub sha256_compressed: String,
    /// `len(decoded)`.
    pub uncompressed_size: u64,
    /// The manifest's opaque field 6, carried through a diff that never reads it.
    pub unknown_field6: i64,
    /// The four A6 checks, or `null` when the blob was not measured.
    pub verified: Option<Verified>,
    /// Why the blob would not decompress, when it would not.
    ///
    /// Separate from `verified` because it is a *different kind* of finding:
    /// the four checks are "is this the chunk the manifest named", while this is
    /// "the bytes are not a complete frame". A truncated download fails two of
    /// the four as a side effect, and without this field a reader would go
    /// looking for a wrong-content problem they do not have.
    pub decode_error: Option<String>,
}

impl BlobProps {
    /// The artifact-relative path a blob of this id is stored at.
    ///
    /// One function, because the writer and the index must agree on it and a
    /// reader has to be able to predict it from the id alone.
    pub fn stored_path_for(chunk_id: &str) -> String {
        format!("chunks/blobs/{chunk_id}.zst")
    }

    /// Measure a fetched blob against the manifest's chunk.
    ///
    /// Never fails. A blob that arrived truncated, or with the wrong content,
    /// or not at all, is exactly the thing this capture exists to record, and a
    /// function that returned `Err` would be a caller with a choice between
    /// dropping the evidence or ignoring the error — and both lose it.
    ///
    /// The one thing that *is* an error is a blob with no zstd frame at all:
    /// the build declared `compression: 1`, so unframed bytes are a protocol
    /// violation rather than a digest mismatch, and there is nothing here to
    /// measure.
    pub fn measure(raw: &[u8], chunk: &SophonChunk) -> Result<BlobProps, ApiError> {
        let what = format!("chunk {}", chunk.id);
        if !hash::is_zstd(raw) {
            return Err(ApiError::shape(
                "chunk",
                format!(
                    "{what}: not a zstd frame ({} bytes, magic {:02x?}); the build declared \
                     compression=1",
                    raw.len(),
                    &raw[..raw.len().min(4)],
                ),
            ));
        }
        let md5_compressed = hash::md5_hex(raw);
        // A frame that will not decode leaves the two decode-dependent checks
        // false and records why, rather than taking the whole blob down with it.
        let decoded = hash::zstd_decompress(raw, &what);
        let md5_decompressed = decoded.as_ref().map(|d| hash::md5_hex(d)).ok();
        Ok(BlobProps {
            chunk_id: chunk.id.clone(),
            stored_path: None,
            bytes: raw.len() as u64,
            container: "zstd".to_string(),
            sha256_compressed: hash::sha256_hex(raw),
            md5_compressed: md5_compressed.clone(),
            md5_decompressed: md5_decompressed.clone().unwrap_or_default(),
            uncompressed_size: decoded.as_ref().map(|d| d.len() as u64).unwrap_or(0),
            unknown_field6: chunk.unknown_field6,
            decode_error: decoded.as_ref().err().cloned(),
            verified: Some(Verified {
                raw_size_matches: raw.len() as i64 == chunk.compressed_size,
                md5_compressed_matches: md5_compressed == chunk.compressed_md5,
                md5_decompressed_matches: md5_decompressed.as_deref()
                    == Some(&chunk.uncompressed_md5),
                decoded_size_matches: matches!(decoded.as_ref(), Ok(d) if d.len() as i64 == chunk.uncompressed_size),
            }),
        })
    }

    /// The entry for a blob that was never fetched.
    ///
    /// `--no-chunk-blobs` produces these. The declared sizes come from the
    /// manifest and the digests are **empty strings rather than zeros**,
    /// because a zeroed digest is a claim and an absent one is not. M9's
    /// `verify` reports `verified: null` as "not measured", which is the
    /// honest reading and is why the spec says that flag breaks A7–A10 by
    /// design.
    pub fn unmeasured(chunk: &SophonChunk) -> BlobProps {
        BlobProps {
            chunk_id: chunk.id.clone(),
            stored_path: None,
            bytes: 0,
            container: "zstd".to_string(),
            md5_compressed: String::new(),
            md5_decompressed: chunk.uncompressed_md5.clone(),
            sha256_compressed: String::new(),
            uncompressed_size: chunk.uncompressed_size.max(0) as u64,
            unknown_field6: chunk.unknown_field6,
            verified: None,
            decode_error: None,
        }
    }

    /// The A6 assertions this blob contributes, named for the selfcheck.
    ///
    /// One entry per failing check rather than one "A6 failed" row, because a
    /// selfcheck with 71 rows for 71 chunks has to be readable per chunk and a
    /// blob failing three of four is three findings.
    pub fn failures(&self) -> Vec<&'static str> {
        self.verified.map(|v| v.failures()).unwrap_or_default()
    }

    /// Whether this blob can be assembled from.
    ///
    /// `verified: None` is *not* green, and neither is a blob that would not
    /// decode. A blob nobody measured is not a blob anybody should build a
    /// fixture file out of, and neither is one whose bytes are not a frame.
    pub fn is_usable(&self) -> bool {
        self.verified.is_some_and(|v| v.all_green())
            && self.decode_error.is_none()
            && self.stored_path.is_some()
    }

    /// Why this blob is unusable, in one line, for the run log.
    pub fn why_unusable(&self) -> Option<String> {
        if self.is_usable() {
            return None;
        }
        Some(match (&self.decode_error, self.verified) {
            (Some(e), _) => format!("{}: {e}", self.chunk_id),
            (None, None) => format!("{}: never measured", self.chunk_id),
            (None, Some(v)) => format!("{}: {v}", self.chunk_id),
        })
    }

    /// The path, or a message naming the chunk.
    pub fn require_path(&self) -> Result<&str, String> {
        self.stored_path
            .as_deref()
            .ok_or_else(|| format!("chunk {} has no stored blob", self.chunk_id))
    }

    /// The recorded digest of the decoded bytes.
    pub fn decoded_md5(&self) -> Result<&str, String> {
        if self.verified.is_none() {
            return Err(format!("chunk {} was never measured", self.chunk_id));
        }
        Ok(&self.md5_decompressed)
    }

    /// Attach the path it was written to.
    pub fn set_stored(&mut self, path: impl Into<String>) {
        self.stored_path = Some(path.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk and the exact bytes that should satisfy it.
    ///
    /// Built by compressing real content rather than by hand-writing digests,
    /// so a test cannot pass because both sides of the assertion were written
    /// from the same wrong constant.
    fn chunk_with(payload: &[u8]) -> (SophonChunk, Vec<u8>) {
        let raw = zstd::encode_all(payload, 3).expect("encode");
        let c = SophonChunk {
            id: format!(
                "{:016x}_{}",
                0xabcd_ef01_2345_6789u64,
                hash::md5_hex(payload)
            ),
            uncompressed_md5: hash::md5_hex(payload),
            offset: 0,
            compressed_size: raw.len() as i64,
            uncompressed_size: payload.len() as i64,
            unknown_field6: -4_117_343_013_703_778_304,
            compressed_md5: hash::md5_hex(&raw),
        };
        (c, raw)
    }

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_blob_that_agrees_with_its_chunk_is_green_on_all_four() {
        let (c, raw) = chunk_with(&payload(4096));
        let p = BlobProps::measure(&raw, &c).expect("a zstd frame");
        let v = p.verified.expect("measured");
        assert!(v.all_green(), "{}", v);
        assert_eq!(v.failures(), Vec::<&str>::new());
        assert_eq!(v.to_string(), "all four A6 checks hold");
        assert_eq!(p.bytes, raw.len() as u64);
        assert_eq!(p.uncompressed_size, 4096);
        assert_eq!(p.container, "zstd");
        assert_eq!(p.md5_decompressed, c.uncompressed_md5);
        assert_eq!(
            p.unknown_field6, c.unknown_field6,
            "carried through verbatim"
        );
        assert_eq!(p.chunk_id, c.id);
    }

    /// A truncated download is *recorded*, not thrown away — it is the whole
    /// reason `measure` is infallible — and the reason it failed is recorded as
    /// "not a complete frame" rather than left to be inferred from two false
    /// digest checks.
    #[test]
    fn a_truncated_blob_is_recorded_with_the_decode_failure_attached() {
        let (c, raw) = chunk_with(&payload(4096));
        let cut = &raw[..raw.len() - 8];
        let p = BlobProps::measure(cut, &c).expect("it still has the zstd magic");
        let v = p.verified.expect("measured");
        assert!(!v.all_green());
        assert!(
            !v.raw_size_matches,
            "the manifest said {}",
            c.compressed_size
        );
        assert!(!v.md5_compressed_matches, "different bytes");
        assert!(!v.md5_decompressed_matches);
        assert!(!v.decoded_size_matches);
        let why = p.decode_error.as_deref().expect("and it says why");
        assert!(why.contains("incomplete frame"), "{why}");
        assert!(why.contains(&c.id), "names the chunk: {why}");
        assert!(!p.is_usable());
        assert!(p.why_unusable().unwrap().contains("incomplete frame"));
    }

    #[test]
    fn a_green_blob_has_no_decode_error_and_says_so() {
        let (c, raw) = chunk_with(&payload(4096));
        let mut p = BlobProps::measure(&raw, &c).unwrap();
        assert!(p.decode_error.is_none());
        // Measured green but not yet written is still not assemblable, and
        // `why_unusable` says which of the two it is.
        assert!(p.verified.unwrap().all_green());
        assert!(!p.is_usable());
        p.set_stored(BlobProps::stored_path_for(&c.id));
        assert!(p.is_usable());
        assert!(p.why_unusable().is_none());
    }

    /// Each of the four checks fails on its own when exactly its own input is
    /// wrong. A test that only ever produced a "several things are wrong" blob
    /// would pass with any two of the four swapped.
    #[test]
    fn each_a6_check_can_fail_alone() {
        let (c, raw) = chunk_with(&payload(4096));

        // 1. raw length: a different declared size, same bytes.
        let mut wrong_len = c.clone();
        wrong_len.compressed_size += 1;
        let v = BlobProps::measure(&raw, &wrong_len)
            .unwrap()
            .verified
            .unwrap();
        assert_eq!(v.failures(), vec!["len(raw blob) == compressed_size"]);

        // 2. compressed md5.
        let mut wrong_c = c.clone();
        wrong_c.compressed_md5 = "0".repeat(32);
        let v = BlobProps::measure(&raw, &wrong_c)
            .unwrap()
            .verified
            .unwrap();
        assert_eq!(v.failures(), vec!["md5(raw blob) == compressed_md5"]);

        // 3. decompressed md5, via the manifest's `uncompressed_md5`.
        let mut wrong_u = c.clone();
        wrong_u.uncompressed_md5 = "0".repeat(32);
        let v = BlobProps::measure(&raw, &wrong_u)
            .unwrap()
            .verified
            .unwrap();
        assert_eq!(
            v.failures(),
            vec!["md5(zstd(raw blob)) == uncompressed_md5"]
        );

        // 4. decoded length.
        let mut wrong_d = c.clone();
        wrong_d.uncompressed_size += 1;
        let v = BlobProps::measure(&raw, &wrong_d)
            .unwrap()
            .verified
            .unwrap();
        assert_eq!(v.failures(), vec!["len(decoded) == uncompressed_size"]);
    }

    #[test]
    fn several_failures_are_all_named_and_the_message_reads_as_a_sentence() {
        let mut c = chunk_with(&payload(64)).0;
        c.uncompressed_md5 = "0".repeat(32);
        c.compressed_md5 = "0".repeat(32);
        c.compressed_size += 1;
        c.uncompressed_size += 1;
        let (_, raw) = chunk_with(&payload(64));
        let v = BlobProps::measure(&raw, &c).unwrap().verified.unwrap();
        assert_eq!(v.failures().len(), 4);
        let s = v.to_string();
        assert!(s.starts_with("4 of the four"), "{s}");
        assert!(
            s.contains("compressed_md5") && s.contains("uncompressed_size"),
            "{s}"
        );
    }

    /// An unframed blob is a protocol violation, not a digest mismatch, and the
    /// message has to say so — a reader seeing four `false` would go looking for
    /// a corrupted download rather than for the wrong `compression` setting.
    #[test]
    fn bytes_that_are_not_zstd_are_an_error_not_a_measurement() {
        let (c, _) = chunk_with(&payload(64));
        let e = BlobProps::measure(b"plain bytes, sorry", &c).unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("not a zstd frame"), "{msg}");
        assert!(msg.contains("compression=1"), "{msg}");
        assert!(msg.contains(&c.id), "names the chunk: {msg}");
    }

    /// An unmeasured blob is not a green blob. `--no-chunk-blobs` produces a
    /// bundle that cannot be assembled from, and that has to be visible rather
    /// than defaulting to "no failures found".
    #[test]
    fn an_unmeasured_blob_is_not_usable_and_says_so() {
        let (c, _) = chunk_with(&payload(64));
        let p = BlobProps::unmeasured(&c);
        assert!(p.verified.is_none());
        assert!(
            !p.is_usable(),
            "nothing was measured, so nothing can be assembled"
        );
        assert!(p.failures().is_empty(), "no failures, but also no evidence");
        assert!(p.why_unusable().unwrap().contains("never measured"));
        assert!(p.require_path().unwrap_err().contains(&c.id));
        assert!(p.decoded_md5().unwrap_err().contains("never measured"));
        // The manifest's identity survives; the digests do not pretend to.
        assert_eq!(p.md5_decompressed, c.uncompressed_md5);
        assert_eq!(p.md5_compressed, "");
        assert_eq!(
            p.uncompressed_size, 64,
            "declared, so a reader knows the gap"
        );
    }

    #[test]
    fn the_stored_path_is_predictable_from_the_chunk_id_alone() {
        assert_eq!(
            BlobProps::stored_path_for("f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b"),
            "chunks/blobs/f5dc3ca1f3a0ba3d_291a7719a4a23664d37825a41fad3c5b.zst"
        );
        // The *file name* part is flat: an id is hex plus one underscore, so a
        // chunk id can never introduce a path separator or a `..`.
        let id = "616e6fbbf0a97dc9_a21a4420161bca81a78f73b27bfa359a";
        let name = BlobProps::stored_path_for(id)
            .rsplit('/')
            .next()
            .expect("a file name")
            .to_string();
        assert_eq!(name, format!("{id}.zst"));
        assert!(
            !name.contains('/') && !name.contains('\\') && !name.contains(".."),
            "a chunk id can never escape the bundle: {name}"
        );
    }

    #[test]
    fn the_serialised_shape_is_the_schema_doc_s_field_names() {
        let (c, raw) = chunk_with(&payload(128));
        let mut p = BlobProps::measure(&raw, &c).expect("a frame");
        p.set_stored(BlobProps::stored_path_for(&c.id));
        let v = serde_json::to_value(&p).expect("serialise");
        let mut keys: Vec<&str> = v
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "bytes",
                "chunk_id",
                "container",
                "decode_error",
                "md5_compressed",
                "md5_decompressed",
                "sha256_compressed",
                "stored_path",
                "uncompressed_size",
                "unknown_field6",
                "verified",
            ],
            "docs/01-artifact-schema.md §12, plus the one field the schema does \
             not carry because a truncated frame needs somewhere to say so"
        );
        assert_eq!(
            v["verified"]
                .as_object()
                .expect("verified")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>(),
            vec![
                "decoded_size_matches",
                "md5_compressed_matches",
                "md5_decompressed_matches",
                "raw_size_matches"
            ],
            "four checks, named as A6 names them"
        );
        // And `seq` is *not* here: provenance lives in api/requests.ndjson, so
        // the index cannot drift out of step with it.
        assert!(v.get("seq").is_none());
        assert!(
            v["decode_error"].is_null(),
            "a green blob serialises it away"
        );
    }
}
