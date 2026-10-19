use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

const MD5_BUF: usize = 512 * 1024;

/// The MD5 of the empty byte string — what a **missing** file hashes to.
///
/// The scan reports "nothing is correct" for a file it cannot read, rather
/// than failing: a missing file is a normal state for a repair, not an error.
pub const EMPTY_MD5: &str = "d41d8cd98f00b204e9800998ecf8427e";

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

/// What one read pass over a file answered.
///
/// The scan is the primitive the in-place repair is built on: **one** read
/// answers three questions at once — the whole-file MD5 (the skip check), the
/// per-destination verdict ("is this chunk already correct *where it
/// belongs*?") and the per-source verdict ("is this reuse source intact?").
/// The old code answered the first with one read and the third with another
/// one per candidate; here they all come out of a single pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scan {
    /// The whole-file MD5 of the bytes that are on disk. For a **missing**
    /// file this is [`EMPTY_MD5`].
    pub file_md5: String,
    /// Per destination range: is it already correct where it belongs?
    pub dest_ok: Vec<bool>,
    /// Per candidate source range: is the reuse source intact?
    pub source_ok: Vec<bool>,
}

/// One range the scan has to hash: `(offset, length, expected_md5)`.
pub type Range<'a> = (u64, u64, &'a str);

/// Hash a file once and answer every question the repair asks about it.
///
/// `dest` are the ranges the manifest's chunks occupy (their expected MD5 is
/// the chunk's own); `source` are the ranges a reuse candidate would be read
/// from (their expected MD5 is the chunk's, again — a source is only usable
/// if the bytes there hash to what the chunk wants).
///
/// The two lists may overlap, and the ranges inside them need not be sorted:
/// the ranges are collected, de-duplicated and sorted once, and the verdicts
/// are mapped back to the caller's indices.
///
/// **A missing file is not an error.** It answers "nothing is correct" —
/// [`EMPTY_MD5`] and every verdict `false` — because a missing file is the
/// normal state of a file that has to be fetched whole. A file that is
/// *shorter* than the ranges asked about keeps its own MD5 (over the bytes
/// that are there) and reports the ranges it does not cover as not correct.
pub fn scan_file(path: &Path, dest: &[Range], source: &[Range]) -> Result<Scan> {
    // Collect the distinct ranges, remembering which caller entries point at
    // each one. De-duplication is by (offset, length, md5): the same bytes
    // with the same expectation are hashed once, and the same bytes with two
    // different expectations are hashed twice (they are two different claims
    // about the same range, and both have to be answered).
    let mut specs: Vec<Spec> = Vec::new();
    let mut dest_ref: Vec<usize> = Vec::with_capacity(dest.len());
    let mut source_ref: Vec<usize> = Vec::with_capacity(source.len());
    for (offset, len, md5) in dest {
        dest_ref.push(intern(&mut specs, *offset, *len, md5));
    }
    for (offset, len, md5) in source {
        source_ref.push(intern(&mut specs, *offset, *len, md5));
    }
    // Sorted by offset so the read pass can stop scanning ranges once it is
    // past them. The sort permutes the list, so the caller's indices are
    // remapped through it — an index that is not remapped answers a
    // *different* range's question.
    specs.sort_by_key(|s| (s.offset, s.len));
    let mut remap = vec![0usize; specs.len()];
    for (new_index, s) in specs.iter().enumerate() {
        remap[s.id] = new_index;
    }

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Scan {
                file_md5: EMPTY_MD5.to_string(),
                dest_ok: vec![false; dest.len()],
                source_ok: vec![false; source.len()],
            });
        }
        Err(e) => return Err(e).with_context(|| format!("open {}", path.display())),
    };

    let mut whole = md5::Context::new();
    let mut ctxs: Vec<md5::Context> = specs.iter().map(|_| md5::Context::new()).collect();
    let mut fed: Vec<u64> = vec![0; specs.len()];
    let mut buf = vec![0u8; MD5_BUF];
    let mut pos = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        whole.consume(&buf[..n]);
        let block_start = pos;
        let block_end = pos + n as u64;
        for (i, s) in specs.iter().enumerate() {
            let r_end = s.offset.saturating_add(s.len);
            if r_end <= block_start {
                continue; // the range ended before this block
            }
            if s.offset >= block_end {
                break; // sorted by offset: no later range can overlap either
            }
            let from = s.offset.max(block_start);
            let to = r_end.min(block_end);
            if to > from {
                let a = (from - block_start) as usize;
                let b = (to - block_start) as usize;
                ctxs[i].consume(&buf[a..b]);
                fed[i] += to - from;
            }
        }
        pos = block_end;
    }

    let ok: Vec<bool> = ctxs
        .into_iter()
        .zip(specs.iter())
        .enumerate()
        .map(|(i, (ctx, s))| {
            // A range that the file does not fully cover cannot be correct,
            // whatever its bytes hash to.
            fed[i] == s.len && format!("{:x}", ctx.compute()).eq_ignore_ascii_case(&s.md5)
        })
        .collect();

    Ok(Scan {
        file_md5: format!("{:x}", whole.compute()),
        dest_ok: dest_ref.iter().map(|i| ok[remap[*i]]).collect(),
        source_ok: source_ref.iter().map(|i| ok[remap[*i]]).collect(),
    })
}

