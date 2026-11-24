//! The text form: one JSON object per line, in two shapes.
//!
//! A stream is a [`Line::Meta`] first — the schema version and the case mode
//! the cache was written under — followed by one [`Line::Entry`] per cached
//! path. Nothing spans lines, so a consumer can parse a line at a time and an
//! import can write a row at a time without holding the whole cache in memory.
//!
//! ```text
//! {"type":"meta","version":4,"case_sensitive":true}
//! {"type":"entry","path":"a.txt","kind":"file","size":42,"mtime_ns":123,"hashes":[["md5","d41d8cd98f00b204e9800998ecf8427e"]]}
//! ```
//!
//! Digests are spelled as lowercase hex, under the algorithm name the row
//! actually carries. The pairs are a **list**, not a map: the cache's wire
//! format keeps unknown names verbatim, and a known name with a wrong-length
//! digest lives in the same set under that name, so a map could not always
//! spell a row this build can read. Ordering is the wire format's — known
//! algorithms first, then foreign names — so export is deterministic.
//!
//! Only the current schema is supported. A foreign `version` is refused at
//! import rather than reinterpreted; see [`MetaLine`].

use anyhow::{Context, Result, bail};
use girsync_cache::{CACHE_VERSION, FileRec, Meta, hash::DigestSet};
use serde::{Deserialize, Serialize};
use std::io::Write;

/// One line of a cache stream.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Line {
    /// The stream's first line: the cache's `meta` row.
    Meta(MetaLine),
    /// One cached path and its record.
    Entry(EntryLine),
}

/// The `meta` row in text form.
///
/// `version` must be exactly [`CACHE_VERSION`]. This tool is current-schema
/// only: there is no conversion of older rows, so a stream that claims another
/// version is refused instead of being written under the wrong codec.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetaLine {
    pub version: u32,
    pub case_sensitive: bool,
}

impl MetaLine {
    /// Build the text row from a cache's decoded `meta`.
    pub fn from_meta(meta: &Meta) -> Self {
        Self {
            version: meta.version,
            case_sensitive: meta.case_sensitive,
        }
    }

    /// Check the version and return the case mode for the new cache.
    pub fn case_sensitive(&self) -> Result<bool> {
        if self.version != CACHE_VERSION {
            bail!(
                "unsupported cache version {} in NDJSON (want {CACHE_VERSION}; only the current schema is supported)",
                self.version
            );
        }
        Ok(self.case_sensitive)
    }
}

/// One cached path in text form.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntryLine {
    /// The `/`-separated relative path that keys the row.
    pub path: String,
    /// `"file"` or `"dir"`, the cache's closed kind registry.
    pub kind: String,
    pub size: u64,
    /// Nanoseconds since the epoch.
    pub mtime_ns: i64,
    /// `(algorithm name, lowercase hex digest)` pairs.
    pub hashes: Vec<(String, String)>,
}

impl EntryLine {
    /// Spell a decoded row as text.
    ///
    /// The order is the wire format's: known digests in canonical order, then
    /// names this build does not know (including a known name whose digest has
    /// an unexpected length). That keeps export byte-for-byte reproducible for
    /// the same cache content.
    pub fn from_rec(path: &str, rec: &FileRec) -> Self {
        let mut hashes = Vec::with_capacity(rec.hashes.len());
        for (algo, digest) in rec.hashes.iter() {
            hashes.push((algo.as_str().to_string(), girpr_digest::encode_hex(digest)));
        }
        for (name, digest) in rec.hashes.foreign_iter() {
            hashes.push((name.to_string(), girpr_digest::encode_hex(digest)));
        }
        Self {
            path: path.to_string(),
            kind: rec.kind.clone(),
            size: rec.size,
            mtime_ns: rec.mtime_ns,
            hashes,
        }
    }

    /// Decode back into the row the cache stores.
    ///
    /// Hex is decoded through the shared primitive; algorithm names go through
    /// [`DigestSet::insert_named`], which is the cache's own routing rule, so
    /// an unknown name is carried verbatim and a known name with an unexpected
    /// length lands where the wire decoder would put it. `kind` is a closed
    /// registry here just as it is in the wire format.
    pub fn to_rec(&self) -> Result<FileRec> {
        if self.kind != "file" && self.kind != "dir" {
            bail!("invalid record kind {:?}", self.kind);
        }
        let mut hashes = DigestSet::default();
        for (name, hex) in &self.hashes {
            let digest = girpr_digest::decode_hex(hex)
                .with_context(|| format!("decode hex digest for algorithm {name:?}"))?;
            hashes.insert_named(name, &digest);
        }
        Ok(FileRec {
            kind: self.kind.clone(),
            size: self.size,
            mtime_ns: self.mtime_ns,
            hashes,
        })
    }
}

/// Parse one line. A bad line names the JSON error; the caller adds the line
/// number and any context about which stream it came from.
pub fn parse_line(text: &str) -> Result<Line> {
    serde_json::from_str(text).context("parse NDJSON line")
}

/// Write one line, including its newline.
///
/// The caller owns the buffered writer; this only keeps the object-then-newline
/// shape in one place so export cannot accidentally emit pretty JSON or omit
/// the separator.
pub fn write_line<W: Write>(out: &mut W, line: &Line) -> Result<()> {
    serde_json::to_writer(&mut *out, line).context("render NDJSON line")?;
    out.write_all(b"\n").context("write NDJSON newline")
}
