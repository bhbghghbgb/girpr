use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

const MD5_BUF: usize = 512 * 1024;

/// The MD5 of the empty file — what a missing (or empty) file hashes to.
///
/// Named rather than inlined so the "the file is not there" answer and the
/// "the file is there and empty" answer cannot drift apart.
pub const EMPTY_MD5: &str = "d41d8cd98f00b204e9800998ecf8427e";

/// A byte range of a file, with the MD5 the manifest expects for its content.
///
/// `(offset, len, expected_md5)`. The MD5 is the *uncompressed* chunk MD5 out
/// of the Sophon manifest, so a range is "correct" exactly when the bytes at
/// `offset..offset+len` hash to it.
pub type ScanRange<'a> = (u64, u64, &'a str);

/// What one read pass over a file answers.
///
/// The scan exists so the repair loop needs **one** read per file instead of
/// three: the whole-file MD5 (the skip check), the per-chunk verdict ("is
/// this chunk already correct where it belongs?") and the per-source verdict
/// ("is this reuse source intact?") all come out of the same pass over the
/// bytes. Every answer is derived from bytes that were read anyway, so asking
/// for them costs hashing and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// Whole-file MD5 of what was on disk. The MD5 of the empty file when the
    /// file was absent.
    pub file_md5: String,
    /// Bytes on disk (`0` when the file was absent).
    pub file_len: u64,
    /// Per destination range, **in the order given**: is the range already
    /// correct where it belongs?
    pub dest_ok: Vec<bool>,
    /// Per candidate source range, **in the order given**: is the source
    /// intact?
    pub source_ok: Vec<bool>,
}

impl Scan {
    /// The scan of a file that is not there.
    ///
    /// "Nothing is correct" is the *answer*, not a failure: a file that does
    /// not exist has no correct chunk and no intact reuse source, and the
    /// repair of it is a full write. A missing file therefore never fails the
    /// scan.
    pub fn absent(dest_len: usize, source_len: usize) -> Self {
        Self {
            file_md5: EMPTY_MD5.to_string(),
            file_len: 0,
            dest_ok: vec![false; dest_len],
            source_ok: vec![false; source_len],
        }
    }
}

/// One byte range being hashed, with the state of its digest.
struct RangeState {
    offset: u64,
    len: u64,
    /// Lowercased expected MD5 — the manifest's spelling is not canonical.
    expect: String,
    ctx: md5::Context,
    /// Bytes of this range actually read. A range that runs past the end of
    /// the file is short, and a short range is never correct.
    seen: u64,
}

impl RangeState {
    /// Is the range exactly what the manifest says it should be?
    ///
    /// Three things have to hold: every byte of the range was on disk, the
    /// range hashes to the expected MD5, and the expectation was not empty
    /// (an empty MD5 is "unknown", not "matches").
    fn is_ok(&self) -> bool {
        self.seen == self.len
            && !self.expect.is_empty()
            && format!("{:x}", self.ctx.clone().compute()) == self.expect
    }
}

/// One read pass: the whole-file MD5 plus a verdict per byte range.
///
/// `dest` are the ranges the file *should* have (the manifest's chunk
/// destinations); `source` are the ranges a chunk could be *reused* from
/// (the local reuse map's offsets). The two lists are independent — the same
/// range can be in both, and a source is usually somewhere else in the file
/// — and the verdicts are returned in the order the ranges were given.
///
/// A file that is not there is **not** an error: the scan says nothing is
/// correct (see [`Scan::absent`]). A file that is shorter than a range makes
/// that range incorrect, because a short range cannot hash to anything.
pub fn scan_file(path: &Path, dest: &[ScanRange<'_>], source: &[ScanRange<'_>]) -> Result<Scan> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Scan::absent(dest.len(), source.len()));
        }
        Err(e) => return Err(e).with_context(|| format!("open {}", path.display())),
    };
    scan_reader(&mut f, dest, source)
}

/// [`scan_file`], over a handle the caller already holds.
///
/// The in-place repair opens its target `read+write` and then has to write to
/// it, so the scan cannot own the handle: it borrows it, and the caller seeks
/// before every write afterwards.
///
/// The scan always starts at the beginning of the file: the handle's current
/// position is whatever the caller's last read or write left it at, and a
/// scan that started there would hash half a file.
pub fn scan_reader(f: &mut File, dest: &[ScanRange<'_>], source: &[ScanRange<'_>]) -> Result<Scan> {
    f.seek(SeekFrom::Start(0))?;
    let mut ranges: Vec<RangeState> = Vec::new();
    let mut index: HashMap<(u64, u64, String), usize> = HashMap::new();
    let mut dest_idx = Vec::with_capacity(dest.len());
    let mut source_idx = Vec::with_capacity(source.len());
    // The two lists are interned separately, so a range that appears in both
    // shares one digest and each list's verdict lands on the right range.
    for (offset, len, md5) in dest.iter() {
        dest_idx.push(intern(&mut ranges, &mut index, *offset, *len, md5));
    }
    for (offset, len, md5) in source.iter() {
        source_idx.push(intern(&mut ranges, &mut index, *offset, *len, md5));
    }

    // The sweep walks the file once, in 512 KiB blocks, and feeds every range
    // the bytes that fall inside it. The ranges are sorted by offset so the
    // block's overlap with them is a contiguous walk, not a scan of the list.
    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| (ranges[i].offset, ranges[i].len));

    let mut whole = md5::Context::new();
    let mut buf = vec![0u8; MD5_BUF];
    let mut pos: u64 = 0;
    let mut total: u64 = 0;
    let mut first = 0usize;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        whole.consume(&buf[..n]);
        let block_end = pos + n as u64;
        // Ranges that ended before this block are done with; the rest are
        // visited in order and take the part of themselves inside the block.
        while first < order.len() {
            let r = &ranges[order[first]];
            if r.offset + r.len <= pos {
                first += 1;
            } else {
                break;
            }
        }
        let mut j = first;
        while j < order.len() {
            let ri = order[j];
            let r = &ranges[ri];
            if r.offset >= block_end {
                break;
            }
            let start = r.offset.max(pos);
            let end = (r.offset + r.len).min(block_end);
            if end > start {
                let a = (start - pos) as usize;
                let b = (end - pos) as usize;
                ranges[ri].ctx.consume(&buf[a..b]);
                ranges[ri].seen += end - start;
            }
            j += 1;
        }
        pos = block_end;
        total += n as u64;
    }

    Ok(Scan {
        file_md5: format!("{:x}", whole.compute()),
        file_len: total,
        dest_ok: dest_idx.iter().map(|&i| ranges[i].is_ok()).collect(),
        source_ok: source_idx.iter().map(|&i| ranges[i].is_ok()).collect(),
    })
}

/// Find or create the range state for `(offset, len, md5)`.
///
/// Two ranges are the same range when they cover the same bytes **and**
/// expect the same content; the same bytes with a different expectation are
/// two different questions, so they get two digests.
fn intern(
    ranges: &mut Vec<RangeState>,
    index: &mut HashMap<(u64, u64, String), usize>,
    offset: u64,
    len: u64,
    md5: &str,
) -> usize {
    let expect = md5.to_ascii_lowercase();
    let key = (offset, len, expect.clone());
    if let Some(&i) = index.get(&key) {
        return i;
    }
    let i = ranges.len();
    ranges.push(RangeState {
        offset,
        len,
        expect,
        ctx: md5::Context::new(),
        seen: 0,
    });
    index.insert(key, i);
    i
}

/// Read `len` bytes at `offset` into a buffer.
///
/// The in-place repair's capture pass: the bytes of a reuse source, read
/// before anything overwrites them. A short read is an error rather than a
/// silent truncation — the caller has to know it did not get what it asked
/// for.
pub fn read_range(f: &mut File, offset: u64, len: u64) -> Result<Vec<u8>> {
    f.seek(SeekFrom::Start(offset))?;
    let mut out = vec![0u8; len as usize];
    f.read_exact(&mut out)?;
    Ok(out)
}

