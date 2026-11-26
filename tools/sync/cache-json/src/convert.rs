//! The two directions of the conversion.
//!
//! Export reads the cache read-only and writes one NDJSON line at a time, so a
//! consumer can parse it while the run is still going. Import writes the new
//! redb cache into a random sibling of the destination and publishes the whole
//! file in one rename, so a failure anywhere before that rename leaves the old
//! destination intact. Only the current schema is accepted on either side.
//!
//! The cache's own API does the reading and writing: rows are handed over as
//! [`girsync_cache::FileRec`] values and encoded by the cache's wire codec.
//! This module never opens redb or postcard itself, so a change to the cache's
//! byte format cannot leave a second, stale codec here.

use anyhow::{Context, Result, bail};
use girsync_cache::{CACHE_VERSION, CacheOpen};
use std::cell::Cell;
use std::collections::HashSet;
use std::io::{self, BufRead, Read, Write};
use std::path::Path;
use tempfile::NamedTempFile;

use crate::ndjson::{self, EntryLine, Line, MetaLine};

/// Counts from one export.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportStats {
    pub entries: usize,
    pub files: usize,
    pub dirs: usize,
}

/// Counts from one import.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportStats {
    pub entries: usize,
    pub files: usize,
    pub dirs: usize,
}

/// The prefix of the temporary sibling an import writes into.
///
/// It sits beside the destination, so the name is inside girsync's own
/// `girpr-cache` namespace: a leftover from a killed import reads as tool
/// state rather than as content the next mirror would copy or delete.
pub const JSON_TEMP_PREFIX: &str = "girpr-cache-json-";

/// Line bookkeeping shared between the import's reader and its error paths.
///
/// `Cell` lets the reader, which the JSON stream deserializer owns, update the
/// counters while the caller reads them to label its own semantic errors.
struct LineState {
    line: Cell<usize>,
    bytes: Cell<usize>,
    content: Cell<bool>,
}

impl LineState {
    fn new() -> Self {
        Self {
            line: Cell::new(1),
            bytes: Cell::new(0),
            content: Cell::new(false),
        }
    }
}

/// Feed bytes to the JSON stream deserializer while counting lines and
/// refusing blank ones.
///
/// `serde_json`'s stream skips whitespace between values, so a line containing
/// only whitespace would otherwise be silently accepted. This adapter keeps
/// the import's rule that a blank line is a malformed line, and gives it the
/// same one-based line number the report's parser used to supply.
struct LineReader<'a, R> {
    inner: R,
    state: &'a LineState,
}

impl<R: Read> Read for LineReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let read = self.inner.read(buf)?;
        if read == 0 {
            // A whitespace-only last line, with no newline to trip the check
            // below, is blank as well.
            if self.state.bytes.get() > 0 && !self.state.content.get() {
                return Err(blank_line(self.state.line.get()));
            }
            return Ok(0);
        }
        for &byte in &buf[..read] {
            self.state.bytes.set(self.state.bytes.get() + 1);
            if byte == b'\n' {
                if !self.state.content.get() {
                    return Err(blank_line(self.state.line.get()));
                }
                self.state.line.set(self.state.line.get() + 1);
                self.state.bytes.set(0);
                self.state.content.set(false);
            } else if !byte.is_ascii_whitespace() {
                self.state.content.set(true);
            }
        }
        Ok(read)
    }
}

fn blank_line(line: usize) -> io::Error {
    io::Error::other(format!("line {line}: blank line"))
}

/// Attach the one-based line number a JSON error was detected at.
///
/// I/O errors carry no meaningful position in `serde_json` (it reports line
/// 0 for them); a blank-line error already names its own line.
fn json_error(err: serde_json::Error) -> anyhow::Error {
    if err.classify() == serde_json::error::Category::Io {
        anyhow::Error::new(err)
    } else {
        let line = err.line();
        anyhow::Error::new(err).context(format!("line {line}"))
    }
}

/// Export the cache at `cache_path` to `out` as NDJSON.
///
/// The cache is opened read-only: no meta rewrite, no backup, no rebuild. The
/// meta line comes first, then one line per row, streamed from a single read
/// transaction. Output is flushed before returning, so a caller writing to a
/// file can read it as soon as this returns.
pub fn export_cache<W: Write>(cache_path: &Path, mut out: W) -> Result<ExportStats> {
    let cache = girsync_cache::CacheDb::open_record(cache_path)?;
    let meta = cache.meta()?;
    if meta.version != CACHE_VERSION {
        // `open_record` already refuses a foreign version; this is the same
        // gate at the boundary that defines the text form, so an export can
        // never emit a meta line this import would refuse.
        bail!(
            "unsupported cache version {} (want {CACHE_VERSION})",
            meta.version
        );
    }
    ndjson::write_line(&mut out, &Line::Meta(MetaLine::from_meta(&meta)))
        .context("write meta line")?;

    let mut stats = ExportStats::default();
    cache.visit_entries(|path, rec| {
        ndjson::write_line(&mut out, &Line::Entry(EntryLine::from_rec(path, rec)))?;
        stats.entries += 1;
        if rec.is_file() {
            stats.files += 1;
        } else {
            stats.dirs += 1;
        }
        Ok(())
    })?;
    out.flush().context("flush NDJSON output")?;
    Ok(stats)
}

