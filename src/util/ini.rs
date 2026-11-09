use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::path::Path;

use anyhow::{Context, Result};

use super::atomic::write_atomic;
use super::path::clean_rel;

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
    Ok(text.lines().filter_map(game_version_line).next_back())
}

/// The value of one `game_version` line, if it has the documented shape.
///
/// The key must start the line exactly (no leading whitespace) and be exactly
/// `game_version`; optional whitespace may surround the first `=`, and the
/// value is everything after it, trimmed of surrounding whitespace. A later
/// `=` stays in the value. A line with nothing after `=` is not a match; the
/// caller keeps the **last** match.
fn game_version_line(line: &str) -> Option<String> {
    let (key, rest) = split_first_eq(line)?;
    if key.trim_end() != "game_version" {
        return None;
    }
    if rest.is_empty() {
        return None;
    }
    Some(rest.trim().to_string())
}

/// Split an INI line at its **first** `=`.
///
/// This is the single spelling of the shared boundary rule: both the config
/// loader and the `game_version` scan treat the first `=` as the key/value
/// boundary, so a later `=` belongs to the value. Only that boundary is shared;
/// the callers keep their own key anchoring, trimming and empty-value rules,
/// which deliberately differ.
fn split_first_eq(line: &str) -> Option<(&str, &str)> {
    line.split_once('=')
}

/// Read res_category ignore file: JSON-lines {"category":"...","is_delete":true}.
///
/// `Ok(None)` means the file is not there (the launcher did not configure
/// it); `Err` means it is there but could not be read **or** its text could
/// not be parsed into any entry, which must not become an empty ignore set
/// without a trace. One leading UTF-8 BOM is stripped; a mixed file warns per
/// bad line and keeps the well-formed entries (see `parse_config_list`).
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

/// The refusal for a game dir that cannot hold `config.ini`.
///
/// A missing path, a path that exists as something other than a directory, and
/// a link whose target is missing are different mistakes: the message must
/// name the one that happened instead of calling every case "does not exist".
///
/// `Ok(Some(_))` is that refusal. `Ok(None)` means the path could not be
/// classified without swallowing a different error (for example the target of
/// an existing link could not be resolved); the caller then surfaces the
/// original I/O error rather than inventing a refusal. `Err` is a failure to
/// inspect the path itself: the `NotFound` caller surfaces it, with the path
/// named, because calling an unreadable path "missing" would be a guess. The
/// `NotADirectory` caller keeps its own error instead: that kind already names
/// the real condition, so nothing is being guessed there.
///
/// A Windows junction is a reparse point, but Rust's `FileType::is_symlink`
/// reports name-surrogate reparse tags (symlinks and mount points/junctions)
/// as links, so a dangling junction is classified here too. Other reparse
/// points (cloud placeholders, for example) are not name surrogates and stable
/// `std` does not expose them as links; a dangling one therefore reads as "is
/// not a directory". Detecting those needs the Windows reparse-point APIs,
/// which this crate deliberately does not take on.
fn game_dir_refusal(game_dir: &Path) -> std::io::Result<Option<anyhow::Error>> {
    // `Path::exists()` follows links, so it reads a dangling link as absent.
    // `symlink_metadata` inspects the link itself and tells the two apart.
    let state = match std::fs::symlink_metadata(game_dir) {
        Ok(md) if md.file_type().is_symlink() => match std::fs::metadata(game_dir) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => "is a dangling symlink",
            Ok(_) => "is not a directory",
            Err(_) => return Ok(None),
        },
        Ok(_) => "is not a directory",
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "does not exist",
        // A path component is not a directory, so the game dir cannot be one
        // either (the `ENOTDIR` answer some platforms give for
        // `<non-directory>/config.ini`).
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => "is not a directory",
        Err(e) => return Err(e),
    };
    Ok(Some(anyhow::anyhow!(
        "game dir {} {state}",
        game_dir.display()
    )))
}

