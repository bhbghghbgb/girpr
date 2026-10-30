use std::collections::HashSet;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};

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
    Ok(game_version_re()
        .captures_iter(&text)
        .last()
        .map(|c| c[1].trim().to_string()))
}

/// The `game_version` line pattern, compiled once per process.
///
/// The pattern is a literal, so parsing it once is enough; it is called per run
/// in production but often in tests. Semantics are fixed: `^` anchors at the
/// line start (no leading whitespace before `game_version`), `\s*` around `=`,
/// at least one character of value, trailing whitespace trimmed, and the caller
/// takes the **last** match.
fn game_version_re() -> &'static regex::Regex {
    static GAME_VERSION: OnceLock<regex::Regex> = OnceLock::new();
    GAME_VERSION.get_or_init(|| {
        regex::Regex::new(r"(?m)^game_version\s*=\s*(.+?)\s*$")
            .expect("the game_version pattern is a literal")
    })
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
    // One read: `exists()` then `read_to_string` would stat twice and leave a
    // TOCTOU window. Absent is the documented fresh-install input; any other
    // read error is refused rather than silently treated as an empty config.
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
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
    // Writing to a `String` is infallible, so the `fmt::Result` is ignored;
    // the blank line after each section is still pushed by hand.
    for s in &sections {
        let _ = writeln!(out, "[{}]", s.name);
        for (k, _, v) in &s.keys {
            let _ = writeln!(out, "{k}={v}");
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

    /// The cached pattern keeps the documented shape: no leading whitespace
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
