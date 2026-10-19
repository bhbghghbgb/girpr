use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

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
pub fn scan_file(
    path: &Path,
    dest: &[ScanRange<'_>],
    source: &[ScanRange<'_>],
) -> Result<Scan> {
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
pub fn scan_reader(
    f: &mut File,
    dest: &[ScanRange<'_>],
    source: &[ScanRange<'_>],
) -> Result<Scan> {
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

pub fn file_len(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}

/// Turn a `/`-separated manifest rel path into a native one.
///
/// Manifests always use `/`; every comparison in this crate (plan, reuse map,
/// purge classification) is done on the `/` form, and the conversion happens
/// only at the filesystem boundary.
pub fn normalize_rel(rel: &str) -> String {
    rel.replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR)
}

/// Parse `config.ini` last `game_version=` match.
/// NOTE(starward-parity): last-match-wins mirrors `GetLocalGameVersionAsync`
/// (`matches[^1]` over `game_version=(.+)`); missing file means fresh install.
/// Do not "fix" to first-match.
/// See <https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L47-L70>
pub fn read_game_version(game_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(game_dir.join("config.ini")).ok()?;
    let re = regex::Regex::new(r"(?m)^game_version\s*=\s*(.+?)\s*$").ok()?;
    re.captures_iter(&text)
        .last()
        .map(|c| c[1].trim().to_string())
}

/// Read res_category ignore file: JSON-lines {"category":"...","is_delete":true}.
pub fn read_ignore_categories(path: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return set;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
            && v.get("is_delete")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
            && let Some(c) = v.get("category").and_then(|c| c.as_str())
        {
            set.insert(c.to_string());
        }
    }
    set
}

/// Read blacklist file: JSON-lines {"fileName":"..."}.
pub fn read_blacklist(path: &Path) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return set;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
            && let Some(n) = v.get("fileName").and_then(|n| n.as_str())
        {
            set.insert(n.replace('\\', "/"));
        }
    }
    set
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
    {
        let mut f = File::create(&tmp)?;
        f.write_all(out.as_bytes())?;
        f.sync_all().ok();
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

    #[test]
    fn an_intact_file_answers_every_range_correctly() {
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "intact");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "one_bad");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "truncated");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "absent");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "unsorted");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        // Descending by offset: the scan must not care.
        let dest = Ranges::of(&[(64, 65, hex(&data[64..129])), (0, 64, hex(&data[0..64]))]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![true, true]);
    }

    #[test]
    fn a_range_can_be_both_a_destination_and_a_source() {
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "both");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "source");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "past_end");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "empty_expect");
        std::fs::write(dir.join("f.dat"), b"abc").unwrap();
        let dest = Ranges::of(&[(0, 3, String::new())]);
        let scan = scan_file(&dir.join("f.dat"), &dest.view(), &[]).unwrap();
        assert_eq!(scan.dest_ok, vec![false]);
    }

    #[test]
    fn the_expected_md5_is_compared_case_insensitively() {
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "case");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "boundary");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_scan", "read_range");
        let data = four_blocks();
        std::fs::write(dir.join("f.dat"), &data).unwrap();
        let mut f = File::open(dir.join("f.dat")).unwrap();
        let got = read_range(&mut f, 64, 8).unwrap();
        assert_eq!(got, &data[64..72]);
    }
    #[test]
    fn version_last_match_wins() {
        let dir = crate::testtemp::TempDir::new("girpr_test_cfg", "version_last_match_wins");
        std::fs::write(
            dir.join("config.ini"),
            "[General]\ngame_version=4.0.0\ngame_version=5.1.0\n",
        )
        .unwrap();
        assert_eq!(read_game_version(&dir).unwrap(), "5.1.0");
    }

    #[test]
    fn ignore_and_blacklist_parse() {
        let dir = crate::testtemp::TempDir::new("girpr_test_lists", "ignore_and_blacklist_parse");
        std::fs::write(
            dir.join("rc"),
            "{\"category\":\"10302\",\"is_delete\":true}\n{}\n",
        )
        .unwrap();
        assert!(read_ignore_categories(&dir.join("rc")).contains("10302"));
        std::fs::write(dir.join("bl"), "{\"fileName\":\"a/b.dat\"}\n").unwrap();
        assert!(read_blacklist(&dir.join("bl")).contains("a/b.dat"));
    }

    #[test]
    fn config_bump_forces_target_keys_preserves_rest_and_is_idempotent() {
        let dir = crate::testtemp::TempDir::new("girpr_test_cfg", "bump_idempotent");
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
        assert_eq!(read_game_version(&dir).unwrap(), "5.1.0");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_cfg", "sections");
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
        assert_eq!(read_game_version(&dir).unwrap(), "5.1.0");
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
        let dir = crate::testtemp::TempDir::new("girpr_test_cfg", "headerless");
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