/// The `NotFound` caller's error for a game dir that could not be inspected.
///
/// The inspection error keeps its kind (a caller can still tell the failure
/// apart) but gains the path context the call site adds, so the caller is not
/// left with a bare OS error.
fn inspect_game_dir_error(game_dir: &Path, inspection: std::io::Error) -> anyhow::Error {
    anyhow::Error::from(inspection).context(format!("inspect game dir {}", game_dir.display()))
}

/// Bump (or create) game-dir `config.ini` with latest version + channel fields.
///
/// The game directory must already exist as a directory; only `config.ini`
/// itself may be missing (the fresh-install input). A missing directory, or a
/// path that exists but is not a directory, is an error and is not created,
/// even though the shared atomic writer would create a parent: this writer
/// repairs an existing install, so a wrong path must be reported rather than
/// turned into a stray file tree.
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
) -> Result<()> {
    let path = game_dir.join("config.ini");
    // One read: `exists()` then `read_to_string` would stat twice and leave a
    // TOCTOU window. Absent is the documented fresh-install input; any other
    // read error is refused rather than silently treated as an empty config.
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // `NotFound` covers both "no config.ini yet" (fresh install, a
            // supported input) and a game dir that is missing or not a
            // directory. This writer only updates an existing install, so the
            // latter stays an error: the post phase runs after the game dir was
            // scanned, and creating a tree for a wrong path would hide the
            // mistake instead of reporting it.
            if !game_dir.is_dir() {
                return match game_dir_refusal(game_dir) {
                    Ok(Some(refusal)) => Err(refusal),
                    Ok(None) => Err(e.into()),
                    // The original error was `NotFound`, so an inspection
                    // failure (a permission denial on the way to the path, for
                    // example) is the honest answer; reporting the path as
                    // missing would be a guess.
                    Err(inspection) => Err(inspect_game_dir_error(game_dir, inspection)),
                };
            }
            String::new()
        }
        Err(e) => {
            // Some platforms answer `ENOTDIR` for `<non-directory>/config.ini`
            // instead of `NotFound`; name the real condition there too rather
            // than passing the nested read through as an unreadable file.
            // Only that kind means the path is not a directory: any other
            // error (a permission failure on a parent, say) keeps its own kind
            // and message instead of being relabelled.
            // An inspection failure falls through to the original
            // `NotADirectory`, which already names the real condition; unlike
            // the `NotFound` arm, nothing is being guessed.
            if e.kind() == std::io::ErrorKind::NotADirectory
                && let Ok(Some(refusal)) = game_dir_refusal(game_dir)
            {
                return Err(refusal);
            }
            return Err(e.into());
        }
    };
    let mut sections = parse_ini(&text);
    merge_ini(&mut sections, latest, biz, channel);
    let out = render_ini(&mut sections);
    publish_ini(&path, out.as_bytes())
}

/// One `[section]` of a `config.ini`, in first-seen order.
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

/// The index of the first section with this lowercase name.
fn find_section(sections: &[Section], lower: &str) -> Option<usize> {
    sections.iter().position(|s| s.lower == lower)
}

/// The `[General]` index, created when it is not there yet.
fn ensure_general(sections: &mut Vec<Section>) -> usize {
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
}

/// Load the existing text into sections, preserving every `[section]` and
/// merging duplicates by their case-insensitive name.
///
/// Comment (`;` / `#`, no-`=` lines) and blank lines are dropped on load;
/// header-less keys land in `[General]` (a deliberate deviation from
/// Collapse's generic `[default]` bucket, since game-dir files are
/// `[General]`-scoped).
fn parse_ini(text: &str) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut current: Option<usize> = None;
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
        let Some((key, rest)) = split_first_eq(line) else {
            continue;
        };
        let key = key.trim_end();
        if key.is_empty() {
            continue;
        }
        // Collapse: value is trimmed, surrounding `"` stripped.
        let value = rest.trim().trim_matches('"');
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
    sections
}