/// Import NDJSON from `input` into a new redb cache at `cache_path`.
///
/// The first line must be the meta line; the rest are entry lines. A missing
/// or duplicate meta, a foreign version, a malformed line, a duplicate path
/// and an invalid kind are all refused with the offending line number. Nothing
/// at `cache_path` is touched until the whole stream has been written, and
/// then it is replaced in one rename. Set `force` to replace an existing file.
pub fn import_cache<R: BufRead>(input: R, cache_path: &Path, force: bool) -> Result<ImportStats> {
    let state = LineState::new();
    let reader = LineReader {
        inner: input,
        state: &state,
    };
    let mut stream = serde_json::Deserializer::from_reader(reader).into_iter::<Line>();
    let first = match stream
        .next()
        .transpose()
        .map_err(|err| json_error(err).context("read meta line"))?
    {
        Some(line) => line,
        None => bail!("empty NDJSON (expected a meta line)"),
    };
    let meta = match first {
        Line::Meta(meta) => meta,
        Line::Entry(_) => bail!("line 1 is an entry; a meta line must come first"),
    };
    let case_sensitive = meta.case_sensitive().context("line 1")?;

    if cache_path.is_dir() {
        bail!(
            "destination {} is not a file; only the redb cache format is supported",
            cache_path.display()
        );
    }
    if cache_path.exists() && !force {
        bail!(
            "refusing to overwrite {} (pass --force)",
            cache_path.display()
        );
    }

    // The temporary is claimed before any entry is read, so a failure in the
    // stream leaves the same thing behind as any other failure: nothing of
    // this run at the destination. Its name is random and inside the tool's
    // namespace, and dropping the handle on an error removes the file.
    let temp = claim_temp_sibling(cache_path)?;
    let temp_path = temp.path().to_path_buf();
    let stats = match build_cache(&temp_path, &mut stream, &state, case_sensitive) {
        Ok(stats) => stats,
        Err(e) => return Err(cleanup_temp(e, &temp_path)),
    };
    replace_destination(temp, cache_path).map_err(|e| cleanup_temp(e, &temp_path))?;
    Ok(stats)
}

/// Write the rows from `stream` into the cache at `temp`.
///
/// The cache's own `open_db` creates the tables and the meta row with the case
/// mode read from the stream; `begin_write` batches the rows, committing at the
/// end and on the cache's own thresholds. `HashSet` is the one thing this import holds in memory: a
/// duplicate path would silently overwrite a row in redb, so it is refused
/// here with its line number instead.
fn build_cache(
    temp: &Path,
    stream: &mut impl Iterator<Item = Result<Line, serde_json::Error>>,
    state: &LineState,
    case_sensitive: bool,
) -> Result<ImportStats> {
    let cache = girsync_cache::open_db(
        temp,
        case_sensitive,
        CacheOpen::ReadWrite {
            ignore_cache: false,
            backup_first: false,
            retain: None,
        },
    )?;
    let mut stats = ImportStats::default();
    let mut seen = HashSet::new();
    {
        let mut write = cache.begin_write()?;
        for item in stream {
            let line = state.line.get();
            let entry = match item.map_err(json_error)? {
                Line::Entry(entry) => entry,
                Line::Meta(_) => bail!("line {line} is a second meta line"),
            };
            if !seen.insert(entry.path.clone()) {
                bail!("line {line} repeats path {:?}", entry.path);
            }
            let rec = entry
                .to_rec()
                .with_context(|| format!("line {line}, path {:?}", entry.path))?;
            write
                .put(&entry.path, &rec)
                .with_context(|| format!("write cache row at line {line}"))?;
            stats.entries += 1;
            if rec.is_file() {
                stats.files += 1;
            } else {
                stats.dirs += 1;
            }
        }
        write.commit().context("commit imported cache rows")?;
    }
    // The handle holds redb's file lock and the cache's lock sidecar. Both
    // must go before the finished file moves to its final name: an orphaned
    // sidecar would be residue in the folder, named after a file that no
    // longer exists.
    drop(cache);
    girsync_cache::remove_lock_sidecar(temp)
        .with_context(|| format!("remove lock sidecar of {}", temp.display()))?;
    Ok(stats)
}

/// Claim a free temporary sibling next to `dst`.
///
/// `tempfile` creates it atomically with a random middle and retries on a
/// collision, so two imports into one folder cannot share a temporary. The
/// handle owns the file: dropping it on any error path removes it.
fn claim_temp_sibling(dst: &Path) -> Result<NamedTempFile> {
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let mut builder = tempfile::Builder::new();
    builder.prefix(JSON_TEMP_PREFIX).suffix(".redb");
    builder
        .tempfile_in(parent)
        .with_context(|| format!("claim import temporary in {}", parent.display()))
}

/// Move the finished temporary onto the destination.
///
/// An existing file is replaced by one rename, so the destination is either
/// the old cache or the new one, never a half-written mixture. A fresh file is
/// published with a no-clobber rename, which refuses a destination that
/// appeared after the initial check.
fn replace_destination(temp: NamedTempFile, dst: &Path) -> Result<()> {
    let temp_display = temp.path().display().to_string();
    let result = if dst.is_file() {
        temp.persist(dst).map(|_| ())
    } else {
        temp.persist_noclobber(dst).map(|_| ())
    };
    result.with_context(|| {
        format!(
            "move the imported cache {temp_display} onto {}",
            dst.display()
        )
    })
}

/// Remove the temporary's lock sidecar after a failure, reporting the cleanup
/// failure beside the primary one rather than instead of it.
fn cleanup_temp(primary: anyhow::Error, temp: &Path) -> anyhow::Error {
    match girsync_cache::remove_lock_sidecar(temp) {
        Ok(()) => primary,
        Err(cleanup) => {
            let summary = format!("{primary}");
            primary.context(format!(
                "{summary}; removing the lock sidecar of {} also failed: {cleanup}",
                temp.display()
            ))
        }
    }
}
