//! Where decoded chunk bytes come from: the bundle's stored blobs, or an
//! in-memory map for a test that has no bundle.
//!
//! A trait here would be three implementors and a generic on every function
//! below; the union reads the same and keeps [`ChunkSource::decoded`] the one
//! place that decides how a chunk is obtained.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::blobs::ChunkIndex;
use crate::hash;

/// Where decoded chunk bytes come from.
///
/// Two modes and no more: the bundle's stored blobs, or an in-memory map for a
/// test that has no bundle. A trait here would be three implementors and a
/// generic on every function below; the union reads the same and keeps
/// [`ChunkSource::decoded`] the one place that decides how a chunk is obtained.
#[derive(Debug, Clone)]
pub struct ChunkSource<'i> {
    inner: Inner<'i>,
}

#[derive(Debug, Clone)]
enum Inner<'i> {
    /// Read `chunks/blobs/<id>.zst` and decompress it.
    Disk {
        root: PathBuf,
        /// Borrowed, not copied. A `BTreeMap` of every chunk's id, stored path
        /// and md5 built at construction would be a second copy of the index's
        /// whole id space for the life of the source.
        index: &'i ChunkIndex,
        /// The run's `--max-decompressed-bytes`, recorded in `MANIFEST.json`.
        /// Every blob decode is bounded by it, whatever the index declares.
        ceiling: u64,
        /// One cell per stored blob, filled on first decode, when
        /// [`ChunkSource::memoizing`] asked for it.
        cache: Option<BTreeMap<String, OnceLock<Vec<u8>>>>,
    },
    /// Already-decoded bytes, keyed by chunk id.
    Memory(BTreeMap<String, Vec<u8>>),
}