/// Force the version and channel keys in `[General]`.
///
/// Forced: `game_version=<latest>`, `channel` / `sub_channel` / `cps` per biz
/// and `game_biz` — a deliberate Starward carryover (`SetGameConfigIniAsync`
/// writes it, Collapse never does; game/launcher interop expects it, so it
/// stays forced). `sdk_version`: Collapse parity — preserved when present,
/// created empty when missing (Collapse only ever holds the empty
/// `DefaultIniVersion` default; the real SDK version lives in
/// `plugin_sdk_version`, which girpr never touches).
fn merge_ini(sections: &mut Vec<Section>, latest: &str, biz: &str, channel: (&str, &str, &str)) {
    let g = ensure_general(sections);
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
}

/// Render the loaded and merged sections in Collapse `SaveInner`'s save shape:
/// sections in first-seen order, keys sorted alphabetically
/// (case-insensitive), and a blank line after each section.
fn render_ini(sections: &mut [Section]) -> String {
    let mut out = String::new();
    for s in sections.iter_mut() {
        s.keys.sort_by(|a, b| a.1.cmp(&b.1));
    }
    // Writing to a `String` is infallible, so the `fmt::Result` is ignored;
    // the blank line after each section is still pushed by hand.
    for s in sections.iter() {
        let _ = writeln!(out, "[{}]", s.name);
        for (k, _, v) in &s.keys {
            let _ = writeln!(out, "{k}={v}");
        }
        out.push('\n');
    }
    out
}

