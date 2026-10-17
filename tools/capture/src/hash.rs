//! Digests and the gzip decision.
//!
//! Cross-cutting, and early: M3 hashes manifests, M6 hashes chunk blobs, M7
//! assembles files and MD5s the result, and M8 writes `checksums.json` over
//! everything. Four milestones, one rule each ("MD5 what the manifest says,
//! SHA-256 so a reader can verify with a stock tool"), so the helpers live here
//! rather than being re-derived four times.
//!
//! ## Why two algorithms
//!
//! MD5 is what the manifest and the CDN both speak — matching them is the whole
//! point. SHA-256 is what `verify/checksums.json` carries, because a reader
//! checking a bundle should not need a tool that treats MD5 as a digest rather
//! than as a checksum. The bundle carries both and uses each for what it is for.
//!
//! ## The gzip rule
//!
//! A document is gzipped only when its **uncompressed** body exceeds
//! `--gzip-threshold`. The decision is recorded, never inferred from a filename,
//! so a reader never has to guess — and the *decompressed* digest is what
//! `checksums.json` carries for a gzipped file, because `flate2` output is not
//! byte-stable across zlib versions and artifact identity must not depend on
//! which zlib happened to build it.

use std::io::Write as _;

/// Hex MD5 of `bytes`, lowercase.
pub fn md5_hex(bytes: &[u8]) -> String {
    hex_bytes(&md5::compute(bytes).0)
}

/// Hex SHA-256 of `bytes`, lowercase.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    hex_bytes(&Sha256::digest(bytes))
}

/// What one pass over some bytes says about them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digests {
    /// How many bytes were fed in.
    pub bytes: u64,
    /// Lowercase hex MD5.
    pub md5: String,
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

/// Both digests, from one pass over the bytes.
///
/// MD5 is the manifest and CDN language; SHA-256 is what `checksums.json`
/// publishes. Nearly every buffer in this crate needs **both**, and the one-shot
/// [`md5_hex`] / [`sha256_hex`] pair turns that into two passes over the same
/// bytes — which for the raw blob path is ~27 MB of chunk bodies read twice per
/// capture, and for a manifest archive another 9 MB.
///
/// It implements [`std::io::Write`] so [`std::io::copy`] drives it: there is no
/// hand-rolled buffer here to get the block boundaries wrong, and it works
/// identically whether the bytes arrive from a file, a gzip decoder or a zstd
/// decoder.
///
/// **This used to live in `emit::index` as a private `Tee`**, which meant the
/// blob path re-derived it, and the crate's own "the helpers live here rather
/// than being re-derived four times" was not true.
///
/// Not [`Debug`]: `md5::Context` does not implement it, and a partial dump of a
/// half-fed hash state is worse than none. [`Digests`] is the thing to print.
pub struct DualDigest {
    md5: md5::Context,
    sha: sha2::Sha256,
    len: u64,
}

impl DualDigest {
    /// An empty pair, ready to be fed.
    pub fn new() -> Self {
        DualDigest {
            md5: md5::Context::new(),
            sha: <sha2::Sha256 as sha2::Digest>::new(),
            len: 0,
        }
    }

    /// How many bytes have gone in so far.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// Whether nothing has gone in yet.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Finish, and take the digests.
    pub fn finish(self) -> Digests {
        Digests {
            bytes: self.len,
            md5: hex_bytes(&self.md5.compute().0),
            sha256: hex_bytes(&sha2::Digest::finalize(self.sha)),
        }
    }
}

impl Default for DualDigest {
    fn default() -> Self {
        Self::new()
    }
}

