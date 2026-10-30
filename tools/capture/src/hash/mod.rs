//! Digests and the gzip decision.
//!
//! Cross-cutting, and early: the manifest path hashes archives, the blob path
//! hashes chunk bodies, assembly MD5s a rebuilt file, and the index writer
//! digests everything it writes. Four call sites, one rule each ("MD5 what the
//! manifest says, SHA-256 so a reader can verify with a stock tool"), so the
//! helpers live here rather than being re-derived four times.
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
use std::path::{Path, PathBuf};

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
/// **One type, because one pass over a buffer is a property of the buffer, not
/// of the caller.** Every consumer here needs MD5 *and* SHA-256 over the same
/// bytes, so the one-shot and the streaming forms have to agree — and a type that
/// both are derived from is what makes that structural rather than a convention.
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

/// A file that digests every byte written to it, on the way past.
///
/// A chunk blob is 3 689 476 B at its largest, so holding one in a `Vec<u8>`
/// before writing it means up to ~30 MB of bodies resident at `--jobs 8`. This
/// writes as it reads and hashes what passes through: the digests are the same
/// numbers over the same bytes, and no caller ever holds the body — peak memory
/// is one network chunk.
///
/// **Writes the file before it hashes the buffer.** A partial `write` returns
/// `Err` and then hashes nothing, so the digest never counts bytes that did not
/// land. Order only matters because [`Self::restart`] is the other way to lose
/// them.
pub struct HashingFile {
    path: PathBuf,
    file: std::fs::File,
    digests: DualDigest,
}

impl HashingFile {
    /// Create or truncate `path` and start hashing into it.
    pub fn create(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let file = std::fs::File::create(&path)?;
        Ok(HashingFile {
            path,
            file,
            digests: DualDigest::new(),
        })
    }

    /// The file being written, for the `rename` that follows.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Throw away everything written so far and start again from empty.
    ///
    /// **Every retry must call this, and the reason is not tidy.** A failed
    /// attempt has already written part of its body into the file. Without a
    /// restart the next attempt's bytes land *after* those, and the digests then
    /// cover the concatenation of two responses — a value that looks plausible,
    /// differs from either body, and would be recorded as the blob's md5.
    ///
    /// Three things guard it, because none of them would otherwise notice:
    /// `chunks/index.json`'s A6 checks would catch the md5, but only *after* the
    /// bundle is written, and only if the bytes differ from the manifest's; and
    /// `the_digests_of_a_retried_blob_cover_only_the_body_that_landed` is the
    /// test that fails at the point of the bug.
    pub fn restart(&mut self) -> std::io::Result<()> {
        self.file.set_len(0)?;
        use std::io::Seek as _;
        self.file.rewind()?;
        self.digests = DualDigest::new();
        Ok(())
    }

    /// Flush and take the digests of everything written.
    ///
    /// Consumes the handle because the digests cannot be read out of a
    /// [`DualDigest`] without ending it. The path survives on `self.path`, so the
    /// caller can reopen the file for the decode that follows.
    pub fn finish(mut self) -> std::io::Result<Digests> {
        std::io::Write::flush(&mut self.file)?;
        Ok(self.digests.finish())
    }
}

impl std::io::Write for HashingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.write_all(buf)?;
        // Cannot fail, and not `?`'d: the digests are in memory by construction.
        let _ = self.digests.write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
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
/// Production writes through [`crate::emit::write_json_gz`], which streams
/// through the encoder and never holds the whole body. This function is
/// `#[cfg(test)]` — and that is the point. The signature is
/// `&[u8] -> Vec<u8>`, which is precisely the shape that invites someone to go
/// back to serialising a 27 MB manifest decode into a `String` and compressing it
/// in one shot. `cfg(test)` turns "please don't" into "you can't", and this stays
/// available because the gate that proves streaming is byte-equal needs something
/// to compare against.
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
/// what the gzip-rule assertions are about. Production does not go through here
/// to *write* anything — that is `gzip`'s story, and the note there applies in
/// reverse.
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