/// Write the rendered INI through the shared atomic writer.
///
/// The version bump must survive a power loss, so the temp is synced before
/// the rename; the test hook skips only that sync, never the write or rename.
fn publish_ini(path: &Path, bytes: &[u8]) -> Result<()> {
    write_atomic(path, bytes, !test_no_fsync())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use crate::util::junction_support::{junction_skip_reason, try_junction_dir};
    use crate::util::path::temp_sibling;
    use std::collections::HashSet;

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

    /// The line scan keeps the documented shape: no leading whitespace
    /// before the key, whitespace allowed around `=`, a value of at least one
    /// character, trailing whitespace trimmed, and the last match wins.
    #[test]
    fn version_pattern_keeps_its_anchored_shape() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_cfg", "version_shape");
        std::fs::write(
            dir.join("config.ini"),
            "[General]\n game_version=9.9.9\ngame_version=4.0.0\ngame_version = 5.1.0  \ngame_version=\n",
        )
        .unwrap();
        assert_eq!(
            read_game_version(&dir).unwrap().unwrap(),
            "5.1.0",
            "the indented and the empty-value lines must not match"
        );
    }

    #[test]
    fn version_line_shapes() {
        let cases = [
            ("game_version=1.0.0", Some("1.0.0")),
            (" game_version=1.0.0", None),
            ("\tgame_version=1.0.0", None),
            ("game_version=", None),
            ("game_version", None),
            ("game_version_extra=1.0.0", None),
            ("Game_version=1.0.0", None),
            ("game_version\t=\t1.0.0\t", Some("1.0.0")),
            ("game_version = 1.0.0   ", Some("1.0.0")),
            ("game_version=1=2=3", Some("1=2=3")),
            ("game_version=   ", Some("")),
        ];
        for (line, expected) in cases {
            assert_eq!(
                game_version_line(line).as_deref(),
                expected,
                "line {line:?}"
            );
        }
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
        write_config_ini(&dir, "5.1.0", "hk4e_global", ("1", "0", "hyp_hoyoverse")).unwrap();
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
        write_config_ini(&dir, "5.1.0", "hk4e_global", ("1", "0", "hyp_hoyoverse")).unwrap();
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
        write_config_ini(&dir, "5.1.0", "hk4e_global", ("1", "0", "hyp_hoyoverse")).unwrap();
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
        write_config_ini(&dir, "5.1.0", "hk4e_global", ("1", "0", "hyp_hoyoverse")).unwrap();
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
        write_config_ini(&dir, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo")).unwrap();
        let text = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        assert!(text.starts_with("[General]\n"), "{text}");
        assert!(text.contains("game_version=5.1.0"), "{text}");
        assert!(text.contains("cps=hyp_mihoyo"), "{text}");
        assert!(text.contains("game_biz=hk4e_cn"), "{text}");
        assert!(text.contains("sdk_version=\n"), "{text}");

        std::fs::write(dir.join("config.ini"), "fps=60\ngame_version=1.0.0\n").unwrap();
        write_config_ini(&dir, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo")).unwrap();
        let text2 = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        assert!(text2.contains("fps=60"), "{text2}");
        assert!(text2.contains("game_version=5.1.0"), "{text2}");
        assert_eq!(text2.matches("[General]").count(), 1, "{text2}");
    }

    /// The writer updates an existing install: an absent `config.ini` inside an
    /// existing game dir is the fresh-install input and is created, but an
    /// absent game dir is an error and must not be created by the shared atomic
    /// writer's parent creation.
    #[test]
    fn config_bump_requires_an_existing_game_dir() {
        let root = girpr_tempdir::TempDir::new("girpr_test_cfg", "missing_game_dir");
        let missing = root.join("not-installed");
        let err = write_config_ini(&missing, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo"))
            .expect_err("a missing game dir must be reported, not created");
        assert!(
            format!("{err:#}").contains("does not exist"),
            "the refusal names the missing game dir: {err:#}"
        );
        assert!(!missing.exists(), "the writer must not create the game dir");
    }

    /// An inspection failure other than `NotFound`/`NotADirectory` is returned
    /// to the caller rather than being discarded in favour of a missing-path
    /// answer. An embedded NUL makes the metadata call fail with
    /// `InvalidInput` on every supported platform.
    #[test]
    fn game_dir_inspection_errors_are_not_reported_as_missing() {
        let err = game_dir_refusal(Path::new("bad\0path"))
            .expect_err("a path that cannot be inspected must be an error");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{err}");
    }

    /// The `NotFound` arm surfaces a failed game-dir inspection with the name
    /// of the path it could not inspect, and keeps the I/O error's kind so the
    /// caller can still tell a permission failure from a malformed path. A NUL
    /// makes `symlink_metadata` fail with `InvalidInput` on every supported
    /// platform, so the inspection failure comes from the real syscall
    /// boundary; the context is applied exactly as the caller applies it.
    /// A NUL read fails with `InvalidInput` before `write_config_ini` reaches
    /// the `NotFound` arm, so the private helper is driven directly here: there
    /// is no call-site path that can reach that arm with an unreadable path.
    #[test]
    fn a_failed_game_dir_inspection_keeps_its_kind_and_names_the_path() {
        let game_dir = Path::new("uninspected\0game-dir");
        let inspection = game_dir_refusal(game_dir).expect_err("a NUL path cannot be inspected");
        assert_eq!(
            inspection.kind(),
            std::io::ErrorKind::InvalidInput,
            "{inspection}"
        );

        let err = inspect_game_dir_error(game_dir, inspection);
        let msg = format!("{err:#}");
        assert!(
            msg.contains("inspect game dir uninspected"),
            "the call site names the path: {msg}"
        );
        let io = err
            .downcast_ref::<std::io::Error>()
            .expect("the inspection error survives the context");
        assert_eq!(io.kind(), std::io::ErrorKind::InvalidInput, "{err:#}");
    }

    /// A path that exists as a file is a different mistake from a missing
    /// game dir, and the refusal says which one happened; it still does not
    /// touch the path.
    #[test]
    fn config_bump_refuses_a_game_path_that_is_not_a_directory() {
        let root = girpr_tempdir::TempDir::new("girpr_test_cfg", "game_path_is_file");
        let path = root.join("not-a-dir");
        std::fs::write(&path, b"not a game dir").unwrap();

        let err = write_config_ini(&path, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo"))
            .expect_err("a non-directory game path must be reported, not written through");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("is not a directory") && msg.contains("not-a-dir"),
            "the refusal names the real condition: {msg}"
        );
        assert!(
            !msg.contains("does not exist"),
            "a present path must not be reported as missing: {msg}"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"not a game dir",
            "the writer must not replace the path it refused"
        );
    }

    /// A game path that is a symlink to a missing target exists as a link but
    /// not as a directory. `Path::exists()` follows the link and would call it
    /// missing; the refusal must name the dangling link instead. Symlink
    /// creation needs a privilege or developer mode on Windows, so the test
    /// skips where the link cannot be made.
    #[test]
    fn config_bump_refuses_a_dangling_symlink_game_dir() {
        let root = girpr_tempdir::TempDir::new("girpr_test_cfg", "dangling_game_dir");
        let target = root.join("missing-target");
        let link = root.join("dangling-game-dir");
        let mut cleanup = LinkGuard::new(&link);
        if let Err(e) = try_symlink_dir(&target, &link) {
            match symlink_skip_reason(&e) {
                Some(why) => {
                    eprintln!("SKIP precondition: {why}: {e}");
                    return;
                }
                None => panic!("could not create the test directory symlink: {e}"),
            }
        }

        let err = write_config_ini(&link, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo"))
            .expect_err("a dangling symlink must be reported, not written through");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("dangling symlink") && msg.contains("dangling-game-dir"),
            "the refusal names the dangling link: {msg}"
        );
        assert!(
            !msg.contains("does not exist"),
            "an existing link must not be reported as missing: {msg}"
        );
        assert!(
            !target.exists(),
            "the writer must not create the symlink target"
        );
        cleanup.remove().expect("the symlink is removable");
    }

    /// A directory junction whose target does not exist is a name-surrogate
    /// reparse point, so `FileType::is_symlink` reports it as a link and the
    /// refusal must name the dangling link instead of "does not exist".
    /// Junctions need no privilege, so only a proven host refusal (an
    /// unsupported platform or the known privilege denial) skips; any other
    /// failure is a broken test environment and must fail loudly.
    #[cfg(windows)]
    #[test]
    fn config_bump_refuses_a_dangling_junction_game_dir() {
        let root = girpr_tempdir::TempDir::new("girpr_test_cfg", "dangling_junction");
        let target = root.join("never-created-target");
        let link = root.join("dangling-game-dir");
        let mut cleanup = LinkGuard::new(&link);
        if let Err(e) = try_junction_dir(&target, &link) {
            match junction_skip_reason(&e) {
                Some(why) => {
                    eprintln!("SKIP precondition: {why}: {e}");
                    return;
                }
                None => panic!("could not create the test directory junction: {e}"),
            }
        }

        let err = write_config_ini(&link, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo"))
            .expect_err("a dangling junction must be reported, not written through");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("dangling symlink") && msg.contains("dangling-game-dir"),
            "the refusal names the dangling link: {msg}"
        );
        assert!(
            !msg.contains("does not exist"),
            "an existing junction must not be reported as missing: {msg}"
        );
        assert!(
            !target.exists(),
            "the writer must not create the junction target"
        );
        cleanup.remove().expect("the junction is removable");
    }

    /// Removes a link or junction — never its target — when the guard leaves
    /// scope, so a failed assertion does not leave a dangling reparse point for
    /// the temp-dir cleanup to walk.
    ///
    /// Armed before the link is created: a creation that fails partway leaves
    /// nothing behind, and a host that skips the case is cleaned too.
    struct LinkGuard<'a> {
        link: &'a Path,
        armed: bool,
    }

    impl<'a> LinkGuard<'a> {
        fn new(link: &'a Path) -> Self {
            Self { link, armed: true }
        }

        /// Remove the link now and report a failure. `Drop` stays armed until
        /// the removal succeeds, so a failed success-path removal is retried
        /// there instead of leaving the link behind; a successful removal is
        /// not repeated.
        fn remove(&mut self) -> std::io::Result<()> {
            let result = remove_link(self.link);
            if result.is_ok() {
                self.armed = false;
            }
            result
        }
    }

    impl Drop for LinkGuard<'_> {
        fn drop(&mut self) {
            if self.armed {
                // Best effort: this is the failure path, and a cleanup error
                // must not replace the test's own panic.
                let _ = remove_link(self.link);
            }
        }
    }

    /// Remove the link itself, never its target. On Windows both a directory
    /// symlink and a junction are removed with `remove_dir`; elsewhere the link
    /// is removed with `remove_file`, which also covers a link to a directory.
    fn remove_link(link: &Path) -> std::io::Result<()> {
        match std::fs::symlink_metadata(link) {
            Ok(_) => remove_link_at(link),
            // The creation may have failed before anything was left.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    #[cfg(windows)]
    fn remove_link_at(link: &Path) -> std::io::Result<()> {
        std::fs::remove_dir(link)
    }

    #[cfg(not(windows))]
    fn remove_link_at(link: &Path) -> std::io::Result<()> {
        std::fs::remove_file(link)
    }

    /// Whether a failed link creation — a directory symlink or a junction —
    /// is the known privilege denial that means the test cannot run on this
    /// host. Every other error is a broken test environment and must fail
    /// loudly instead of silently skipping.
    fn link_creation_denied(e: &std::io::Error) -> bool {
        #[cfg(windows)]
        {
            // A link creation the host's policy denies reports
            // `ERROR_PRIVILEGE_NOT_HELD` (1314); a directory symlink without
            // the privilege (or developer mode) is the usual case. Rust maps
            // it to `Uncategorized`, so the raw code is the reliable check.
            e.raw_os_error() == Some(1314)
        }
        #[cfg(unix)]
        {
            // `symlink(2)` reports `EPERM` (1) or `EACCES` (13) when the
            // caller lacks permission. Anything else (a read-only filesystem,
            // for example) is not the privilege denial this test skips on.
            matches!(e.raw_os_error(), Some(1) | Some(13))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = e;
            false
        }
    }

    /// Why a failed directory-symlink creation means the case cannot run on
    /// this host, or `None` when it is a broken test environment that must
    /// fail loudly.
    ///
    /// A platform without directory symlinks reports `Unsupported`; the known
    /// privilege denial is the other untestable host.
    ///
    /// Treating every `Unsupported` as a host skip is deliberate: `std` may map
    /// more platform failures there over time, and a host that cannot make the
    /// link cannot exercise the case. Any other error is a broken test
    /// environment and must still panic rather than quietly skip.
    fn symlink_skip_reason(e: &std::io::Error) -> Option<&'static str> {
        if e.kind() == std::io::ErrorKind::Unsupported {
            Some("directory symlinks are not supported on this platform")
        } else if link_creation_denied(e) {
            Some("creating a directory symlink is denied on this host")
        } else {
            None
        }
    }

    #[cfg(unix)]
    fn try_symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn try_symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    #[cfg(not(any(unix, windows)))]
    fn try_symlink_dir(_target: &Path, _link: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "directory symlinks are not supported on this platform",
        ))
    }

    /// Only the known privilege denial or a platform without directory
    /// symlinks is a skip; other failures would hide a broken test
    /// environment.
    #[test]
    fn only_a_denied_or_unsupported_symlink_skips_the_link_test() {
        assert!(
            symlink_skip_reason(&std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not a creation-denied error",
            ))
            .is_none(),
            "an unexpected failure must still fail the test"
        );
        assert_eq!(
            symlink_skip_reason(&std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "no directory symlinks",
            )),
            Some("directory symlinks are not supported on this platform"),
            "an unsupported platform is a skip, not a panic"
        );
        #[cfg(windows)]
        {
            assert!(link_creation_denied(&std::io::Error::from_raw_os_error(
                1314
            )));
            assert!(!link_creation_denied(&std::io::Error::from_raw_os_error(5)));
            assert_eq!(
                symlink_skip_reason(&std::io::Error::from_raw_os_error(1314)),
                Some("creating a directory symlink is denied on this host"),
                "the privilege denial skips, not panics"
            );
            assert!(
                symlink_skip_reason(&std::io::Error::from_raw_os_error(5)).is_none(),
                "another access failure must still fail the test"
            );
        }
        #[cfg(unix)]
        {
            assert!(link_creation_denied(&std::io::Error::from_raw_os_error(1)));
            assert!(link_creation_denied(&std::io::Error::from_raw_os_error(13)));
            assert!(!link_creation_denied(&std::io::Error::from_raw_os_error(
                30
            )));
            for denied in [1, 13] {
                assert_eq!(
                    symlink_skip_reason(&std::io::Error::from_raw_os_error(denied)),
                    Some("creating a directory symlink is denied on this host"),
                    "the privilege denial skips, not panics"
                );
            }
            assert!(
                symlink_skip_reason(&std::io::Error::from_raw_os_error(30)).is_none(),
                "a read-only filesystem must still fail the test"
            );
        }
    }

    /// A failed explicit removal must leave `LinkGuard` armed, so `Drop`
    /// retries it. A non-empty directory makes the first removal fail; clearing
    /// it lets the retry succeed, and the link must then be gone.
    #[cfg(any(unix, windows))]
    #[test]
    fn link_guard_retries_a_failed_explicit_removal_on_drop() {
        let root = girpr_tempdir::TempDir::new("girpr_test_cfg", "guard_retry");
        let link = root.join("link");

        #[cfg(windows)]
        {
            // `remove_dir` refuses a non-empty directory.
            std::fs::create_dir(&link).unwrap();
            std::fs::write(link.join("blocker"), b"x").unwrap();
            {
                let mut guard = LinkGuard::new(&link);
                assert!(
                    guard.remove().is_err(),
                    "a non-empty directory is not removable"
                );
                std::fs::remove_file(link.join("blocker")).unwrap();
            }
        }
        #[cfg(unix)]
        {
            // `remove_file` refuses a directory.
            std::fs::create_dir(&link).unwrap();
            {
                let mut guard = LinkGuard::new(&link);
                assert!(guard.remove().is_err(), "a directory is not a file");
                std::fs::remove_dir(&link).unwrap();
                std::fs::write(&link, b"x").unwrap();
            }
        }

        assert!(
            std::fs::symlink_metadata(&link).is_err(),
            "the guard must retry the failed removal when it drops"
        );
    }

    /// A parent that cannot be traversed is a permission failure, not a
    /// missing or non-directory game dir: the refusal must not relabel it.
    /// Hosts that ignore the mode (root, some containers) cannot exercise the
    /// denial, so the test skips there.
    #[cfg(unix)]
    #[test]
    fn config_bump_surfaces_a_parent_permission_error_unchanged() {
        use std::os::unix::fs::PermissionsExt;

        let root = girpr_tempdir::TempDir::new("girpr_test_cfg", "parent_permission");
        let locked = root.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let game = locked.join("game");
        // The denial must be observable before the test can exercise it: a
        // host that ignores the mode (root, some containers) can still read
        // the directory, and then the game dir is merely missing rather than
        // unreachable.
        if std::fs::read_dir(&locked).is_ok() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
            return;
        }

        let err = write_config_ini(&game, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo"))
            .expect_err("an unreachable game dir must not be treated as a fresh install");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();

        let io = err
            .downcast_ref::<std::io::Error>()
            .expect("the permission failure keeps its I/O error");
        assert_eq!(io.kind(), std::io::ErrorKind::PermissionDenied, "{err:#}");
        assert!(
            !format!("{err:#}").contains("is not a directory"),
            "a permission failure must not be relabelled: {err:#}"
        );
    }

    /// An accepted config write and a failed one both leave no temp sibling:
    /// the shared temp name is the only place this writer creates one, and
    /// publishing always renames it away or removes it.
    #[test]
    fn config_writes_leave_no_temp_sibling() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_cfg", "no_temp_sibling");
        let path = dir.join("config.ini");
        let tmp = temp_sibling(&path);

        write_config_ini(&dir, "5.1.0", "hk4e_cn", ("1", "1", "hyp_mihoyo")).unwrap();
        assert!(path.is_file(), "the accepted write publishes config.ini");
        assert!(
            !tmp.exists(),
            "an accepted write renames the temp away instead of leaving it"
        );

        // A directory at the target makes the publish rename fail after the
        // temp was written; the failure cleanup removes it.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let err = publish_ini(&path, b"[General]\n").unwrap_err();
        assert!(format!("{err:#}").contains("publish"), "{err:#}");
        assert!(
            path.is_dir(),
            "the failed publish leaves the target untouched"
        );
        assert!(
            !tmp.exists(),
            "a failed write removes the temp instead of leaving it"
        );
    }

    /// Both readers take the first `=`. `parse_ini` keeps its raw key/value
    /// shape (key trimmed at the end only, value trimmed and unquoted, an
    /// empty value allowed), while the version scan stays anchored.
    #[test]
    fn readers_split_on_the_first_equals() {
        let sections = parse_ini(
            "k e y= raw = value \n\
             empty=\n\
             quoted=  \"a=b\"  \n\
             =value\n\
                =value\n\
             spaced   =1\n",
        );
        assert_eq!(sections.len(), 1, "header-less keys land in [General]");
        let general = &sections[0];
        assert_eq!(general.name, "General");
        let keys: Vec<(&str, &str)> = general
            .keys
            .iter()
            .map(|(k, _, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                ("k e y", "raw = value"),
                ("empty", ""),
                ("quoted", "a=b"),
                ("spaced", "1"),
            ],
            "keys split at the first '=', only end-trimmed; a leading '=' is not a key"
        );

        for (line, expected) in [
            ("game_version=1=2=3", Some("1=2=3")),
            ("game_version==x", Some("=x")),
            (" game_version=1.0.0", None),
            ("game_version_extra=1.0.0", None),
            ("game_version=   ", Some("")),
        ] {
            assert_eq!(
                game_version_line(line).as_deref(),
                expected,
                "line {line:?}"
            );
        }
    }

    /// The extracted parse / merge / render steps keep the documented section
    /// and key rules: header-less keys go to `[General]`, duplicate sections
    /// merge case-insensitively into the first spelling, comments and blank
    /// lines drop, forced keys are created with their canonical spelling,
    /// `sdk_version` is created empty, and keys sort per section.
    #[test]
    fn parse_merge_and_render_pin_the_section_rules() {
        let mut sections = parse_ini(
            "; leading comment\n\
             fps=60\n\
             [Launcher]\n\
             log_level=3\n\
             [launcher]\n\
             game_install_path=C:/g\n\
             [General]\n\
             FPS=120\n\
             channel=5\n",
        );
        merge_ini(
            &mut sections,
            "5.1.0",
            "hk4e_global",
            ("1", "0", "hyp_hoyoverse"),
        );
        let text = render_ini(&mut sections);
        assert_eq!(
            text,
            "[General]\n\
             channel=1\n\
             cps=hyp_hoyoverse\n\
             fps=120\n\
             game_biz=hk4e_global\n\
             game_version=5.1.0\n\
             sdk_version=\n\
             sub_channel=0\n\
             \n\
             [Launcher]\n\
             game_install_path=C:/g\n\
             log_level=3\n\
             \n"
        );
        assert!(
            !text.contains("[launcher]"),
            "the duplicate section merges into the first-seen spelling: {text}"
        );
    }
}