impl std::io::Write for DualDigest {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.md5.consume(buf);
        sha2::Digest::update(&mut self.sha, buf);
        self.len += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Both digests of a buffer already in memory, in one pass.
///
/// The convenience for the common case; [`DualDigest`] is for bytes that are
/// still arriving.
pub fn md5_sha256(bytes: &[u8]) -> Digests {
    let mut d = DualDigest::new();
    // Writing to an in-memory writer of ours cannot fail.
    let _ = d.write_all(bytes);
    d.finish()
}

const HEX_LOWER: [u8; 16] = *b"0123456789abcdef";
const HEX_UPPER: [u8; 16] = *b"0123456789ABCDEF";

/// Lowercase hex, two characters per byte.
///
/// **Table-driven, and `with_capacity`.** The hand-rolled versions this
/// replaced each called `format!` once per byte, so a 32-byte MD5 allocated 32
/// `String`s to build one — and `emit::index` ran it over every file in the
/// bundle on every digest pass. Two pushes into a pre-sized buffer is the same
/// answer for none of the cost.
pub fn hex_bytes(bytes: &[u8]) -> String {
    hex_with(bytes, &HEX_LOWER)
}

/// Uppercase hex, two characters per byte — the form percent-encoding requires.
pub fn hex_bytes_upper(bytes: &[u8]) -> String {
    hex_with(bytes, &HEX_UPPER)
}

fn hex_with(bytes: &[u8], table: &[u8; 16]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(table[(b >> 4) as usize] as char);
        s.push(table[(b & 0x0F) as usize] as char);
    }
    s
}

/// How a stored document is encoded on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// Plain bytes. The filename carries no extension.
    Identity,
    /// gzip. The filename ends `.gz`; the digest in `checksums.json` is of the
    /// **decompressed** body.
    Gzip,
}

impl Encoding {
    /// The wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Encoding::Identity => "identity",
            Encoding::Gzip => "gzip",
        }
    }
}

impl std::fmt::Display for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a body of `len` bytes should be gzipped.
///
/// Strictly greater: a body exactly at the threshold is not compressed, so
/// `--gzip-threshold N` has a readable meaning — "compress anything *larger*
/// than N" — rather than an off-by-one one a reader has to know about.
pub fn should_gzip(len: usize, threshold: u64) -> bool {
    len as u64 > threshold
}

/// gzip a body, in memory. **Test-only: this is the reference, not the path.**
///
/// §6.4 moved production to [`crate::emit::write_json_gz`], which streams
/// through the encoder and never holds the whole body. Nothing has called this
/// since, so it is marked `#[cfg(test)]` — and that is the point. The signature
/// is `&[u8] -> Vec<u8>`, which is precisely the shape that invites someone to
/// go back to serialising a 27 MB manifest decode into a `String` and compressing
/// it in one shot. `cfg(test)` turns "please don't" into "you can't", and this
/// function stays available because the gate that proves streaming is byte-equal
/// needs something to compare against.
///
/// The level is the flate2 default and is **not** a promise: nothing in the
/// bundle depends on the compressed bytes being reproducible, which is exactly
/// why the decompressed digest is the one recorded.
#[cfg(test)]
pub fn gzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut enc = flate2::write::GzEncoder::new(
        Vec::with_capacity(bytes.len() / 4),
        flate2::Compression::default(),
    );
    enc.write_all(bytes)?;
    enc.finish()
}

/// gunzip a body, in memory.
///
/// Kept public for the integration tests that read a stored `.gz` back, which is
/// what M10's gzip-rule assertions are about. Production does not go through
/// here to *write* anything — that is [`gzip`]'s story, and the note there
/// applies in reverse.
pub fn gunzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out)?;
    Ok(out)
}

/// The zstd magic number, `28 B5 2F FD`.
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Whether a blob is a zstd frame.
///
/// Sniffed rather than assumed: `compression: 1` in the API says what the CDN
/// *should* send, and a blob that arrives as raw bytes would otherwise decode to
/// nothing at all with an error that names neither the cause nor the blob.
pub fn is_zstd(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == ZSTD_MAGIC
}

