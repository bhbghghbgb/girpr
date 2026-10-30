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
/// effective [`DEFAULT_MAX_DECOMPRESSED_BYTES`] and applies even when the
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
mod tests;