/// One distinct range to hash, and the MD5 it is expected to have.
struct Spec {
    offset: u64,
    len: u64,
    /// Lowercased, so the comparison is a plain `==`.
    md5: String,
    /// The index this spec had before the list was sorted — the caller's
    /// indices are remapped through it.
    id: usize,
}

/// Intern a range in the spec list, returning its index. An existing spec
/// with the same (offset, length, md5) is reused.
fn intern(specs: &mut Vec<Spec>, offset: u64, len: u64, md5: &str) -> usize {
    let md5 = md5.to_ascii_lowercase();
    if let Some(i) = specs
        .iter()
        .position(|s| s.offset == offset && s.len == len && s.md5 == md5)
    {
        return i;
    }
    specs.push(Spec {
        offset,
        len,
        md5,
        id: specs.len(),
    });
    specs.len() - 1
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

    /// The scan is the primitive the in-place repair is built on, so its
    /// answers are pinned one case at a time.
    mod scan {
        use super::*;

        fn md5hex(data: &[u8]) -> String {
            format!("{:x}", md5::compute(data))
        }

        fn write(dir: &crate::testtemp::TempDir, name: &str, data: &[u8]) -> std::path::PathBuf {
            let p = dir.join(name);
            std::fs::write(&p, data).expect("write fixture");
            p
        }

        #[test]
        fn an_intact_file_answers_true_for_every_destination() {
            let dir = crate::testtemp::TempDir::new("girpr_test_scan", "intact");
            let data = b"AAAABBBBCCCC";
            let path = write(&dir, "f.dat", data);
            let scan = scan_file(
                &path,
                &[(0, 4, &md5hex(b"AAAA")), (4, 4, &md5hex(b"BBBB"))],
                &[],
            )
            .unwrap();
            assert_eq!(scan.dest_ok, vec![true, true]);
            assert_eq!(scan.file_md5, md5hex(data));
        }

        #[test]
        fn one_corrupt_chunk_answers_false_for_that_chunk_only() {
            let dir = crate::testtemp::TempDir::new("girpr_test_scan", "corrupt");
            let path = write(&dir, "f.dat", b"AAAABBBBCCCC");
            let scan = scan_file(
                &path,
                &[(0, 4, &md5hex(b"AAAA")), (4, 4, &md5hex(b"XXXX"))],
                &[],
            )
            .unwrap();
            assert_eq!(
                scan.dest_ok,
                vec![true, false],
                "only the chunk whose bytes differ is wrong"
            );
        }

        #[test]
        fn a_truncated_file_answers_false_for_what_it_does_not_cover() {
            let dir = crate::testtemp::TempDir::new("girpr_test_scan", "truncated");
            let path = write(&dir, "f.dat", b"AAAA");
            let scan = scan_file(
                &path,
                &[(0, 4, &md5hex(b"AAAA")), (4, 4, &md5hex(b"BBBB"))],
                &[],
            )
            .unwrap();
            assert_eq!(scan.dest_ok, vec![true, false]);
            assert_eq!(
                scan.file_md5,
                md5hex(b"AAAA"),
                "a short file reports its own MD5, not the empty one"
            );
        }

        #[test]
        fn an_absent_file_answers_nothing_is_correct_and_is_not_an_error() {
            let dir = crate::testtemp::TempDir::new("girpr_test_scan", "absent");
            let path = dir.join("nope.dat");
            let scan = scan_file(
                &path,
                &[(0, 4, &md5hex(b"AAAA"))],
                &[(0, 4, &md5hex(b"AAAA"))],
            )
            .unwrap();
            assert_eq!(scan.file_md5, EMPTY_MD5);
            assert_eq!(scan.dest_ok, vec![false]);
            assert_eq!(scan.source_ok, vec![false]);
        }

        #[test]
        fn the_verdicts_follow_the_callers_indices_not_the_ranges_order() {
            let dir = crate::testtemp::TempDir::new("girpr_test_scan", "unsorted");
            let path = write(&dir, "f.dat", b"AAAABBBBCCCC");
            // Deliberately descending, and with the second range asked for
            // first: the answer must still be per index.
            let scan = scan_file(
                &path,
                &[(8, 4, &md5hex(b"CCCC")), (0, 4, &md5hex(b"AAAA"))],
                &[(4, 4, &md5hex(b"BBBB"))],
            )
            .unwrap();
            assert_eq!(scan.dest_ok, vec![true, true]);
            assert_eq!(scan.source_ok, vec![true]);
        }

        #[test]
        fn a_range_that_is_also_a_destination_gets_one_hash_and_two_verdicts() {
            let dir = crate::testtemp::TempDir::new("girpr_test_scan", "both");
            let path = write(&dir, "f.dat", b"AAAABBBB");
            let scan = scan_file(
                &path,
                &[(0, 4, &md5hex(b"AAAA"))],
                &[(0, 4, &md5hex(b"AAAA"))],
            )
            .unwrap();
            assert_eq!(scan.dest_ok, vec![true]);
            assert_eq!(scan.source_ok, vec![true]);
            // The same range asked for with a different expectation is a
            // different claim about the same bytes, and is answered on its own.
            let scan = scan_file(
                &path,
                &[(0, 4, &md5hex(b"AAAA"))],
                &[(0, 4, &md5hex(b"ZZZZ"))],
            )
            .unwrap();
            assert_eq!(scan.dest_ok, vec![true]);
            assert_eq!(scan.source_ok, vec![false]);
        }

        #[test]
        fn a_source_past_the_end_of_the_file_is_not_intact() {
            let dir = crate::testtemp::TempDir::new("girpr_test_scan", "past_end");
            let path = write(&dir, "f.dat", b"AAAA");
            let scan = scan_file(&path, &[], &[(4, 4, &md5hex(b"BBBB"))]).unwrap();
            assert_eq!(scan.source_ok, vec![false]);
        }

        /// The verdicts follow the caller's indices even when the ranges are
        /// sorted differently from the order they were asked for — and even
        /// when two ranges are the same bytes with different expectations.
        #[test]
        fn a_range_asked_for_with_the_wrong_expectation_is_not_correct() {
            let dir = crate::testtemp::TempDir::new("girpr_test_scan", "wrong_expectation");
            let path = write(&dir, "f.dat", b"AAAABBBBCCCC");
            let scan = scan_file(
                &path,
                // [4,8) holds BBBB, not AAAA: the expectation is not met.
                &[(4, 4, &md5hex(b"AAAA"))],
                // [0,4) does hold AAAA.
                &[(0, 4, &md5hex(b"AAAA"))],
            )
            .unwrap();
            assert_eq!(scan.dest_ok, vec![false]);
            assert_eq!(scan.source_ok, vec![true]);
        }
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