/// Lowercase hex MD5 of a file.
pub fn md5_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut buf = vec![0u8; MD5_BUF];
    let mut ctx = md5::Context::new();
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
    }
    Ok(format!("{:x}", ctx.compute()))
}

/// MD5 of a byte slice range of a file [offset, offset+len).
pub fn md5_file_slice(path: &Path, offset: u64, len: u64) -> Result<String> {
    use std::io::Seek;
    use std::io::SeekFrom;
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    f.seek(SeekFrom::Start(offset))?;
    let mut ctx = md5::Context::new();
    let mut remaining = len;
    let mut buf = vec![0u8; MD5_BUF.min(1 << 20)];
    while remaining > 0 {
        let want = (remaining as usize).min(buf.len());
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(format!("{:x}", ctx.compute()))
}

/// What a file-length probe found.
///
/// `Absent` is exactly `NotFound`. Every other `metadata` error — a denied
/// attribute read, a sharing violation, an invalid component — is
/// `Unreadable`, not "absent" and not "zero": collapsing the two makes a
/// present file look missing and a check report a size mismatch for a file it
/// never managed to stat.
#[derive(Debug)]
pub enum FileLen {
    /// The file exists; this is its length.
    Present(u64),
    /// The path is not there.
    Absent,
    /// The path could not be statted.
    Unreadable(std::io::Error),
}

/// Classify one `metadata` result.
///
/// Split from [`file_len`] so the absent/unreadable split can be pinned with
/// an injected `io::Error`, the way `post::classify_deprecated` is.
fn classify_len(stat: std::io::Result<std::fs::Metadata>) -> FileLen {
    match stat {
        Ok(m) => FileLen::Present(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileLen::Absent,
        Err(e) => FileLen::Unreadable(e),
    }
}

/// Length of a path with absence and an unreadable stat kept distinct.
pub fn file_len(path: &Path) -> FileLen {
    classify_len(std::fs::metadata(path))
}

/// Validate a **server-controlled relative path** and return it in `/` form.
///
/// The manifest, the deprecated-files list and the audio directory names are
/// all strings a server chose. [`Path::join`] **replaces** the base for an
/// absolute path, for a Windows drive-relative path (`C:evil`), and for a UNC
/// path, and `..` walks out of it. This refuses all of those spellings before
/// any sink sees them:
///
/// - empty, a drive prefix (`C:`), a root (`/x`, `\\server\share`), or any
///   `ParentDir` component;
/// - anything that names no file (only `.` components);
/// - a trailing separator (`game/`): every caller expects the rel to name one
///   object, and a spelling that also says "directory" cannot be both;
/// - Windows name rules, because the install is a Windows tree even when the
///   checker runs elsewhere: a reserved device name (`NUL`, `con.dat`,
///   `COM1`, `CONIN$`, …), a component ending in `.` or a space (Windows
///   strips those and aliases the name), or a `:` other than the drive prefix
///   (an NTFS alternate data stream).
///
/// The check is lexical and platform-independent: `C:evil` is a `Prefix`
/// component on Windows but a plain component on Unix, so the drive spelling is
/// rejected explicitly rather than left to `std`.
///
/// This is not a filesystem check; [`contained_join`] adds that.
pub fn contained_rel(rel: &str) -> Result<PathBuf> {
    let normalized = rel.replace('\\', "/");
    anyhow::ensure!(!normalized.is_empty(), "path is empty");
    anyhow::ensure!(
        !normalized.ends_with('/'),
        "path has a trailing separator: {rel}"
    );
    let bytes = normalized.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        anyhow::bail!("path has a drive prefix: {rel}");
    }
    let path = Path::new(&normalized);
    anyhow::ensure!(
        !path.has_root() && !path.is_absolute(),
        "path is absolute: {rel}"
    );
    let mut named = false;
    // Build the cleaned form rather than returning the input: `a/./b` is
    // accepted but must not survive as `a/./b`, because the plan stores this
    // string and every later comparison (the purge's expected set, the reuse
    // map, the scope) works on the plain form. A leading `.` in particular
    // would make `expected_paths()` disagree with the purge walk — which
    // reports `game/foo.dat` — and the cleanup would delete the file this run
    // just repaired.
    let mut clean = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(part) => {
                let part = part
                    .to_str()
                    .with_context(|| format!("path is not UTF-8: {rel}"))?;
                anyhow::ensure!(
                    !part.ends_with('.') && !part.ends_with(' '),
                    "path has a Windows-trailing dot or space: {rel}"
                );
                anyhow::ensure!(
                    !part.contains(':'),
                    "path has a Windows alternate-stream separator: {rel}"
                );
                anyhow::ensure!(
                    !is_windows_device_name(part),
                    "path names a Windows device: {rel}"
                );
                named = true;
                clean.push(part);
            }
            // `a/./b` is still contained; it just says nothing.
            Component::CurDir => {}
            _ => anyhow::bail!("path is not a plain relative path: {rel}"),
        }
    }
    anyhow::ensure!(named, "path names no file: {rel}");
    Ok(clean)
}

/// Validate a server-controlled relative path and return the `/`-separated
/// string every gate and sink shares.
///
/// This is the lexical half of [`contained_join`] with no filesystem step. It
/// exists so a caller normalises a server or config spelling **once**, at the
/// boundary where it enters the run, and then reuses that exact string for the
/// path-scope decision and for the filesystem operation. Cleaning only at the
/// sink (as `contained_join` does) leaves the gate comparing a different
/// spelling, so `./a/b` can pass a scope that hides `a/b`.
pub fn clean_rel(rel: &str) -> Result<String> {
    Ok(contained_rel(rel)?.to_string_lossy().replace('\\', "/"))
}

/// Is this component a name Windows reserves for a device in every directory?
///
/// The comparison uses the part before the first `.` — `NUL.txt` is still the
/// `NUL` device — and ignores case, which NTFS does for these names too.
fn is_windows_device_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or_default();
    let upper = stem.trim_end_matches([' ', '.']).to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        upper
            .strip_prefix(prefix)
            .is_some_and(|n| n.len() == 1 && ('1'..='9').contains(&n.chars().next().unwrap()))
    })
}

/// Join a server-controlled relative path under `base`, refusing anything that
/// resolves outside it.
///
/// The lexical check is [`contained_rel`]. The filesystem check resolves the
/// **nearest existing ancestor** of both sides — so a missing `game_dir` (a
/// `--print-plan` run, a fresh install) still works — and requires the joined
/// path to stay under the base. That is what catches a symlink inside the
/// install that points out of it, which a lexical `starts_with` cannot.
pub fn contained_join(base: &Path, rel: &str) -> Result<PathBuf> {
    let rel = contained_rel(rel)?;
    let joined = base.join(rel);
    let base_resolved = resolve_existing_prefix(base)
        .with_context(|| format!("resolve the game dir {}", base.display()))?;
    let joined_resolved = resolve_existing_prefix(&joined)
        .with_context(|| format!("resolve {}", joined.display()))?;
    anyhow::ensure!(
        joined_resolved.starts_with(&base_resolved),
        "path escapes {}: {}",
        base.display(),
        joined.display()
    );
    Ok(joined)
}

/// Canonicalize `path` as far as it exists, then append the missing tail.
///
/// `canonicalize` fails on a path that is not there yet, and the plan runs
/// before `game_dir` is created; resolving the longest existing prefix keeps
/// the symlink resolution where it matters without requiring the leaf (or the
/// whole tree) to exist.
///
/// Only [`ErrorKind::NotFound`] means "this component is not there yet". Every
/// other error means the component **could not be resolved** — a denied
/// attribute read, a sharing violation, a regular file used as a directory —
/// and is returned instead of being swallowed. Treating such a component as a
/// literal name and continuing lexically is exactly the "filesystem check that
/// silently became a lexical check" failure this helper exists to prevent: a
/// symlink whose target cannot be canonicalized would pass the containment
/// comparison.
fn resolve_existing_prefix(path: &Path) -> Result<PathBuf> {
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path;
    loop {
        match std::fs::canonicalize(cur) {
            Ok(mut resolved) => {
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match (cur.parent(), cur.file_name()) {
                    (Some(parent), Some(name)) => {
                        missing.push(name.to_os_string());
                        cur = parent;
                    }
                    // No existing prefix at all (or a root): the lexical form is
                    // the best answer, and both sides use the same rule.
                    _ => return Ok(path.to_path_buf()),
                }
            }
            Err(e) => {
                return Err(e).with_context(|| format!("canonicalize {}", cur.display()));
            }
        }
    }
}