impl<'i> ChunkSource<'i> {
    /// A source that reads the bundle's stored blobs.
    ///
    /// Nothing is copied out of `index` and nothing is cached: the two expensive
    /// choices this type makes are the caller's, and both are documented on
    /// [`ChunkSource::decoded`].
    pub fn from_index(
        root: &std::path::Path,
        index: &'i ChunkIndex,
        ceiling: u64,
    ) -> ChunkSource<'i> {
        ChunkSource {
            inner: Inner::Disk {
                root: root.to_path_buf(),
                index,
                ceiling,
                cache: None,
            },
        }
    }

    /// Keep every chunk this source decodes, so one referenced N times is read
    /// and decompressed once.
    ///
    /// **Verify asks for this; capture does not**, and the reason is the same
    /// fact seen from two ends.
    ///
    /// A capture assembles each file once per side and the repeats are close
    /// together in time, so the page cache serves them; remembering the bytes
    /// instead would hold the whole fixture resident, which is the one thing
    /// capture is careful not to do.
    ///
    /// A `verify` is the opposite. It rebuilds every capture the index knows
    /// about, and a blob that two captures share — which is most of them, since
    /// sharing is the point of a blob store — is read and decompressed once per
    /// reference. Measured on the reference bundle (`gircap verify`, 38 blobs,
    /// 66 references):
    ///
    /// | | today | memoised |
    /// |---|---|---|
    /// | blob bytes read | 78,747,429 B across 104 opens | 27,612,723 B across 38 |
    /// | bytes decompressed | 70,010,206 B | 39,559,611 B |
    /// | peak resident | — | +39,559,611 B of decoded blobs |
    ///
    /// That last row is the price, and it is the reason this is opt-in: it is a
    /// real increase in peak memory in exchange for less than half the I/O.
    /// `verify` already holds every rebuilt file at once by design, so the
    /// addition is in the same order as what it already spends.
    ///
    /// The cells are sized from the index rather than grown on demand, so a
    /// chunk nobody asks for costs a `OnceLock`, not a decode.
    pub fn memoizing(mut self) -> Self {
        if let Inner::Disk { index, cache, .. } = &self.inner
            && cache.is_none()
        {
            let cells = index
                .blobs
                .iter()
                .filter(|b| b.props.stored_path.is_some())
                .map(|b| (b.props.chunk_id.clone(), OnceLock::new()))
                .collect();
            let Inner::Disk {
                root,
                index,
                ceiling,
                ..
            } = std::mem::replace(&mut self.inner, Inner::Memory(BTreeMap::new()))
            else {
                unreachable!("just matched on Disk")
            };
            self.inner = Inner::Disk {
                root,
                index,
                ceiling,
                cache: Some(cells),
            };
        }
        self
    }

    /// A source backed by already-decoded bytes.
    pub fn from_memory(map: BTreeMap<String, Vec<u8>>) -> ChunkSource<'i> {
        ChunkSource {
            inner: Inner::Memory(map),
        }
    }

    /// The decoded bytes of one chunk.
    ///
    /// Borrowed in memory mode and decompressed on demand from disk otherwise.
    /// Caching is [`ChunkSource::memoizing`]'s business; this says "nothing is
    /// cached across calls", because for capture that is the right default — the
    /// three assemblies of a 22 MB file would otherwise hold three copies of
    /// it.
    ///
    /// The md5 the index claims is checked here and not merely recorded, every
    /// time: a store that read a corrupted blob without noticing would make
    /// `verify` impossible, and the whole point of this milestone is that
    /// `verify` can trust what it reads. A memoised source checks it on the
    /// decode that fills the cell, which is still every blob it ever returns.
    pub fn decoded(&self, chunk_id: &str) -> Result<Cow<'_, [u8]>, String> {
        match &self.inner {
            Inner::Memory(m) => m
                .get(chunk_id)
                .map(|v| Cow::Borrowed(v.as_slice()))
                .ok_or_else(|| format!("chunk {chunk_id} is not in the source")),
            Inner::Disk {
                root,
                index,
                ceiling,
                cache,
            } => {
                let cell = cache.as_ref().and_then(|c| c.get(chunk_id));
                if let Some(held) = cell.and_then(OnceLock::get) {
                    return Ok(Cow::Borrowed(held.as_slice()));
                }
                let b = index
                    .get(chunk_id)
                    .ok_or_else(|| format!("chunk {chunk_id} is not in the index"))?;
                let rel = b
                    .props
                    .stored_path
                    .as_deref()
                    .ok_or_else(|| format!("chunk {chunk_id} has no stored blob"))?;
                // **The read-side twin of the manifest check.** `stored_path` is
                // a document field inside a bundle someone may have edited, and
                // this is the join that turns it into a file handle. The shape
                // rule stops a bundle from making `verify` or `materialize`
                // read outside its own root, whatever the capture that wrote the
                // index did.
                if let Some(problem) = crate::paths::relative_path_problem(rel) {
                    return Err(format!(
                        "chunk {chunk_id}: the index's stored_path {problem}"
                    ));
                }
                // **A path is not proof of usable bytes.** A partial fetch and a
                // blob whose measurement failed both keep `stored_path`, and
                // assembling from either would mix unverified bytes into a file
                // the bundle claims to reproduce.
                if let Some(why) = b.props.why_unusable() {
                    return Err(format!("chunk {chunk_id} cannot be assembled from: {why}"));
                }
                let path = root.join(rel);
                // **Decoded from the file, not from a `Vec` of it.** Reading the
                // whole blob first would keep the compressed bytes alive for the
                // whole decode beside the decompressed ones; decoding straight
                // through a buffered reader drops the compressed copy entirely, so
                // peak is the output plus one read window.
                //
                // The capacity hint comes from the index, which carries the
                // declared decompressed length; it is a hint, not an allocation
                // size. `zstd_decompress_from` also derives its hard output
                // limit from that field (capped absolutely), so a bomb blob or a
                // tiny false declaration is refused rather than materialised.
                let file = std::fs::File::open(&path)
                    .map_err(|io| format!("open {}: {io}", path.display()))?;
                let out = hash::zstd_decompress_from(
                    std::io::BufReader::with_capacity(64 * 1024, file),
                    &format!("chunk {chunk_id}"),
                    Some(b.props.uncompressed_size),
                    *ceiling,
                )?;
                if !b.props.md5_decompressed.is_empty() {
                    let got = hash::md5_hex(&out);
                    if got != b.props.md5_decompressed {
                        return Err(format!(
                            "chunk {chunk_id}: {rel} decodes to md5 {got}, the index says {}",
                            b.props.md5_decompressed
                        ));
                    }
                }
                match cell {
                    // `set` either stores the value or finds the cell already
                    // full, and both leave it populated, so handing back a
                    // borrow is sound either way — and saves copying the decode
                    // a second time into the caller's buffer.
                    Some(cell) => {
                        let _ = cell.set(out);
                        Ok(Cow::Borrowed(
                            cell.get().expect("a set cell is full").as_slice(),
                        ))
                    }
                    None => Ok(Cow::Owned(out)),
                }
            }
        }
    }

    /// How many chunks this source can serve.
    pub fn len(&self) -> usize {
        match &self.inner {
            Inner::Memory(m) => m.len(),
            Inner::Disk { index, .. } => index
                .blobs
                .iter()
                .filter(|b| b.props.stored_path.is_some())
                .count(),
        }
    }

    /// Whether it can serve anything at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