/// The default ceiling on one decompressed buffer.
///
/// **A declaration is not a permission slip.** The manifest's
/// `uncompressed_size` and a chunk's `uncompressed_size` are document fields,
/// and either may be absent or wrong; a frame that declares a small size but
/// expands to gigabytes must still be refused. 256 MiB is far above anything
/// this tool fetches (the reference manifest decodes to ~16.8 MiB and its
/// largest chunk to 3.7 MiB) while keeping a decompression bomb from reaching
/// the allocator as an abort.
///
/// **The ceiling is a flag, not a constant.** `--max-decompressed-bytes` sets
/// it, and the effective value is recorded in `MANIFEST.json` so a decode can
/// be reproduced. This name is only the flag's default.
pub const DEFAULT_MAX_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

/// The output limit for a decode whose claimed length is `expected`.
///
/// The limit is the declaration itself; the decoder is read through
/// `take(limit + 1)` so the extra byte is what detects a frame longer than it
/// claimed, without materialising the rest of a bomb. `ceiling` is the run's
/// effective [`DEFAULT_DEFAULT_MAX_DECOMPRESSED_BYTES`] and applies even when the
/// declaration is missing or absurd.
pub fn decompression_limit(expected: Option<u64>, ceiling: u64) -> u64 {
    expected.unwrap_or(ceiling).min(ceiling)
}

/// A starting capacity for a decode, never the untrusted declaration itself.
fn decode_capacity(expected: Option<u64>, limit: u64) -> usize {
    /// Grow from a small buffer; the point of the limit is that the upper bound
    /// is known, not that it is reserved up front.
    const INITIAL: u64 = 64 * 1024;
    let bounded = expected.unwrap_or(0).min(limit).min(INITIAL);
    usize::try_from(bounded).unwrap_or(usize::MAX)
}

/// zstd-decompress a blob, refusing to produce more than `limit` bytes.
///
/// **The output is bounded, not just the input.** `zstd::decode_all` expands the
/// whole frame before returning, so a small compressed body could make the
/// process allocate gigabytes (O4). The decoder is streamed through
/// `take(limit + 1)` instead; once it produces one byte past `limit` the answer
/// is a typed error, before the rest of the frame is materialised.
///
/// Fails with a message that names the bytes it was handed, because "zstd error"
/// on a 9 MB blob with no identifier is a useless error report.
pub fn zstd_decompress(bytes: &[u8], what: &str, limit: u64) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    let mut dec =
        zstd::stream::read::Decoder::new(bytes).map_err(|e| zstd_error(bytes, what, e))?;
    let mut out = Vec::with_capacity(decode_capacity(None, limit));
    let read_limit = limit.saturating_add(1);
    (&mut dec)
        .take(read_limit)
        .read_to_end(&mut out)
        .map_err(|e| zstd_error(bytes, what, e))?;
    if out.len() as u64 > limit {
        return Err(zstd_too_big_bytes(bytes, what, limit));
    }
    Ok(out)
}

/// [`zstd_decompress`] over a reader, optionally pre-sized.
///
/// This is what makes a chunk blob measurable without being held: decoding
/// straight from the file drops peak memory from `compressed + decompressed` to
/// the decompressed buffer plus one read window, which on a 22 MB file with a
/// 2 MB blob is the difference between two buffers and one.
///
/// `expected` is the decompressed length when the caller knows it, which for a
/// chunk it does — the index carries it. It is both a capacity hint and the
/// source of the hard output limit ([`decompression_limit`]); a declaration
/// that is smaller than the frame is refused rather than decoded past, and a
/// false huge declaration is clamped by the run's `ceiling`. The returned length
/// is whatever actually decoded within that bound.
pub fn zstd_decompress_from<R: std::io::Read>(
    mut reader: R,
    what: &str,
    expected: Option<u64>,
    ceiling: u64,
) -> Result<Vec<u8>, String> {
    use std::io::Read as _;
    // The magic is read up front so a refusal can quote it — a reader being
    // consumed cannot be re-read. Same shape as `zstd_measure_from`.
    let mut first = [0_u8; 4];
    let n = read_upto(&mut reader, &mut first).map_err(|e| zstd_error_streamed(&[], what, e))?;
    let head: &[u8] = &first[..n];
    if !is_zstd(head) {
        return Err(zstd_error_streamed(head, what, "not a zstd frame"));
    }
    let limit = decompression_limit(expected, ceiling);
    let mut out = Vec::with_capacity(decode_capacity(expected, limit));
    let mut dec = zstd::stream::read::Decoder::new(std::io::Cursor::new(head).chain(reader))
        .map_err(|e| zstd_error_streamed(head, what, e))?;
    (&mut dec)
        .take(limit.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|e| zstd_error_streamed(head, what, e))?;
    if out.len() as u64 > limit {
        return Err(zstd_error_streamed(
            head,
            what,
            format!("the decoded output exceeds the {limit} B decode limit"),
        ));
    }
    Ok(out)
}