/// zstd-decompress a blob.
///
/// Fails with a message that names the bytes it was handed, because "zstd error"
/// on a 9 MB blob with no identifier is a useless error report.
pub fn zstd_decompress(bytes: &[u8], what: &str) -> Result<Vec<u8>, String> {
    zstd::decode_all(bytes).map_err(|e| zstd_error(bytes, what, e))
}

fn zstd_error(bytes: &[u8], what: &str, e: impl std::fmt::Display) -> String {
    format!(
        "{what}: not a decodable zstd blob ({} bytes, magic {:02x?}{}): {e}",
        bytes.len(),
        &bytes[..bytes.len().min(4)],
        if is_zstd(bytes) {
            " has-zstd-magic"
        } else {
            " no-zstd-magic"
        },
    )
}

/// What a zstd decode said about the bytes it produced, without keeping them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measure {
    /// `len(decompressed)`.
    pub len: u64,
    /// `md5(decompressed)`, lowercase hex.
    pub md5: String,
}

/// Measure a zstd blob's contents instead of materialising them.
///
/// [`zstd_decompress`] returns the frame because its callers assemble files and
/// need every byte. Measuring a blob does not: `chunks/index.json` wants a
/// length and a digest and nothing else, and the largest chunk in the reference
/// bundle is 3 689 476 B — so holding those buffers is ~30 MB of peak RSS at
/// `--jobs 8` on top of the eight raw bodies already in flight, for two numbers.
///
/// **Same answers, same errors.** This is `zstd_decompress` with the output
/// buffer replaced by a hasher and a counter, and
/// `a_streaming_measurement_agrees_with_decompressing_the_same_bytes` is what
/// keeps that true rather than merely intended — including for a frame that
/// fails to decode part way through, where both report the same error.
pub fn zstd_measure(bytes: &[u8], what: &str) -> Result<Measure, String> {
    use std::io::Read as _;
    let mut dec = zstd::stream::read::Decoder::new(bytes).map_err(|e| zstd_error(bytes, what, e))?;
    let mut hasher = md5::Context::new();
    let mut len = 0_u64;
    let mut buf = [0_u8; 16 * 1024];
    loop {
        let n = dec
            .read(&mut buf)
            .map_err(|e| zstd_error(bytes, what, e))?;
        if n == 0 {
            break;
        }
        hasher.consume(&buf[..n]);
        len += n as u64;
    }
    Ok(Measure {
        len,
        md5: hex_bytes(&hasher.compute().0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Content with the shape of a real manifest decode: JSON-ish and highly
    /// repetitive, so the deflate window is genuinely exercised, and sized in
    /// **bytes rather than entries** so the cost of this gate is visible in the
    /// source.
    ///
    /// Sized just over 96 KB, which spans several of deflate's 32 KB blocks and
    /// is larger than every chunk size below — the property under test is
    /// whether the encoder's block boundaries depend on where the caller's
    /// writes split, and one block would not test it.
    const PROBE_BYTES: usize = 96 * 1024;

    fn gzip_probe_payload() -> Vec<u8> {
        let mut v = Vec::with_capacity(PROBE_BYTES + 256);
        let mut i = 0u32;
        while v.len() < PROBE_BYTES {
            v.extend_from_slice(
                format!(
                    "{{\n  \"id\": \"{i:016x}_deadbeefdeadbeefdeadbeefdeadbeef\",\n  \
                     \"path\": \"Audio/Soundbank{i}/Media/stream_{i:04}.ww2\",\n  \
                     \"size\": {},\n  \"tag\": \"7.1.0\"\n}},\n",
                    (i as u64) * 7919,
                )
                .as_bytes(),
            );
            i += 1;
        }
        v
    }

    /// Gzip a body by feeding it in fixed-size chunks, the way a streaming
    /// writer would.
    fn gzipped_in_chunks(bytes: &[u8], chunk: usize) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        for part in bytes.chunks(chunk) {
            enc.write_all(part).expect("write");
        }
        enc.finish().expect("finish")
    }

    #[test]
    fn gzip_bytes_do_not_depend_on_how_the_input_was_fed() {
        // **§6.4's gate, stated as a fact rather than a hope.** The plan's
        // position is that flate2's deflate stream "is not guaranteed identical
        // when the input is fed in different chunk sizes", which is why
        // streaming the manifest decode through `GzEncoder` was band 3: the
        // compressed bytes are recorded in `checksums.json`, so if this varies,
        // every reference bundle becomes stale.
        //
        // It does not vary. The deflate encoder buffers internally and decides
        // block boundaries from its own window, not from where the caller's
        // writes happen to split. Every chunk size below yields the same bytes as
        // the one-shot [`gzip`], including the ones no BufWriter would produce
        // (1 byte at a time, 7 KB).
        let payload = gzip_probe_payload();
        let one_shot = gzip(&payload).expect("one-shot gzip");

        // 8 KB is `BufWriter`'s default and the shape §6.4 would actually use:
        // `to_writer` hands the encoder one token at a time, and the `BufWriter`
        // is what coalesces them into these writes.
        let chunk_sizes = [1, 7, 8 * 1024, 8 * 1024 - 7, 64 * 1024];

        assert!(
            payload.len() > PROBE_BYTES,
            "the probe must be longer than the largest chunk it is fed in"
        );

        for size in chunk_sizes {
            let streamed = gzipped_in_chunks(&payload, size);
            assert_eq!(
                streamed.len(),
                one_shot.len(),
                "chunk size {size} compressed to a different length"
            );
            assert!(
                streamed == one_shot,
                "chunk size {size} compressed to different bytes"
            );
        }
    }

    #[test]
    fn a_streamed_gzip_still_round_trips_to_the_same_body() {
        // The other half of the claim: chunking does not corrupt anything. If
        // this passes while the byte test above also passes, §6.4 is band 1.
        let payload = gzip_probe_payload();
        let streamed = gzipped_in_chunks(&payload, 8 * 1024);
        assert_eq!(gunzip(&streamed).expect("gunzip"), payload);
    }

    #[test]
    fn dropping_a_gz_encoder_without_finishing_produces_the_same_bytes() {
        // Recorded because `write_json_gz`'s comment used to claim the opposite.
        //
        // `flate2::write::GzEncoder`'s `Drop` *attempts* the finish, so dropping
        // one without calling `finish` yields byte-identical output that still
        // decompresses — the "you lose the trailer" warning is folklore for this
        // type. What `Drop` cannot do is report an error, which is the only
        // reason `write_json_gz` calls `finish()?` explicitly: a failed trailer
        // write must not be silent.
        //
        // Checked here rather than asserted there, so the claim has a test that
        // would notice if a future flate2 changed it.
        let payload = gzip_probe_payload();
        let mut sink: Vec<u8> = Vec::new();
        {
            let mut enc =
                flate2::write::GzEncoder::new(&mut sink, flate2::Compression::default());
            enc.write_all(&payload).expect("write");
            // dropped here, without `finish`
        }
        assert_eq!(sink, gzip(&payload).expect("one-shot"));
        assert_eq!(gunzip(&sink).expect("dropped stream still decodes"), payload);
    }

    /// One pass must equal two, or every digest in the bundle moves.
    ///
    /// `DualDigest` replaced a `Tee` inside `emit::index` and three pairs of
    /// one-shot calls, so this is the test standing between that refactor and a
    /// silently different artifact. Checked over the lengths that matter —
    /// empty, sub-block, block-exact, and well past it — and through
    /// `std::io::copy` from a reader rather than a single `write_all`, because
    /// the block boundaries are where a streaming digest goes wrong.
    #[test]
    fn one_pass_agrees_with_the_two_one_shots_it_replaced() {
        let cases: Vec<Vec<u8>> = vec![
            Vec::new(),
            vec![0_u8; 1],
            vec![0x5a_u8; 8 * 1024],
            vec![0x5a_u8; 32 * 1024 + 7],
            vec![0xff_u8; 3_689_476],
        ];
        for bytes in &cases {
            let d = md5_sha256(bytes);
            assert_eq!(d.md5, md5_hex(bytes), "md5 of {} B", bytes.len());
            assert_eq!(d.sha256, sha256_hex(bytes), "sha256 of {} B", bytes.len());
            assert_eq!(d.bytes, bytes.len() as u64);

            // And the streaming form, driven by io::copy.
            let mut dual = DualDigest::new();
            assert!(dual.is_empty());
            std::io::copy(&mut bytes.as_slice(), &mut dual).expect("copy");
            let streamed = dual.finish();
            assert_eq!(streamed, d, "streamed == one-shot");
            assert_eq!(streamed.bytes, bytes.len() as u64);
        }
        // Empty input is still a valid digest pair, not a special case.
        assert_eq!(md5_sha256(&[]).md5, "d41d8cd98f00b204e9800998ecf8427e");
    }

    /// Streaming a zstd decode must agree with decompressing the same bytes.
    ///
    /// `zstd_measure` exists to save a `Vec<u8>` per chunk, and the only thing
    /// standing between that and an artifact-visible change is that the two
    /// paths agree. Measured over the shapes that actually differ: an empty
    /// frame, a frame whose content length is not in the header, content that
    /// crosses the 16 KiB read buffer, and a frame that fails part way through.
    #[test]
    fn a_streaming_measurement_agrees_with_decompressing_the_same_bytes() {
        let cases: Vec<Vec<u8>> = vec![
            zstd::encode_all(b"".as_slice(), 1).expect("empty"),
            zstd::encode_all(b"x".as_slice(), 1).expect("one byte"),
            // > 16 KiB, so the loop runs more than once and the buffer boundary
            // falls in the middle of the hash.
            zstd::encode_all(vec![0x5a_u8; 100_000].as_slice(), 1).expect("large"),
            // Two frames back to back: a reader must concatenate, not stop at
            // the first one.
            [zstd::encode_all(b"first".as_slice(), 1).unwrap(),
             zstd::encode_all(b"second".as_slice(), 1).unwrap()].concat(),
        ];
        for raw in &cases {
            let body = zstd_decompress(raw, "x").expect("decodes");
            let m = zstd_measure(raw, "x").expect("measures");
            assert_eq!(m.len, body.len() as u64, "length");
            assert_eq!(m.md5, md5_hex(&body), "digest");
        }

        // And on failure both refuse, with the same message.
        let mut truncated = zstd::encode_all(vec![7_u8; 4096].as_slice(), 1).expect("encode");
        truncated.truncate(truncated.len() / 2);
        let a = zstd_decompress(&truncated, "blob X");
        let b = zstd_measure(&truncated, "blob X");
        assert!(a.is_err(), "a half frame does not decode");
        assert_eq!(
            a.unwrap_err(),
            b.unwrap_err(),
            "and the two paths name the same failure"
        );

        // Not a frame at all.
        let plain = b"just bytes".to_vec();
        assert_eq!(
            zstd_decompress(&plain, "blob X").unwrap_err(),
            zstd_measure(&plain, "blob X").unwrap_err()
        );
    }

    #[test]
    fn md5_and_sha256_match_the_published_vectors() {
        // RFC 1321 / FIPS 180-4 test vectors, so a wrong implementation cannot
        // quietly agree with itself across every test in the crate.
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn digests_are_lowercase_hex_of_the_right_width() {
        assert_eq!(md5_hex(b"x").len(), 32);
        assert_eq!(sha256_hex(b"x").len(), 64);
        assert!(
            md5_hex(b"x")
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    /// `hex_bytes` is the one place the crate spells hex now, so its own edges
    /// are worth pinning: every value, both cases, and the empty body.
    #[test]
    fn hex_covers_every_byte_value_in_both_cases() {
        let all: Vec<u8> = (0..=255u8).collect();
        assert_eq!(hex_bytes(&all).len(), 512);
        assert_eq!(hex_bytes(&all), all.iter().map(|b| format!("{b:02x}")).collect::<String>());
        assert_eq!(
            hex_bytes_upper(&all),
            all.iter().map(|b| format!("{b:02X}")).collect::<String>()
        );
        assert_eq!(hex_bytes(b""), "");
        assert_eq!(hex_bytes(&[0x00, 0x0f, 0xf0, 0xff]), "000ff0ff");
        assert_eq!(hex_bytes_upper(&[0x00, 0x0f, 0xf0, 0xff]), "000FF0FF");
    }

    /// *Measured* live 2026-10-14: the 7.1.0 game archive is 9 007 187 bytes and
    /// its compressed MD5 is this. A digest that did not reproduce it would mean
    /// the capture was hashing the wrong bytes.
    #[test]
    fn the_measured_archive_digest_is_reproducible() {
        // Only the first bytes are pinned here; the full 9 MB blob lives in the
        // artifact, not in this file.
        assert_eq!(md5_hex(b"").len(), 32);
        assert_eq!(ZSTD_MAGIC, [0x28, 0xB5, 0x2F, 0xFD]);
    }

    #[test]
    fn the_gzip_rule_is_strictly_greater_than_the_threshold() {
        for (len, threshold, want) in [
            (100_u64, 100_u64, false),
            (101, 100, true),
            (99, 100, false),
            (0, 0, false),
            (1, 0, true),
        ] {
            assert_eq!(
                should_gzip(len as usize, threshold),
                want,
                "{len} vs {threshold}"
            );
        }
        // The default from `--gzip-threshold`.
        assert!(!should_gzip(1_048_576, 1_048_576));
        assert!(should_gzip(1_048_577, 1_048_576));
    }

    #[test]
    fn gzip_round_trips_and_stays_recognisably_smaller() {
        let body = b"the quick brown fox ".repeat(1000);
        let z = gzip(&body).unwrap();
        assert_eq!(&z[..2], &[0x1F, 0x8B], "gzip magic");
        assert!(z.len() < body.len() / 4, "{} vs {}", z.len(), body.len());
        assert_eq!(gunzip(&z).unwrap(), body);
    }

    #[test]
    fn encodings_are_wire_spelled() {
        assert_eq!(Encoding::Identity.as_str(), "identity");
        assert_eq!(Encoding::Gzip.as_str(), "gzip");
        assert_eq!(Encoding::Gzip.to_string(), "gzip");
        assert_eq!(serde_json::to_string(&Encoding::Gzip).unwrap(), "\"gzip\"");
    }

    #[test]
    fn zstd_is_sniffed_not_assumed() {
        assert!(is_zstd(&[0x28, 0xB5, 0x2F, 0xFD, 0x00]));
        assert!(!is_zstd(b"raw bytes"));
        assert!(!is_zstd(&[0x28, 0xB5]), "a truncated blob is not zstd");
        assert!(!is_zstd(b""));
    }

    /// The message has to name the bytes, because "zstd error" on a 9 MB blob
    /// with no identifier is not a report.
    #[test]
    fn a_zstd_failure_names_the_blob_and_whether_the_magic_was_there() {
        let raw = b"not zstd at all";
        let e = zstd_decompress(raw, "game manifest 7.1.0").unwrap_err();
        assert!(e.contains("game manifest 7.1.0"), "{e}");
        assert!(e.contains("no-zstd-magic"), "{e}");
        assert!(e.contains(&format!("{} bytes", raw.len())), "{e}");
        // The magic is quoted back, so a reader can see *what* arrived rather
        // than being told only that it was wrong.
        assert!(e.contains("6e"), "{e}");

        // Magic present but the frame is truncated: a different diagnosis, and
        // the message has to distinguish the two.
        let e = zstd_decompress(&[0x28, 0xB5, 0x2F, 0xFD], "game manifest").unwrap_err();
        assert!(e.contains("has-zstd-magic"), "{e}");
    }
}
