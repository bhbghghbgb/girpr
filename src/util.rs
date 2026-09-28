use anyhow::{Context, Result};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

const MD5_BUF: usize = 512 * 1024;

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
/// See https://github.com/Scighost/Starward/blob/3e2da5ffecde252211edb74b850ee13d6b93f6dd/src/Starward.RPC/GameInstall/GamePackageService.cs#L47-L70
pub fn read_game_version(game_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(game_dir.join("config.ini")).ok()?;
    let re = regex::Regex::new(r"(?m)^game_version\s*=\s*(.+?)\s*$").ok()?;
    re.captures_iter(&text).last().map(|c| c[1].trim().to_string())
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
            && v.get("is_delete").and_then(|b| b.as_bool()).unwrap_or(false)
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

/// Bump (or create) config.ini with latest version + channel fields.
/// Starward parity (`SetGameConfigIniAsync`): preserve unknown keys, force the
/// known set below. Single `[General]` header; comments/other sections are not
/// preserved (matches Starward's wholesale rewrite; Collapse preserves sections).
/// TODO(sdk_version): Starward writes the real channel SDK version
/// (`GameChannelSDK?.Version ?? ""`); v1 always writes `""` (no SDK fetch).
/// Fetch the channel SDK and write the real version if a channel ever requires it.
pub fn write_config_ini(
    game_dir: &Path,
    latest: &str,
    biz: &str,
    channel: (&str, &str, &str),
    sdk_version: &str,
    dry_run: bool,
) -> Result<()> {
    let path = game_dir.join("config.ini");
    let mut map: Vec<(String, String)> = Vec::new();
    if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('[') || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let k = k.trim().to_string();
                if k == "game_version"
                    || k == "channel"
                    || k == "sub_channel"
                    || k == "cps"
                    || k == "sdk_version"
                    || k == "game_biz"
                {
                    continue;
                }
                map.push((k, v.trim().to_string()));
            }
        }
    }
    map.push(("game_version".into(), latest.into()));
    map.push(("channel".into(), channel.0.into()));
    map.push(("sub_channel".into(), channel.1.into()));
    map.push(("cps".into(), channel.2.into()));
    map.push(("sdk_version".into(), sdk_version.into()));
    map.push(("game_biz".into(), biz.into()));
    let mut out = String::from("[General]\n");
    for (k, v) in &map {
        out.push_str(&format!("{}={}\n", k, v));
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
        let dir = std::env::temp_dir().join("girpr_test_cfg");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.ini"), "[General]\ngame_version=4.0.0\ngame_version=5.1.0\n").unwrap();
        assert_eq!(read_game_version(&dir).unwrap(), "5.1.0");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ignore_and_blacklist_parse() {
        let dir = std::env::temp_dir().join("girpr_test_lists");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rc"), "{\"category\":\"10302\",\"is_delete\":true}\n{}\n").unwrap();
        assert!(read_ignore_categories(&dir.join("rc")).contains("10302"));
        std::fs::write(dir.join("bl"), "{\"fileName\":\"a/b.dat\"}\n").unwrap();
        assert!(read_blacklist(&dir.join("bl")).contains("a/b.dat"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn config_bump_forces_target_keys_preserves_rest_and_is_idempotent() {
        let dir = std::env::temp_dir().join("girpr_test_cfg_bump");
        std::fs::create_dir_all(&dir).unwrap();
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
            "",
            false,
        )
        .unwrap();
        let text = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        assert!(text.starts_with("[General]\n"));
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.contains(&"game_version=5.1.0"));
        assert!(!lines.contains(&"game_version=4.0.0"));
        assert!(lines.contains(&"fps=120"));
        assert!(lines.contains(&"sdk_version="));
        assert!(!lines.contains(&"sdk_version=9.9.9"));
        assert!(lines.contains(&"game_biz=hk4e_global"));
        assert!(lines.contains(&"channel=1"));
        assert!(lines.contains(&"sub_channel=0"));
        assert!(lines.contains(&"cps=hyp_hoyoverse"));
        assert_eq!(read_game_version(&dir).unwrap(), "5.1.0");
        // rerun: no duplicate keys (existing forced keys replaced in place)
        write_config_ini(
            &dir,
            "5.1.0",
            "hk4e_global",
            ("1", "0", "hyp_hoyoverse"),
            "",
            false,
        )
        .unwrap();
        let text2 = std::fs::read_to_string(dir.join("config.ini")).unwrap();
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
                .filter(|l| l.starts_with(&format!("{key}=")))
                .count();
            assert_eq!(count, 1, "{key} must appear exactly once");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