/// Parse `config.ini` last `game_version=` match.
/// NOTE(starward-parity): last-match-wins mirrors `GetLocalGameVersionAsync`
/// (`matches[^1]` over `game_version=(.+)`); missing file means fresh install.
/// Do not "fix" to first-match.
///
/// `Ok(None)` is the documented fresh-install/absent case (and a file with no
/// match). A read error other than `NotFound` is `Err`: an unreadable
/// `config.ini` must not silently look like a fresh install and overwrite the
/// version on the next bump.
/// See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L47-L70>
pub fn read_game_version(game_dir: &Path) -> anyhow::Result<Option<String>> {
    let p = game_dir.join("config.ini");
    let text = match std::fs::read_to_string(&p) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("read {}", p.display())),
    };
    let re = regex::Regex::new(r"(?m)^game_version\s*=\s*(.+?)\s*$")
        .expect("the game_version pattern is a literal");
    Ok(re
        .captures_iter(&text)
        .last()
        .map(|c| c[1].trim().to_string()))
}

/// Read res_category ignore file: JSON-lines {"category":"...","is_delete":true}.
///
/// `Ok(None)` means the file is not there (the launcher did not configure
/// it); `Err` means it is there but could not be read **or** its text could
/// not be parsed into any entry, which must not become an empty ignore set
/// without a trace. One leading UTF-8 BOM is stripped; a mixed file warns per
/// bad line and keeps the well-formed entries (see [`parse_config_list`]).
///
/// A line is an entry only when `is_delete` is `true`; an absent or `false`
/// `is_delete` is the launcher's explicit no-op. A wrong-typed `is_delete`,
/// or `is_delete: true` without a string `category`, is a malformed entry —
/// not a silent skip: ignoring it would leave a category the user excluded
/// repaired.
pub fn read_ignore_categories(path: &Path) -> anyhow::Result<Option<HashSet<String>>> {
    let Some(text) = read_config_list(path, "ignore categories")? else {
        return Ok(None);
    };
    let set = parse_config_list(path, "ignore categories", &text, |v| {
        let Some(is_delete) = v.get("is_delete") else {
            // `{}` and an absent `is_delete` are the launcher's no-op.
            return Ok(None);
        };
        let is_delete = is_delete.as_bool().ok_or_else(|| {
            anyhow::anyhow!(
                "`is_delete` must be a boolean, found {}",
                json_kind(is_delete)
            )
        })?;
        if !is_delete {
            return Ok(None);
        }
        let category = v.get("category").and_then(|c| c.as_str()).ok_or_else(|| {
            anyhow::anyhow!("`category` must be a string when `is_delete` is true")
        })?;
        Ok(Some(category.to_string()))
    })?;
    Ok(Some(set))
}

/// Read blacklist file: JSON-lines {"fileName":"..."}.
///
/// Entries are normalised once, here at the boundary, through
/// [`clean_rel`]: the plan compares them against the cleaned manifest rel, so
/// a `./`- or `//`-spelled entry must become the same string or it protects
/// nothing. An entry that cannot be a contained rel is a broken explicit
/// configuration and is refused with the entry in the error, not dropped
/// (silently ignoring it is the fail-open this normalisation closes).
///
/// The absent/unreadable/parse split is the same as
/// [`read_ignore_categories`]: a missing file is an empty blacklist; a file
/// that cannot be read, or whose only text is unparseable or malformed, is an
/// error. A line without a string `fileName` is a malformed entry, not a
/// silent no-op (the case where an explicit exclusion protects nothing).
pub fn read_blacklist(path: &Path) -> anyhow::Result<Option<HashSet<String>>> {
    let Some(text) = read_config_list(path, "blacklist")? else {
        return Ok(None);
    };
    let raw = parse_config_list(path, "blacklist", &text, |v| {
        let name = v.get("fileName").and_then(|n| n.as_str()).ok_or_else(|| {
            anyhow::anyhow!(
                "`fileName` must be a string, found {}",
                v.get("fileName").map_or("nothing", json_kind)
            )
        })?;
        Ok(Some(name.to_string()))
    })?;
    raw.into_iter()
        .map(|entry| clean_rel(&entry).with_context(|| format!("blacklist entry {entry:?}")))
        .collect::<anyhow::Result<HashSet<_>>>()
        .map(Some)
}

/// Parse one of the launcher's JSON-lines lists.
///
/// One leading UTF-8 BOM is stripped first, as in
/// [`crate::repair::audio::read_scan_file`]: the launcher may write one, and
/// U+FEFF is not whitespace, so `trim` would leave it glued to the first `{`
/// and make that line unparseable.
///
/// A line that is not JSON is warned with its 1-based line number and skipped.
/// So is a parseable line that is not a JSON object or whose required fields
/// are missing or wrong-typed: the reader's closure returns `Err`, which is
/// the same malformed-entry state (a `{"fileName":123}` must not silently
/// disable an explicit exclusion). Skipping bad lines keeps the well-formed
/// entries usable when the launcher wrote a crash-truncated or corrupt tail.
/// But a file whose non-blank text produced no entry **and** had at least one
/// bad line is refused: degrading an explicit configuration to "delete
/// nothing" (ignore categories) or "no blacklist" would silently change what
/// the run repairs and keeps, which is what R-6 fixed for the read errors. A
/// valid but entryless line (`{}`) and a genuinely blank file stay empty
/// selections.
fn parse_config_list(
    path: &Path,
    what: &str,
    text: &str,
    mut entry: impl FnMut(&serde_json::Value) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<HashSet<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut set = HashSet::new();
    let mut first_failure: Option<(usize, String)> = None;
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let failure = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) if !v.is_object() => Some(anyhow::anyhow!("the line is not a JSON object")),
            Ok(v) => match entry(&v) {
                Ok(Some(entry)) => {
                    set.insert(entry);
                    None
                }
                Ok(None) => None,
                Err(e) => Some(e),
            },
            Err(e) => Some(e.into()),
        };
        if let Some(e) = failure {
            tracing::warn!(
                path = %path.display(),
                line = i + 1,
                error = %e,
                "invalid entry in the {what} list; ignored"
            );
            first_failure.get_or_insert((i + 1, e.to_string()));
        }
    }
    if set.is_empty()
        && let Some((line, e)) = first_failure
    {
        return Err(anyhow::anyhow!(
            "{what} file {} has no usable entry: line {line}: {e}",
            path.display()
        ));
    }
    Ok(set)
}

/// A short name for a JSON value's type, for a malformed-entry message.
fn json_kind(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// The shared absent/error split for the launcher's JSON-lines config files.
fn read_config_list(path: &Path, what: &str) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("read {what} file {}", path.display())),
    }
}

/// Whether the test-only `GIRPR_TEST_NO_FSYNC` hook is on.
///
/// Default **off**: an unset variable, or any value other than an unambiguous
/// truthy one (`1`, `true`, `yes`, `on`), leaves the `config.ini` `sync_all` in
/// place, so production still flushes the temp file before the rename. Tests
/// opt in through `.config/nextest.toml` to stop paying an HDD flush per real
/// repair; the value is read once per process, not once per write. The temp-file
/// write and the rename are unchanged, so the file contents and any reopen
/// assertion are unaffected — only the durable write-back is skipped.
fn test_no_fsync() -> bool {
    static NO_FSYNC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NO_FSYNC.get_or_init(|| {
        matches!(
            std::env::var("GIRPR_TEST_NO_FSYNC").as_deref(),
            Ok("1") | Ok("true") | Ok("yes") | Ok("on")
        )
    })
}