/// The over-limit refusal for the buffered form, keeping the blob's own detail.
fn zstd_too_big_bytes(bytes: &[u8], what: &str, limit: u64) -> String {
    zstd_error(
        bytes,
        what,
        format!("the decoded output exceeds the {limit} B decode limit"),
    )
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
/// fails to decode part way through, where both report the same error, **and that
/// equality is asserted on the message text itself**. For a source that is not a
/// buffer, [`zstd_measure_from`], whose error text is deliberately shorter.
///
/// **Bounded by `ceiling`.** Measuring still decodes, so a frame that expands
/// past the run's limit is refused rather than hashed forever. The bound is the
/// absolute ceiling, not the frame's declaration: a measurement exists to say
/// what the frame really contains, so a false declaration has to be measured
/// past and reported, not used to truncate the answer.
pub fn zstd_measure(bytes: &[u8], what: &str, ceiling: u64) -> Result<Measure, String> {
    use std::io::Read as _;
    let mut dec =
        zstd::stream::read::Decoder::new(bytes).map_err(|e| zstd_error(bytes, what, e))?;
    let mut hasher = md5::Context::new();
    let mut len = 0_u64;
    let mut buf = [0_u8; 16 * 1024];
    loop {
        let n = dec.read(&mut buf).map_err(|e| zstd_error(bytes, what, e))?;
        if n == 0 {
            break;
        }
        len += n as u64;
        if len > ceiling {
            return Err(zstd_error(
                bytes,
                what,
                format!("the decoded output exceeds the {ceiling} B decode limit"),
            ));
        }
        hasher.consume(&buf[..n]);
    }
    Ok(Measure {
        len,
        md5: hex_bytes(&hasher.compute().0),
    })
}

/// [`zstd_measure`] over a reader rather than a buffer.
///
/// The reason a chunk blob can be measured without being held: the file it was
/// streamed into is the source, and this walks it with a 16 KB window instead of
/// a 3.7 MB `Vec`.
///
/// **The same answers, and the same refusals — one deliberate difference in the
/// error text.** `zstd_error` quotes the blob's total length, and a reader
/// being consumed cannot supply that without reading all of it, which is the thing
/// being avoided. So the message names what *is* known: the first four bytes, and
/// whether they were a frame's magic. That is the part a reader of the error
/// needs — it separates "the CDN answered with an XML error page" from "a zstd
/// frame arrived and is corrupt".
///
/// **The loop below is duplicated from [`zstd_measure`] on purpose.** Making one
/// delegate to the other, to share it, quietly downgrades the *slice* version's
/// message too — and `a_streaming_measurement_agrees_with_decompressing_the_same_bytes`
/// catches that, because that path is required to be byte-identical to
/// [`zstd_decompress`]. Ten duplicated lines beat a shared function that makes
/// one caller lie about what it knows.
///
/// `ceiling` is the run's `--max-decompressed-bytes`, for the reason
/// [`zstd_measure`] gives: the measurement bounds the decode, not the frame's
/// declaration.
pub fn zstd_measure_from<R: std::io::Read>(
    mut reader: R,
    what: &str,
    ceiling: u64,
) -> Result<Measure, String> {
    use std::io::Read as _;

    // Exactly as many bytes as `is_zstd` looks at, and no more: the magic is at
    // the front of a frame. Read before the decoder so the refusal can quote it —
    // a consumed reader cannot be re-read.
    let mut first = [0_u8; 4];
    let n = read_upto(&mut reader, &mut first).map_err(|e| zstd_error_streamed(&[], what, e))?;
    // `Copy`, so the error paths below can still name the head after the slice has
    // been moved into the decoder.
    let head: &[u8] = &first[..n];
    if !is_zstd(head) {
        return Err(zstd_error_streamed(head, what, "not a zstd frame"));
    }

    let mut dec = zstd::stream::read::Decoder::new(std::io::Cursor::new(head).chain(reader))
        .map_err(|e| zstd_error_streamed(head, what, e))?;
    let mut hasher = md5::Context::new();
    let mut len = 0_u64;
    let mut buf = [0_u8; 16 * 1024];
    loop {
        let n = dec
            .read(&mut buf)
            .map_err(|e| zstd_error_streamed(head, what, e))?;
        if n == 0 {
            break;
        }
        len += n as u64;
        if len > ceiling {
            return Err(zstd_error_streamed(
                head,
                what,
                format!("the decoded output exceeds the {ceiling} B decode limit"),
            ));
        }
        hasher.consume(&buf[..n]);
    }
    Ok(Measure {
        len,
        md5: hex_bytes(&hasher.compute().0),
    })
}

/// Fill `buf` up to its length, or as far as the reader gets. How many arrived.
fn read_upto<R: std::io::Read>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// The reader's error text: the head, and whether the magic was there.
///
/// **Says less than [`zstd_error`] on purpose** — see [`zstd_measure_from`].
fn zstd_error_streamed(head: &[u8], what: &str, e: impl std::fmt::Display) -> String {
    format!(
        "{what}: not a decodable zstd blob (first {} bytes, magic {:02x?}{}): {e}",
        head.len(),
        &head[..head.len().min(4)],
        if is_zstd(head) {
            " has-zstd-magic"
        } else {
            " no-zstd-magic"
        },
    )
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
        // **The compressed bytes are not a contract, but they are recorded, so
        // check that chunking does not move them.** flate2's deflate stream is not
        // *guaranteed* identical when the input is fed in different chunk sizes,
        // and `checksums.json` records the `.gz` files — so if this varied, every
        // reference bundle would go stale.
        //
        // It does not vary. The deflate encoder buffers internally and decides
        // block boundaries from its own window, not from where the caller's
        // writes happen to split. Every chunk size below yields the same bytes as
        // the one-shot [`gzip`], including the ones no BufWriter would produce
        // (1 byte at a time, 7 KB).
        let payload = gzip_probe_payload();
        let one_shot = gzip(&payload).expect("one-shot gzip");

        // 8 KB is `BufWriter`'s default and the shape production actually uses:
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
        // The other half of the claim: chunking does not corrupt anything.
        let payload = gzip_probe_payload();
        let streamed = gzipped_in_chunks(&payload, 8 * 1024);
        assert_eq!(gunzip(&streamed).expect("gunzip"), payload);
    }

    #[test]
    fn dropping_a_gz_encoder_without_finishing_produces_the_same_bytes() {
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
            let mut enc = flate2::write::GzEncoder::new(&mut sink, flate2::Compression::default());
            enc.write_all(&payload).expect("write");
            // dropped here, without `finish`
        }
        assert_eq!(sink, gzip(&payload).expect("one-shot"));
        assert_eq!(
            gunzip(&sink).expect("dropped stream still decodes"),
            payload
        );
    }

    fn tmp(name: &str) -> girpr_tempdir::TempDir {
        girpr_tempdir::TempDir::new("gircap-hash", name)
    }

    /// Blob-shaped bytes: binary, with repeats, long enough to span many
    /// network-chunk-sized writes.
    fn blob_bytes() -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..8_000u32 {
            v.push((i.wrapping_mul(2_654_435_761) >> 13) as u8);
            v.push((i >> 5) as u8);
            v.extend_from_slice(b"\x28\xb5\x2f\xfd chunk payload repeated ");
        }
        v
    }

    #[test]
    fn a_hashable_file_reports_the_digests_of_the_bytes_it_stored() {
        // **The whole point of streaming is that the digests are computed *while*
        // writing** rather than after, so the numbers must be the ones
        // `md5_sha256` gives for the same bytes. Fed in deliberately uneven chunks
        // — 1, 7, 4096, and the remainder — because a writer that hashed whole
        // buffers would pass a single-chunk test and fail a real one.
        let dir = tmp("hashing-file");
        let path = dir.join("blob.bin");
        let bytes = blob_bytes();
        let mut hf = HashingFile::create(&path).expect("create");
        for part in bytes.chunks(4093) {
            hf.write_all(part).expect("write");
        }
        let streamed = hf.finish().expect("finish");

        assert_eq!(streamed, md5_sha256(&bytes), "streamed digests differ");
        assert_eq!(
            std::fs::read(&path).expect("read back"),
            bytes,
            "the file must hold exactly what was digested"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_restart_discards_the_bytes_and_the_digests_of_the_failed_attempt() {
        // **The property that makes retrying safe.** A first attempt that fails
        // half way has already written those bytes. Without a restart the second
        // attempt appends after them, and the digests then describe the
        // concatenation — a value that is wrong, plausible, and undetectable
        // downstream except as an A6 failure long afterwards.
        //
        // **Both orderings, and the second is the one that bites.** A retry that
        // is *longer* than the attempt it replaces needs no truncation at all:
        // writing from position 0 simply overwrites the stale bytes and extends.
        // It is a *shorter* retry that leaves a tail of the old body behind, so
        // a test with only the first ordering passes against a `restart` that
        // does nothing at all.
        for (first, second) in [
            (
                b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>".as_slice(),
                blob_bytes().as_slice(),
            ),
            // A truncated large blob retried into a shorter one: the real shape,
            // and the one that leaves a stale tail without the truncate.
            (blob_bytes().as_slice(), b"the shorter body".as_slice()),
        ] {
            let dir = tmp("hashing-restart");
            let path = dir.join("blob.bin");

            let mut hf = HashingFile::create(&path).expect("create");
            hf.write_all(first).expect("write the aborted attempt");
            hf.restart().expect("restart");
            hf.write_all(second).expect("write the retried attempt");
            let digests = hf.finish().expect("finish");

            assert_eq!(
                digests,
                md5_sha256(second),
                "the digests must cover only the body that landed"
            );
            assert_eq!(
                std::fs::read(&path).expect("read back"),
                second,
                "the file must hold only the body that landed, with no stale tail"
            );
            assert_ne!(
                digests,
                md5_sha256(&[first, second].concat()),
                "if these are equal the test is not testing the concatenation"
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_restart_without_one_leaves_the_failed_bytes_in_the_digest() {
        // The bug itself, pinned so the property above is not satisfied by
        // accident. This is what `restart` exists to prevent, written out as a
        // passing test of the *absence* of the fix so a future refactor cannot
        // make `restart` look optional.
        let dir = tmp("hashing-no-restart");
        let path = dir.join("blob.bin");
        let first = b"partial";
        let second = b"and then the rest";

        let mut hf = HashingFile::create(&path).expect("create");
        hf.write_all(first).expect("write");
        // No `restart`.
        hf.write_all(second).expect("write");
        let digests = hf.finish().expect("finish");

        assert_eq!(
            digests,
            md5_sha256(&[first.as_slice(), second.as_slice()].concat()),
            "without a restart the digests cover both attempts"
        );
        assert_ne!(digests, md5_sha256(second));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decompressing_from_a_reader_agrees_with_decompressing_the_buffer() {
        // `ChunkSource::decoded` decodes straight off the file rather than reading
        // the blob into a `Vec` first, so the two paths must be the same function.
        // Checked on a real frame with the declaration absent, accurate and
        // oversized, because a hint that changed the output would be a silent
        // corruption of an assembled file. (A hint *smaller* than the frame is
        // now a refusal, not a truncated answer — see the bomb test below.)
        use std::io::Cursor;
        let payload = b"the quick brown fox jumps over the lazy dog".repeat(900);
        let frame = zstd::encode_all(payload.as_slice(), 3).expect("encode");

        let from_buffer =
            zstd_decompress(&frame, "blob", DEFAULT_MAX_DECOMPRESSED_BYTES).expect("buffer");
        assert_eq!(from_buffer, payload);

        for expected in [None, Some(payload.len() as u64), Some(1 << 30)] {
            let from_reader = zstd_decompress_from(
                Cursor::new(&frame),
                "blob",
                expected,
                DEFAULT_MAX_DECOMPRESSED_BYTES,
            )
            .expect("reader");
            assert_eq!(
                from_reader, payload,
                "declaration {expected:?} changed the output"
            );
        }

        let e = zstd_decompress_from(
            Cursor::new(&frame),
            "blob",
            Some(payload.len() as u64 - 1),
            DEFAULT_MAX_DECOMPRESSED_BYTES,
        )
        .expect_err("a declaration below the frame is refused, not truncated");
        assert!(e.contains("decode limit"), "{e}");
    }

    #[test]
    fn decompressing_from_a_reader_refuses_the_same_things_the_buffer_does() {
        // The refusals must match too, since `decoded` takes this path for every
        // blob.
        use std::io::Cursor;
        let xml = b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>";
        let e = zstd_decompress_from(
            Cursor::new(xml),
            "blob",
            None,
            DEFAULT_MAX_DECOMPRESSED_BYTES,
        )
        .expect_err("not a frame");
        assert!(e.contains("no-zstd-magic"), "{e}");
        assert!(e.contains("blob"), "the message must name the blob: {e}");

        let mut truncated = zstd::encode_all(vec![7_u8; 4096].as_slice(), 1).expect("encode");
        truncated.truncate(truncated.len() / 2);
        let e = zstd_decompress_from(
            Cursor::new(&truncated),
            "blob",
            None,
            DEFAULT_MAX_DECOMPRESSED_BYTES,
        )
        .expect_err("half a frame");
        assert!(e.contains("has-zstd-magic"), "{e}");
    }

    /// **O4.** A frame's *input* is bounded by the response ceiling; its output
    /// is bounded by the declared size, so a 1 MiB run of zeroes is a small
    /// frame (well under 16 KiB) and a declaration of 1 KiB must stop it at the
    /// limit rather than materialise the megabyte.
    #[test]
    fn a_decompression_bomb_is_refused_at_the_decode_limit_before_it_is_materialised() {
        use std::io::Cursor;
        let payload = vec![0_u8; 1 << 20];
        let frame = zstd::encode_all(payload.as_slice(), 1).expect("encode");
        assert!(
            frame.len() < 16 * 1024,
            "the probe is not a bomb: {} B",
            frame.len()
        );

        // The bundle/reader path: the declaration is far smaller than the frame.
        let e = zstd_decompress_from(
            Cursor::new(&frame),
            "chunk x",
            Some(1024),
            DEFAULT_MAX_DECOMPRESSED_BYTES,
        )
        .expect_err("a frame longer than its declaration must be refused");
        assert!(e.contains("decode limit"), "{e}");
        assert!(e.contains("chunk x"), "{e}");

        // The buffered manifest path: the limit is the caller's.
        let e = zstd_decompress(&frame, "manifest x", 1024)
            .expect_err("a frame over the limit must be refused");
        assert!(e.contains("decode limit"), "{e}");
        assert!(e.contains("manifest x"), "{e}");

        // An accurate declaration still decodes, so the limit is not a blanket
        // refusal and a legitimate body just above the start of the buffer works.
        let ok = zstd_decompress_from(
            Cursor::new(&frame),
            "chunk x",
            Some(payload.len() as u64),
            DEFAULT_MAX_DECOMPRESSED_BYTES,
        )
        .expect("an accurate declaration decodes");
        assert_eq!(ok.len(), payload.len());
    }

    /// The limit follows the declaration and never exceeds the absolute
    /// ceiling; the one byte the decoder is allowed past it is the overrun
    /// probe, not part of the limit.
    #[test]
    fn the_decode_limit_is_the_declaration_and_never_above_the_ceiling() {
        let ceiling = DEFAULT_MAX_DECOMPRESSED_BYTES;
        assert_eq!(decompression_limit(None, ceiling), ceiling);
        assert_eq!(decompression_limit(Some(0), ceiling), 0);
        assert_eq!(decompression_limit(Some(1024), ceiling), 1024);
        assert_eq!(decompression_limit(Some(ceiling), ceiling), ceiling);
        assert_eq!(decompression_limit(Some(u64::MAX), ceiling), ceiling);

        // A run may lower the ceiling; a declaration above it is still clamped,
        // and an absent declaration gets the run's value rather than the default.
        assert_eq!(decompression_limit(None, 4096), 4096);
        assert_eq!(decompression_limit(Some(8192), 4096), 4096);
        assert_eq!(decompression_limit(Some(512), 4096), 512);
    }

    #[test]
    fn measuring_from_a_reader_agrees_with_measuring_the_buffer() {
        // The blob path measures from the file it was streamed into, so this has
        // to be the same function with a different source — same length, same md5,
        // and the same answer for a frame that is broken. Read back through a
        // `Cursor` so the reader is genuinely a reader and not a slice in disguise.
        use std::io::Cursor;

        let payload =
            b"the quick brown fox jumps over the lazy dog, repeatedly and at length".repeat(400);
        let frame = zstd::encode_all(payload.as_slice(), 3).expect("encode");

        let from_slice =
            zstd_measure(&frame, "blob", DEFAULT_MAX_DECOMPRESSED_BYTES).expect("slice");
        let from_reader =
            zstd_measure_from(Cursor::new(&frame), "blob", DEFAULT_MAX_DECOMPRESSED_BYTES)
                .expect("reader");
        assert_eq!(from_slice, from_reader, "reader and slice must agree");
        assert_eq!(from_reader.len, payload.len() as u64);
        assert_eq!(from_reader.md5, crate::hash::md5_hex(&payload));

        // A truncated frame fails the same way from both sides, and both say
        // whether the magic was there.
        let cut = &frame[..frame.len() / 2];
        let e_slice = zstd_measure(cut, "blob", DEFAULT_MAX_DECOMPRESSED_BYTES)
            .expect_err("slice rejects it");
        let e_reader = zstd_measure_from(Cursor::new(cut), "blob", DEFAULT_MAX_DECOMPRESSED_BYTES)
            .expect_err("reader rejects it");
        for e in [&e_slice, &e_reader] {
            assert!(e.contains("has-zstd-magic"), "{e}");
            assert!(e.contains("blob"), "{e} must name the blob");
        }

        // Not a frame at all: the reader version must still be able to say the
        // magic was missing, which is the reason `Sniffed` exists.
        let xml = b"<?xml version=\"1.0\"?><Error><Code>NoSuchKey</Code></Error>";
        let e_reader = zstd_measure_from(Cursor::new(xml), "blob", DEFAULT_MAX_DECOMPRESSED_BYTES)
            .expect_err("reader rejects it");
        assert!(e_reader.contains("no-zstd-magic"), "{e_reader}");
    }

    /// A measurement decodes, so it needs the same ceiling a decode gets.
    ///
    /// The declaration is deliberately not the bound here: a frame whose
    /// declaration is smaller than its content must still be measured past the
    /// declaration so the mismatch can be recorded. The ceiling is the hard stop.
    #[test]
    fn measuring_is_bounded_by_the_ceiling() {
        use std::io::Cursor;
        let payload = vec![0_u8; 1 << 20];
        let frame = zstd::encode_all(payload.as_slice(), 1).expect("encode");

        let e = zstd_measure(&frame, "chunk x", 1024).expect_err("a bomb is not measured");
        assert!(e.contains("decode limit"), "{e}");
        assert!(e.contains("chunk x"), "{e}");

        let e = zstd_measure_from(Cursor::new(&frame), "chunk x", 1024)
            .expect_err("a bomb is not measured");
        assert!(e.contains("decode limit"), "{e}");
        assert!(e.contains("chunk x"), "{e}");

        let ok = zstd_measure(&frame, "chunk x", payload.len() as u64).expect("fits the ceiling");
        assert_eq!(ok.len, payload.len() as u64);
    }

    #[test]
    fn measuring_a_reader_streams_rather_than_buffering() {
        // The claim `zstd_measure_from` makes over the slice version: it must not
        // materialise the decoded body. A source that hands out the same bytes
        // forever would hang a buffering reader and starve a streaming one, so
        // the bound below is what proves the loop stops at end of frame.
        use std::io::Cursor;
        let payload = vec![7_u8; 400_000];
        let frame = zstd::encode_all(payload.as_slice(), 3).expect("encode");
        let m = zstd_measure_from(Cursor::new(&frame), "blob", DEFAULT_MAX_DECOMPRESSED_BYTES)
            .expect("measure");
        assert_eq!(m.len, payload.len() as u64);
    }

    /// One pass must equal two, or every digest in the bundle moves.
    ///
    /// `DualDigest` is what every digest in the crate goes through, so this is the
    /// test standing between it and a silently different artifact. Checked over the
    /// lengths that matter — empty, sub-block, block-exact, and well past it — and
    /// through `std::io::copy` from a reader rather than a single `write_all`,
    /// because the block boundaries are where a streaming digest goes wrong.
    #[test]
    fn one_pass_agrees_with_the_two_one_shots() {
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
            [
                zstd::encode_all(b"first".as_slice(), 1).unwrap(),
                zstd::encode_all(b"second".as_slice(), 1).unwrap(),
            ]
            .concat(),
        ];
        for raw in &cases {
            let body = zstd_decompress(raw, "x", DEFAULT_MAX_DECOMPRESSED_BYTES).expect("decodes");
            let m = zstd_measure(raw, "x", DEFAULT_MAX_DECOMPRESSED_BYTES).expect("measures");
            assert_eq!(m.len, body.len() as u64, "length");
            assert_eq!(m.md5, md5_hex(&body), "digest");
        }

        // And on failure both refuse, with the same message.
        let mut truncated = zstd::encode_all(vec![7_u8; 4096].as_slice(), 1).expect("encode");
        truncated.truncate(truncated.len() / 2);
        let a = zstd_decompress(&truncated, "blob X", DEFAULT_MAX_DECOMPRESSED_BYTES);
        let b = zstd_measure(&truncated, "blob X", DEFAULT_MAX_DECOMPRESSED_BYTES);
        assert!(a.is_err(), "a half frame does not decode");
        assert_eq!(
            a.unwrap_err(),
            b.unwrap_err(),
            "and the two paths name the same failure"
        );

        // Not a frame at all.
        let plain = b"just bytes".to_vec();
        assert_eq!(
            zstd_decompress(&plain, "blob X", DEFAULT_MAX_DECOMPRESSED_BYTES).unwrap_err(),
            zstd_measure(&plain, "blob X", DEFAULT_MAX_DECOMPRESSED_BYTES).unwrap_err()
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
        assert_eq!(
            hex_bytes(&all),
            all.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
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
        let e = zstd_decompress(raw, "game manifest 7.1.0", DEFAULT_MAX_DECOMPRESSED_BYTES)
            .unwrap_err();
        assert!(e.contains("game manifest 7.1.0"), "{e}");
        assert!(e.contains("no-zstd-magic"), "{e}");
        assert!(e.contains(&format!("{} bytes", raw.len())), "{e}");
        // The magic is quoted back, so a reader can see *what* arrived rather
        // than being told only that it was wrong.
        assert!(e.contains("6e"), "{e}");

        // Magic present but the frame is truncated: a different diagnosis, and
        // the message has to distinguish the two.
        let e = zstd_decompress(
            &[0x28, 0xB5, 0x2F, 0xFD],
            "game manifest",
            DEFAULT_MAX_DECOMPRESSED_BYTES,
        )
        .unwrap_err();
        assert!(e.contains("has-zstd-magic"), "{e}");
    }
}