/// Bump (or create) game-dir `config.ini` with latest version + channel fields.
///
/// Collapse parity (`GameVersionBase.UpdateGameVersion` /
/// `UpdateGameChannels` and `Hi3Helper.Data.IniFile` load/save):
/// section-preserving surgical update.
/// Every `[section]` survives (duplicate sections merged, names matched
/// case-insensitively); only keys inside `[General]` (matched
/// case-insensitively) are forced; everything else passes through untouched.
/// Comment (`;` / `#`, no-`=` lines) and blank lines are dropped on load and a
/// single blank separator is re-emitted after each section, because neither
/// launcher's INI layer round-trips them.
///
/// Forced in `[General]`: `game_version=<latest>`, `channel` / `sub_channel` /
/// `cps` per biz (cn `1/1/hyp_mihoyo`, global `1/0/hyp_hoyoverse`, bili
/// `14/0/hyp_mihoyo`) and `game_biz` — a deliberate Starward carryover
/// (`SetGameConfigIniAsync` writes it, Collapse never does; game/launcher
/// interop expects it, so it stays forced).
/// `sdk_version`: Collapse parity — preserved when present, created empty when
/// missing (Collapse only ever holds the empty `DefaultIniVersion` default;
/// the real SDK version lives in `plugin_sdk_version`, which girpr never
/// touches). `uapc`, `wpf_version`, `predownload`, `plugin_*_version` are pure
/// passthrough: preserved when present, never created.
///
/// Save shape matches Collapse `IniFile.SaveInner`: sections in first-seen
/// order (`[General]` appended last when missing), keys within a section
/// sorted alphabetically (case-insensitive), UTF-8, blank line after each
/// section. Header-less keys (no preceding `[section]`) are treated as
/// `[General]` — a deliberate deviation from Collapse's generic `[default]`
/// bucket, since game-dir files are `[General]`-scoped.
///
/// See Collapse
/// `CollapseLauncher/Classes/GameManagement/Versioning/GameVersionBase.IniConfig.cs`
/// (`UpdateGameVersion`, `UpdateGameChannels`, `DefaultIniVersion`) and
/// `Hi3Helper.Core/Data/IniFile.cs` (`LoadInner`, `SaveInner`,
/// `TryAddOrOverrideSectionValue`); Starward `game_biz` force in
/// `src/Starward.RPC/GameInstall/GameInstallService.cs::SetGameConfigIniAsync`.
pub fn write_config_ini(
    game_dir: &Path,
    latest: &str,
    biz: &str,
    channel: (&str, &str, &str),
    dry_run: bool,
) -> Result<()> {
    struct Section {
        /// First-seen spelling (e.g. `General`); `General` when created.
        name: String,
        lower: String,
        /// (first-seen spelling, lowercase key, value).
        keys: Vec<(String, String, String)>,
    }
    impl Section {
        fn find_key(&self, lower: &str) -> Option<usize> {
            self.keys.iter().position(|(_, l, _)| l == lower)
        }
        /// Last-wins insert: update value, keep first-seen spelling.
        fn insert(&mut self, key: &str, value: &str) {
            let lower = key.to_ascii_lowercase();
            match self.find_key(&lower) {
                Some(i) => self.keys[i].2 = value.to_string(),
                None => self.keys.push((key.to_string(), lower, value.to_string())),
            }
        }
        /// Forced-key upsert in `[General]`: overwrite value, keep first-seen
        /// spelling when the key already exists (matches Collapse's
        /// case-insensitive indexer); create with canonical spelling otherwise.
        fn force(&mut self, key: &str, value: &str) {
            self.insert(key, value);
            // `insert` keeps the existing spelling on overwrite and uses the
            // canonical spelling on create — exactly the two cases needed.
        }
    }

    let mut sections: Vec<Section> = Vec::new();
    let find_section = |sections: &[Section], lower: &str| -> Option<usize> {
        sections.iter().position(|s| s.lower == lower)
    };
    // Header-less keys land in `[General]` (see doc comment).
    let mut current: Option<usize> = None;
    let ensure_general = |sections: &mut Vec<Section>| -> usize {
        match find_section(sections, "general") {
            Some(i) => i,
            None => {
                sections.push(Section {
                    name: "General".to_string(),
                    lower: "general".to_string(),
                    keys: Vec::new(),
                });
                sections.len() - 1
            }
        }
    };

    let path = game_dir.join("config.ini");
    if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
                let inner = line[1..line.len() - 1].trim();
                if inner.is_empty() {
                    continue;
                }
                let lower = inner.to_ascii_lowercase();
                match find_section(&sections, &lower) {
                    Some(i) => current = Some(i),
                    None => {
                        sections.push(Section {
                            name: inner.to_string(),
                            lower,
                            keys: Vec::new(),
                        });
                        current = Some(sections.len() - 1);
                    }
                }
                continue;
            }
            let Some(eq) = line.find('=') else {
                continue;
            };
            if eq < 1 {
                continue;
            }
            let key = line[..eq].trim_end();
            if key.is_empty() {
                continue;
            }
            // Collapse: value is trimmed, surrounding `"` stripped.
            let value = line[eq + 1..].trim().trim_matches('"');
            let target = match current {
                Some(i) => i,
                None => {
                    let g = ensure_general(&mut sections);
                    current = Some(g);
                    g
                }
            };
            sections[target].insert(key, value);
        }
    }

    let g = ensure_general(&mut sections);
    sections[g].force("game_version", latest);
    sections[g].force("channel", channel.0);
    sections[g].force("sub_channel", channel.1);
    sections[g].force("cps", channel.2);
    sections[g].force("game_biz", biz);
    if sections[g].find_key("sdk_version").is_none() {
        sections[g].keys.push((
            "sdk_version".to_string(),
            "sdk_version".to_string(),
            String::new(),
        ));
    }

    // Collapse `SaveInner`: sections in first-seen order, keys sorted
    // alphabetically (case-insensitive), blank line after each section.
    let mut out = String::new();
    for s in &mut sections {
        s.keys.sort_by(|a, b| a.1.cmp(&b.1));
    }
    for s in &sections {
        out.push_str(&format!("[{}]\n", s.name));
        for (k, _, v) in &s.keys {
            out.push_str(&format!("{k}={v}\n"));
        }
        out.push('\n');
    }
    if dry_run {
        tracing::info!("would write config.ini game_version={}", latest);
        return Ok(());
    }
    let tmp = path.with_extension("ini.girpr_tmp");
    let write = (|| -> Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(out.as_bytes())?;
        if !test_no_fsync() {
            // `write_all` only reaches the page cache. Publishing the rename
            // over a config whose version bump is not durable is exactly the
            // failure this sync exists to prevent, so its error is fatal.
            f.sync_all()
                .with_context(|| format!("sync {}", tmp.display()))?;
        }
        Ok(())
    })();
    if let Err(e) = write {
        match std::fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(cleanup) if cleanup.kind() == std::io::ErrorKind::NotFound => {}
            Err(cleanup) => tracing::debug!(
                path = %tmp.display(),
                error = %cleanup,
                "could not remove the config.ini temp after a failed write"
            ),
        }
        return Err(e);
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // the scan primitive
    // -----------------------------------------------------------------------

    fn hex(data: &[u8]) -> String {
        format!("{:x}", md5::compute(data))
    }

    /// Ranges with **owned** MD5s, so a test can build a list without
    /// borrowing a temporary. [`Ranges::view`] hands out the borrowed form the
    /// scan takes.
    #[derive(Default)]
    struct Ranges {
        owned: Vec<(u64, u64, String)>,
    }

    impl Ranges {
        fn of(list: &[(u64, u64, String)]) -> Self {
            Self {
                owned: list.to_vec(),
            }
        }

        fn view(&self) -> Vec<ScanRange<'_>> {
            self.owned
                .iter()
                .map(|(o, l, m)| (*o, *l, m.as_str()))
                .collect()
        }
    }

    /// A file of four blocks, each with its own content, so a per-range
    /// verdict can be checked one block at a time.
    fn four_blocks() -> Vec<u8> {
        let mut v = Vec::new();
        for (i, b) in b"ABCD".iter().enumerate() {
            v.extend(std::iter::repeat_n(*b, 64 + i));
        }
        v
    }

    #[test]
    fn the_empty_md5_constant_is_the_empty_files_md5() {
        assert_eq!(hex(b""), EMPTY_MD5);
    }

    // -----------------------------------------------------------------------
    // server-controlled path containment
    // -----------------------------------------------------------------------

    /// The spellings a hostile/corrupt server can send. Every one of them must
    /// be refused; `a/b.dat` is the only legitimate shape.
    #[test]
    fn contained_rel_accepts_only_plain_relative_paths() {
        assert!(contained_rel("a/b.dat").is_ok());
        assert!(contained_rel("a\\b.dat").is_ok());
        // Accepted, but returned in the plain `/` form: a leading `.` would
        // make the plan's expected set disagree with the purge walk.
        assert_eq!(
            contained_rel("./a/b.dat").unwrap(),
            PathBuf::from("a/b.dat")
        );
        assert_eq!(
            contained_rel("a/./b.dat").unwrap(),
            PathBuf::from("a/b.dat")
        );

        for bad in [
            "",
            ".",
            "..",
            "../outside.dat",
            "a/../outside.dat",
            "/abs/evil.dat",
            "//server/share/evil.dat",
            "\\\\server\\share\\evil.dat",
            "C:evil.dat",
            "C:/evil.dat",
            "C:\\evil.dat",
            "c:/evil.dat",
            "game/",
            "game\\",
            "game/./",
        ] {
            let got = contained_rel(bad);
            assert!(got.is_err(), "{bad:?} must be refused, got {got:?}");
        }
    }

    /// N-14: Windows name rules are checked on every host, because the install
    /// is a Windows tree. A device name writes to a device (or, for `NUL`,
    /// nowhere) and a trailing dot/space aliases another name; neither is a
    /// file a manifest can legitimately name.
    #[test]
    fn contained_rel_refuses_windows_device_and_alias_names() {
        for bad in [
            "NUL",
            "nul",
            "game/CON",
            "game/con.dat",
            "game/PRN.txt",
            "game/AUX",
            "game/COM1",
            "game/com9.dat",
            "game/lpt1.tar.gz",
            "game/CONIN$",
            "game/CONOUT$",
            "game/CLOCK$",
            "game/trailing.",
            "game/trailing ",
            "dir./file.dat",
            "dir /file.dat",
            "game/a:b",
            "game/a:stream.dat",
        ] {
            let got = contained_rel(bad);
            assert!(got.is_err(), "{bad:?} must be refused, got {got:?}");
        }
        // Near-misses stay legal: only the exact reserved stem is a device.
        for good in [
            "game/conifer.dat",
            "game/nullable.bin",
            "game/console",
            "game/com10.dat",
            "game/lpt",
        ] {
            assert!(contained_rel(good).is_ok(), "{good:?} must be accepted");
        }
    }

    #[test]
    fn contained_join_keeps_a_legitimate_path_under_the_base() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_contained", "join");
        let joined = contained_join(&dir, "game/foo.dat").unwrap();
        assert_eq!(joined, dir.join("game/foo.dat"));
        // The base does not need to exist: `--print-plan` runs before it does.
        let missing = dir.join("not-created-yet");
        let joined = contained_join(&missing, "game/foo.dat").unwrap();
        assert_eq!(joined, missing.join("game/foo.dat"));
        // A traversal attempt cannot reach the sibling file.
        assert!(contained_join(&dir, "../outside.dat").is_err());
        // A trailing separator does not name a file, so it is refused here too.
        assert!(contained_join(&dir, "game/").is_err());
    }

    /// A symlink inside the install that points out of it is an escape even
    /// though the path is lexically relative. Windows needs a privilege to
    /// create one, so the case only runs where it can be made.
    #[cfg(unix)]
    #[test]
    fn contained_join_resolves_a_symlink_and_refuses_the_escape() {
        let outside = girpr_tempdir::TempDir::new("girpr_test_contained", "outside");
        let dir = girpr_tempdir::TempDir::new("girpr_test_contained", "link");
        std::os::unix::fs::symlink(&outside, dir.join("game")).unwrap();
        let got = contained_join(&dir, "game/evil.dat");
        assert!(got.is_err(), "a symlink out of the base must be refused");
        // A symlink that stays inside is still fine.
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("alias")).unwrap();
        assert!(contained_join(&dir, "alias/ok.dat").is_ok());
    }

    /// A component that cannot be **resolved** is not a missing component: the
    /// containment check must fail closed instead of falling back to a lexical
    /// comparison.
    ///
    /// A NUL in a path component is the portable way to make `canonicalize`
    /// return an error that is not `NotFound` (`InvalidInput` on both
    /// platforms). Unix additionally reports `NotADirectory` for a regular
    /// file used as a parent; Windows reports `NotFound` there, which is a
    /// missing path rather than an unresolvable one.
    #[test]
    fn contained_join_fails_closed_when_a_component_cannot_be_resolved() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_contained", "unresolvable");
        let got = contained_join(&dir, "bad\0name");
        assert!(
            got.is_err(),
            "an unresolvable component must be refused, got {got:?}"
        );
        // The error surfaced from the resolver itself, not from the
        // `starts_with` comparison.
        let direct = resolve_existing_prefix(&dir.join("bad\0name"));
        assert!(
            direct.is_err(),
            "a non-NotFound canonicalize error must be propagated"
        );

        #[cfg(unix)]
        {
            std::fs::write(dir.join("blocker"), b"not a directory").unwrap();
            let got = contained_join(&dir, "blocker/child.dat");
            assert!(
                got.is_err(),
                "a file used as a parent is NotADirectory, not missing: {got:?}"
            );
        }
    }

    /// A directory junction inside the install that points out of it is an
    /// escape, exactly like a Unix symlink, so the resolution path needs a
    /// test on the platform whose reparse points it will meet.
    ///
    /// Junctions need no privilege (unlike symlinks), but a host that refuses
    /// to create one cannot confirm the escape here, so the case is skipped
    /// rather than failed. Link support is host-dependent — see
    /// docs/03-repair-rust-implementation-plan.md ("hard-link dedup": NTFS
    /// only, exFAT has no hard links).
    #[cfg(windows)]
    #[test]
    fn contained_join_resolves_a_junction_and_refuses_the_escape() {
        let outside = girpr_tempdir::TempDir::new("girpr_test_contained", "junction_out");
        let dir = girpr_tempdir::TempDir::new("girpr_test_contained", "junction_in");
        let link = dir.join("game");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&*outside)
            .status();
        let made = matches!(status, Ok(s) if s.success()) && link.exists();
        if !made {
            eprintln!("skipping: this host cannot create a directory junction");
            return;
        }
        let got = contained_join(&dir, "game/evil.dat");
        assert!(
            got.is_err(),
            "a junction out of the base must be refused: {got:?}"
        );
        // A junction that stays inside is still fine.
        std::fs::create_dir_all(dir.join("real")).unwrap();
        let alias = dir.join("alias");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&alias)
            .arg(dir.join("real"))
            .status();
        if matches!(status, Ok(s) if s.success()) {
            assert!(contained_join(&dir, "alias/ok.dat").is_ok());
        }
    }

    #[test]
    fn an_intact_file_answers_every_range_correctly() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "intact");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&data[0..64])), (64, 65, hex(&data[64..129]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.file_len, data.len() as u64);
        assert_eq!(scan.file_md5, hex(&data));
        assert_eq!(scan.dest_ok, vec![true, true]);
        assert!(scan.source_ok.is_empty());
    }

    #[test]
    fn one_corrupt_chunk_leaves_the_other_chunk_correct() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "one_bad");
        let good = four_blocks();
        let mut bad = good.clone();
        // Flip a byte inside the second block only.
        bad[70] ^= 0xFF;
        std::fs::write(dir.join("f.dat"), &bad).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&good[0..64])), (64, 65, hex(&good[64..129]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(
            scan.dest_ok,
            vec![true, false],
            "only the block that was actually damaged is wrong"
        );
        assert_ne!(scan.file_md5, hex(&good), "the whole file is wrong too");
    }

    #[test]
    fn a_truncated_file_makes_every_range_past_its_end_wrong() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "truncated");
        let good = four_blocks();
        // Only the first 100 bytes are on disk.
        std::fs::write(dir.join("f.dat"), &good[..100]).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&good[0..64])), (64, 65, hex(&good[64..129]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.file_len, 100);
        assert_eq!(scan.dest_ok, vec![true, false]);
    }

    #[test]
    fn an_absent_file_is_not_an_error_and_nothing_is_correct() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "absent");
        let dest = Ranges::of(&[(0, 64, EMPTY_MD5.to_string())]);
        let source = Ranges::of(&[(0, 64, EMPTY_MD5.to_string())]);
        let scan = scan_file(&dir.join("nope.dat"), &dest.view(), &source.view()).unwrap();
        assert_eq!(scan.file_md5, EMPTY_MD5);
        assert_eq!(scan.file_len, 0);
        assert_eq!(scan.dest_ok, vec![false]);
        assert_eq!(scan.source_ok, vec![false]);
    }

    #[test]
    fn ranges_may_be_given_in_any_order() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "unsorted");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        // Descending by offset: the scan must not care.
        let dest = Ranges::of(&[(64, 65, hex(&data[64..129])), (0, 64, hex(&data[0..64]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![true, true]);
    }

    #[test]
    fn a_range_can_be_both_a_destination_and_a_source() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "both");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&data[0..64]))]);
        let source = Ranges::of(&[(0, 64, hex(&data[0..64]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &source.view()).unwrap();
        assert_eq!(scan.dest_ok, vec![true]);
        assert_eq!(scan.source_ok, vec![true]);
    }

    #[test]
    fn a_source_elsewhere_in_the_file_is_verdicted_on_its_own_bytes() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "source");
        let data = four_blocks();
        // The second block is damaged, so it cannot serve as a source, and the
        // third block is intact and can.
        let mut bad = data.clone();
        bad[70] ^= 0xFF;
        std::fs::write(dir.join("f.dat"), &bad).unwrap();
        let source = Ranges::of(&[
            (64, 65, hex(&data[64..129])),
            (128, 66, hex(&data[128..194])),
        ]);
        let scan = scan_file(&dir.join("f.dat"), &[], &source.view()).unwrap();
        assert_eq!(scan.source_ok, vec![false, true]);
    }

    #[test]
    fn a_range_past_the_end_of_the_file_is_never_correct() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "past_end");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data[..100]).unwrap();
        let dest = Ranges::of(&[(200, 32, hex(&data[200..232]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![false]);
    }

    #[test]
    fn an_empty_expectation_is_never_correct() {
        // An empty MD5 is "unknown", not "matches" — a manifest that carries
        // one must not be read as a free pass.
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "empty_expect");
        std::fs::write(dir.join("f.dat"), b"abc").unwrap();
        let dest = Ranges::of(&[(0, 3, String::new())]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![false]);
    }

    #[test]
    fn the_expected_md5_is_compared_case_insensitively() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "case");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let dest = Ranges::of(&[(0, 64, hex(&data[0..64]).to_ascii_uppercase())]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![true]);
    }

    #[test]
    fn a_file_larger_than_one_block_is_scanned_across_the_boundary() {
        // The sweep is what makes "one read, every answer" true; a range that
        // straddles the 512 KiB block boundary is where an off-by-one would
        // show up.
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "boundary");
        let data: Vec<u8> = (0..(MD5_BUF + 1000)).map(|i| (i % 251) as u8).collect();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let start = MD5_BUF as u64 - 10;
        let dest = Ranges::of(&[(start, 20, hex(&data[start as usize..start as usize + 20]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![true]);
        assert_eq!(scan.file_len, data.len() as u64);
    }

    #[test]
    fn read_range_returns_the_bytes_at_the_offset() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_scan", "read_range");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let mut f = File::open(dir.join("f.dat")).unwrap();
        let got = read_range(&mut f, 64, 8).unwrap();
        assert_eq!(got, &data[64..72]);
    }
    #[test]
    fn version_last_match_wins() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_cfg", "version_last_match_wins");
        std::fs::write(
            dir.join("config.ini"),
            "[General]\ngame_version=4.0.0\ngame_version=5.1.0\n",
        )
        .unwrap();
        assert_eq!(read_game_version(&dir).unwrap().unwrap(), "5.1.0");
    }

    #[test]
    fn ignore_and_blacklist_parse() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lists", "ignore_and_blacklist_parse");
        std::fs::write(
            dir.join("rc"),
            "{\"category\":\"10302\",\"is_delete\":true}\n{}\n",
        )
        .unwrap();
        assert!(
            read_ignore_categories(&dir.join("rc"))
                .unwrap()
                .unwrap()
                .contains("10302")
        );
        std::fs::write(dir.join("bl"), "{\"fileName\":\"a/b.dat\"}\n").unwrap();
        assert!(
            read_blacklist(&dir.join("bl"))
                .unwrap()
                .unwrap()
                .contains("a/b.dat")
        );
    }

    /// V-2: the plan compares the blacklist against the **cleaned** manifest
    /// rel, so the reader must hand it cleaned entries too. A `./`- or
    /// `//`-spelled entry from the launcher is the same file; normalising only
    /// at the plan would make those spellings protect nothing (fail-open). An
    /// entry that cannot be a contained rel is a broken explicit
    /// configuration, not a line to drop silently: the error names it.
    #[test]
    fn blacklist_entries_are_cleaned_and_bad_ones_are_refused() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lists", "blacklist_clean");
        std::fs::write(
            dir.join("bl"),
            "{\"fileName\":\"./game/foo.dat\"}\n\
             {\"fileName\":\"game//bar.dat\"}\n\
             {\"fileName\":\"game\\\\baz.dat\"}\n",
        )
        .unwrap();
        assert_eq!(
            read_blacklist(&dir.join("bl")).unwrap().unwrap(),
            HashSet::from([
                "game/foo.dat".to_string(),
                "game/bar.dat".to_string(),
                "game/baz.dat".to_string(),
            ])
        );

        std::fs::write(dir.join("bl"), "{\"fileName\":\"../outside.dat\"}\n").unwrap();
        let err = read_blacklist(&dir.join("bl")).expect_err("an escape must be refused");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("blacklist entry") && msg.contains("../outside.dat"),
            "the refusal names the entry: {msg}"
        );
    }

    /// R-6: absent is a documented default (`None`); an unreadable file is an
    /// error that names the file, never an empty set that silently changes
    /// what the run repairs or ignores. A directory at the path is the
    /// portable way to force something other than `NotFound`.
    #[test]
    fn unreadable_config_lists_are_errors_and_absent_ones_are_none() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lists", "unreadable");
        for name in ["rc", "bl", "config.ini"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }

        let err = read_game_version(&dir).expect_err("a directory is not config.ini");
        assert!(format!("{err:#}").contains("config.ini"), "{err:#}");
        let err = read_ignore_categories(&dir.join("rc")).expect_err("a directory is not a list");
        assert!(format!("{err:#}").contains("rc"), "{err:#}");
        let err = read_blacklist(&dir.join("bl")).expect_err("a directory is not a list");
        assert!(format!("{err:#}").contains("bl"), "{err:#}");

        let empty = girpr_tempdir::TempDir::new("girpr_test_lists", "absent");
        assert!(read_game_version(&empty).unwrap().is_none());
        assert!(read_ignore_categories(&empty.join("rc")).unwrap().is_none());
        assert!(read_blacklist(&empty.join("bl")).unwrap().is_none());
    }

    /// T-2: `file_len` used `.ok()`, so every `metadata` error looked absent.
    /// The classifier is split out so an injected ACL-style error is pinned
    /// without a permission fixture (S-3's shape), and a NUL path covers the
    /// real syscall boundary on both platforms.
    #[test]
    fn file_len_separates_present_absent_and_unreadable() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_len", "states");
        std::fs::write(dir.join("f"), b"12345").unwrap();
        assert!(matches!(file_len(&dir.join("f")), FileLen::Present(5)));
        assert!(matches!(file_len(&dir.join("missing")), FileLen::Absent));

        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert!(matches!(classify_len(Err(denied)), FileLen::Unreadable(_)));
        let gone = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        assert!(matches!(classify_len(Err(gone)), FileLen::Absent));

        // A NUL cannot be statted (`InvalidInput` on Unix and Windows), so it
        // reaches the unreadable arm without a deny ACL.
        let err = match file_len(Path::new("bad\0name")) {
            FileLen::Unreadable(e) => e,
            other => panic!("a NUL path is not absent: {other:?}"),
        };
        assert_ne!(err.kind(), std::io::ErrorKind::NotFound);

        #[cfg(unix)]
        {
            // Unix: a regular file used as a parent is `ENOTDIR`, which is
            // unreadable, not missing. Windows reports `NotFound` for the
            // same shape, so this arm is platform-specific.
            std::fs::write(dir.join("blocker"), b"x").unwrap();
            match file_len(&dir.join("blocker/child.dat")) {
                FileLen::Unreadable(e) => {
                    assert_eq!(e.kind(), std::io::ErrorKind::NotADirectory);
                }
                other => panic!("ENOTDIR is unreadable, not absent: {other:?}"),
            }
        }
    }

    /// A BOM is not whitespace: one leading BOM is stripped; a mixed file
    /// warns with the path and line number and keeps the well-formed entry.
    #[test]
    fn config_lists_strip_a_bom_and_warn_per_unparseable_line() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lists", "bom_mixed");
        std::fs::write(
            dir.join("bl"),
            "\u{feff}{\"fileName\":\"a/b.dat\"}\n{\"fileName\":\"c/d",
        )
        .unwrap();

        let mut blacklist = HashSet::new();
        let logs = crate::repair::test_support::logs_of(|| {
            blacklist = read_blacklist(&dir.join("bl")).unwrap().unwrap();
        });
        assert_eq!(blacklist, HashSet::from(["a/b.dat".to_string()]));
        assert!(
            logs.contains("invalid entry in the blacklist list"),
            "the dropped line must be logged: {logs}"
        );
        assert!(logs.contains("line=2"), "the log names the line: {logs}");
        assert!(
            logs.contains(&dir.join("bl").display().to_string()),
            "the log names the file: {logs}"
        );

        std::fs::write(
            dir.join("rc"),
            "\u{feff}{\"category\":\"10302\",\"is_delete\":true}\n{\"category\":",
        )
        .unwrap();
        let mut ignore = HashSet::new();
        let logs = crate::repair::test_support::logs_of(|| {
            ignore = read_ignore_categories(&dir.join("rc")).unwrap().unwrap();
        });
        assert_eq!(ignore, HashSet::from(["10302".to_string()]));
        assert!(
            logs.contains("invalid entry in the ignore categories list") && logs.contains("line=2"),
            "the dropped line must be logged with its line: {logs}"
        );
    }

    /// E-1: a line that is valid JSON but whose entry field is missing or
    /// wrong-typed is a malformed entry, not a silent no-op. The two shapes
    /// from the review are pinned here; a mixed file keeps the well-formed
    /// entries and warns per bad line, and an all-malformed file is refused
    /// instead of reading as an empty selection.
    #[test]
    fn malformed_config_list_entries_are_refused_not_dropped() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lists", "malformed_entries");

        // The first review shape: `{"fileName":123}` protects nothing. Alone
        // it is a refused file, naming the path, the line and the field.
        std::fs::write(dir.join("bl"), "{\"fileName\":123}\n").unwrap();
        let mut err = None;
        let logs = crate::repair::test_support::logs_of(|| {
            err = Some(read_blacklist(&dir.join("bl")).expect_err("no usable entry"));
        });
        let msg = format!("{:#}", err.expect("the probe ran"));
        assert!(
            msg.contains("blacklist")
                && msg.contains(&dir.join("bl").display().to_string())
                && msg.contains("line 1")
                && msg.contains("fileName"),
            "the refusal names the file, line and field: {msg}"
        );
        assert!(
            logs.contains("invalid entry in the blacklist list")
                && logs.contains("line=1")
                && logs.contains("a number"),
            "the malformed entry is warned with its line and type: {logs}"
        );

        // Mixed: the well-formed entry survives, the bad line is warned.
        std::fs::write(
            dir.join("bl"),
            "{\"fileName\":\"a/b.dat\"}\n{\"fileName\":null}\n",
        )
        .unwrap();
        let mut got = None;
        let logs = crate::repair::test_support::logs_of(|| {
            got = Some(read_blacklist(&dir.join("bl")).unwrap().unwrap());
        });
        assert_eq!(got.unwrap(), HashSet::from(["a/b.dat".to_string()]));
        assert!(
            logs.contains("invalid entry in the blacklist list")
                && logs.contains("line=2")
                && logs.contains("null"),
            "the bad line is warned, the good one kept: {logs}"
        );

        // The second review shape: `is_delete` as the string `"true"` is not
        // a boolean and must not silently ignore a category.
        std::fs::write(
            dir.join("rc"),
            "{\"category\":\"10302\",\"is_delete\":\"true\"}\n",
        )
        .unwrap();
        let mut err = None;
        let logs = crate::repair::test_support::logs_of(|| {
            err = Some(read_ignore_categories(&dir.join("rc")).expect_err("no usable entry"));
        });
        let msg = format!("{:#}", err.expect("the probe ran"));
        assert!(
            msg.contains("ignore categories")
                && msg.contains(&dir.join("rc").display().to_string())
                && msg.contains("line 1")
                && msg.contains("is_delete"),
            "the refusal names the file, line and field: {msg}"
        );
        assert!(
            logs.contains("invalid entry in the ignore categories list") && logs.contains("line=1"),
            "the malformed entry is warned with its line: {logs}"
        );

        // `is_delete: true` without a string `category` is malformed too.
        std::fs::write(dir.join("rc"), "{\"is_delete\":true}\n").unwrap();
        let err = read_ignore_categories(&dir.join("rc")).expect_err("missing category");
        assert!(
            format!("{err:#}").contains("category"),
            "the refusal names the missing field: {err:#}"
        );

        // A no-op entry (`is_delete` absent or false) stays a valid empty
        // selection, as does the entryless `{}`.
        std::fs::write(dir.join("rc"), "{\"category\":\"10302\"}\n{}\n").unwrap();
        assert!(
            read_ignore_categories(&dir.join("rc"))
                .unwrap()
                .unwrap()
                .is_empty()
        );

        // All-malformed: a parseable but wrong-typed line is refused even
        // though every line was valid JSON.
        std::fs::write(
            dir.join("bl"),
            "{\"fileName\":123}\n{\"FileName\":\"x/y.dat\"}\n",
        )
        .unwrap();
        let err = read_blacklist(&dir.join("bl")).expect_err("no usable entry");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("blacklist")
                && msg.contains(&dir.join("bl").display().to_string())
                && msg.contains("line 1"),
            "the refusal names the file and first bad line: {msg}"
        );
    }

    /// T-1: a list whose only text is unparseable is an error naming the path
    /// and line, not an empty set that silently disables the configuration.
    /// A valid but entryless `{}` and a blank file keep their old meaning.
    #[test]
    fn an_all_malformed_config_list_is_refused() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_lists", "all_malformed");
        std::fs::write(dir.join("rc"), "not json\n{\"category\":\n").unwrap();
        // Keep a subscriber installed while the warning callsite is hit: the
        // mixed-file test asserts the log, and tracing's callsite interest is
        // global, so a concurrent subjective hit from a thread without one
        // must not cache the callsite as disabled.
        let mut rc_err = None;
        crate::repair::test_support::logs_of(|| {
            rc_err = Some(read_ignore_categories(&dir.join("rc")).expect_err("no usable entry"));
        });
        let msg = format!("{:#}", rc_err.expect("the probe ran"));
        assert!(
            msg.contains("ignore categories")
                && msg.contains(&dir.join("rc").display().to_string())
                && msg.contains("line 1"),
            "the refusal names the file and first bad line: {msg}"
        );

        std::fs::write(dir.join("bl"), "{\"fileName\":\n").unwrap();
        let mut bl_err = None;
        crate::repair::test_support::logs_of(|| {
            bl_err = Some(read_blacklist(&dir.join("bl")).expect_err("no usable entry"));
        });
        let msg = format!("{:#}", bl_err.expect("the probe ran"));
        assert!(
            msg.contains("blacklist")
                && msg.contains(&dir.join("bl").display().to_string())
                && msg.contains("line 1"),
            "the refusal names the file and line: {msg}"
        );

        std::fs::write(dir.join("rc"), "{}\n\n").unwrap();
        assert!(
            read_ignore_categories(&dir.join("rc"))
                .unwrap()
                .unwrap()
                .is_empty(),
            "a valid but entryless line stays an empty selection"
        );
        std::fs::write(dir.join("rc"), "\u{feff}").unwrap();
        assert!(
            read_ignore_categories(&dir.join("rc"))
                .unwrap()
                .unwrap()
                .is_empty(),
            "a BOM-only file is blank after the BOM is stripped"
        );
        std::fs::write(dir.join("bl"), "").unwrap();
        assert!(read_blacklist(&dir.join("bl")).unwrap().unwrap().is_empty());
    }

    #[test]
    fn config_bump_forces_target_keys_preserves_rest_and_is_idempotent() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_cfg", "bump_idempotent");
        std::fs::write(
            dir.join("config.ini"),
            "[General]\ngame_version=4.0.0\nfps=120\nsdk_version=9.9.9\nchannel=5\n",
        )
        .unwrap();
        write_config_ini(
            &dir,
            "5.1.0",
            "hk4e_global",
            ("1", "0", "hyp_hoyoverse"),
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        assert!(text.starts_with("[General]\n"), "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.contains(&"game_version=5.1.0"));
        assert!(!lines.contains(&"game_version=4.0.0"));
        assert!(lines.contains(&"fps=120"));
        // Collapse parity: existing sdk_version is preserved, never clobbered.
        assert!(lines.contains(&"sdk_version=9.9.9"));
        // game_biz stays forced (deliberate Starward carryover).
        assert!(lines.contains(&"game_biz=hk4e_global"));
        assert!(lines.contains(&"channel=1"));
        assert!(lines.contains(&"sub_channel=0"));
        assert!(lines.contains(&"cps=hyp_hoyoverse"));
        assert_eq!(read_game_version(&dir).unwrap().unwrap(), "5.1.0");
        // Keys within the section are sorted (Collapse SaveInner parity).
        let sorted = lines
            .iter()
            .filter(|l| l.contains('='))
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let mut expected = sorted.clone();
        expected.sort_by_key(|a| a.to_ascii_lowercase());
        assert_eq!(sorted, expected, "{text}");
        // Rerun is byte-identical (no duplicate keys, stable order).
        let snapshot = text.clone();
        write_config_ini(
            &dir,
            "5.1.0",
            "hk4e_global",
            ("1", "0", "hyp_hoyoverse"),
            false,
        )
        .unwrap();
        let text2 = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        assert_eq!(text2, snapshot, "rerun must be byte-identical");
        for key in [
            "game_version",
            "fps",
            "sdk_version",
            "sub_channel",
            "cps",
            "game_biz",
            "channel",
        ] {
            let count = text2
                .lines()
                .filter(|l| l.to_ascii_lowercase().starts_with(&format!("{key}=")))
                .count();
            assert_eq!(count, 1, "{key} must appear exactly once");
        }
    }

    /// Collapse parity: other `[sections]` survive in place (never flattened
    /// into `[General]`), duplicate sections merge, forced keys match
    /// case-insensitively, comments are dropped, passthrough keys
    /// (`uapc`/`wpf_version`/`predownload`/`plugin_*`) are preserved.
    #[test]
    fn config_bump_preserves_sections_and_merges_duplicates() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_cfg", "sections");
        std::fs::write(
            dir.join("config.ini"),
            "; leading comment\n\
             [General]\n\
             game_version=4.0.0\n\
             FPS=120\n\
             Channel=5\n\
             uapc={\"x\":1}\n\
             wpf_version=1.2\n\
             predownload=a,b,c\n\
             plugin_sdk_version=9.0\n\
             # hash comment\n\
             \n\
             [Launcher]\n\
             log_level=3\n\
             ; inner comment\n\
             [launcher]\n\
             game_install_path=C:/games\n\
             [General]\n\
             cps=old\n",
        )
        .unwrap();
        write_config_ini(
            &dir,
            "5.1.0",
            "hk4e_global",
            ("1", "0", "hyp_hoyoverse"),
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        // Sections survive: [General] + [Launcher], nothing flattened.
        assert!(
            !text.contains("Launcher:"),
            "must not flatten sections:\n{text}"
        );
        assert!(
            !text.contains("comment"),
            "comments must be dropped:\n{text}"
        );
        assert_eq!(text.matches("[General]").count(), 1, "{text}");
        assert_eq!(text.matches("[Launcher]").count(), 1, "{text}");
        // Duplicate [launcher] merged into first-seen [Launcher].
        assert!(text.contains("log_level=3"), "{text}");
        assert!(text.contains("game_install_path=C:/games"), "{text}");
        // Forced keys (case-insensitive match, value forced).
        let lower = text.to_ascii_lowercase();
        assert!(lower.contains("\ngame_version=5.1.0\n"), "{text}");
        assert!(!lower.contains("game_version=4.0.0"), "{text}");
        assert!(lower.contains("\nchannel=1\n"), "{text}");
        assert!(!lower.contains("\nchannel=5\n"), "{text}");
        assert!(lower.contains("\ncps=hyp_hoyoverse\n"), "{text}");
        assert!(!lower.contains("\ncps=old\n"), "{text}");
        assert!(lower.contains("\nsub_channel=0\n"), "{text}");
        assert!(lower.contains("\ngame_biz=hk4e_global\n"), "{text}");
        // Passthrough keys untouched (incl. distinct-case FPS).
        assert!(lower.contains("\nfps=120\n"), "{text}");
        assert!(text.contains("uapc={\"x\":1}"), "{text}");
        assert!(lower.contains("\nwpf_version=1.2\n"), "{text}");
        assert!(lower.contains("\npredownload=a,b,c\n"), "{text}");
        assert!(lower.contains("\nplugin_sdk_version=9.0\n"), "{text}");
        // sdk_version created empty when missing.
        assert!(lower.contains("\nsdk_version=\n"), "{text}");
        assert_eq!(read_game_version(&dir).unwrap().unwrap(), "5.1.0");
        // Rerun byte-identical.
        let snapshot = text.clone();
        write_config_ini(
            &dir,
            "5.1.0",
            "hk4e_global",
            ("1", "0", "hyp_hoyoverse"),
            false,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("config.ini")).unwrap(),
            snapshot
        );
    }

    /// Missing file is created; header-less keys land in `[General]`;
    /// missing `sdk_version` defaults to empty (Collapse `DefaultIniVersion`).
    #[test]
    fn config_bump_creates_missing_and_handles_headerless() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_cfg", "headerless");
        write_config_ini(&dir, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo"), false).unwrap();
        let text = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        assert!(text.starts_with("[General]\n"), "{text}");
        assert!(text.contains("game_version=5.1.0"), "{text}");
        assert!(text.contains("cps=hyp_mihoyo"), "{text}");
        assert!(text.contains("game_biz=hk4e_cn"), "{text}");
        assert!(text.contains("sdk_version=\n"), "{text}");

        std::fs::write(dir.join("config.ini"), "fps=60\ngame_version=1.0.0\n").unwrap();
        write_config_ini(&dir, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo"), false).unwrap();
        let text2 = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        assert!(text2.contains("fps=60"), "{text2}");
        assert!(text2.contains("game_version=5.1.0"), "{text2}");
        assert_eq!(text2.matches("[General]").count(), 1, "{text2}");
    }
}
