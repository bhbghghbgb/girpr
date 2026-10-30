use anyhow::{Context, Result};
use std::path::{Component, Path, PathBuf};

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
    let mut clean = PathBuf::new();
    walk_rel_components(rel, |part| clean.push(part))?;
    Ok(clean)
}

/// Validate the lexical shape of a server-controlled relative path and hand
/// each component that names a file to `emit`.
///
/// The normalisation (`\` → `/`) and the whole-path checks happen once, here;
/// `emit` only ever sees plain component names, so `a/./b` is accepted but
/// `emit` receives `a` and `b` and never `.`.
///
/// The caller builds the cleaned form rather than returning the input: `a/./b`
/// must not survive as `a/./b`, because the plan stores this string and every
/// later comparison (the purge's expected set, the reuse map, the scope) works
/// on the plain form. A leading `.` in particular would make `expected_paths()`
/// disagree with the purge walk — which reports `game/foo.dat` — and the
/// cleanup would delete the file this run just repaired.
fn walk_rel_components(rel: &str, mut emit: impl FnMut(&str)) -> Result<()> {
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
                emit(part);
            }
            // `a/./b` is still contained; it just says nothing.
            Component::CurDir => {}
            _ => anyhow::bail!("path is not a plain relative path: {rel}"),
        }
    }
    anyhow::ensure!(named, "path names no file: {rel}");
    Ok(())
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
    // Share the component walk with `contained_rel` and build the `/` form as
    // it goes: no `PathBuf`, and no second pass over the result.
    let mut clean = String::new();
    walk_rel_components(rel, |part| {
        if !clean.is_empty() {
            clean.push('/');
        }
        clean.push_str(part);
    })?;
    Ok(clean)
}

/// The key a path comparison uses when two spellings may name one file.
///
/// Windows paths are case-insensitive, so the plan's dedup and the purge's
/// keep comparison both fold case there; on Unix they name distinct files and
/// the key is the path itself. The two must agree, so the rule lives here
/// instead of once per module.
///
/// The choice is compile-time (`cfg!`), so both branches are type-checked on
/// every host while the behavior stays exactly "fold where Windows paths are
/// case-insensitive".
pub(crate) fn path_key(rel: &str) -> String {
    if cfg!(windows) {
        rel.to_lowercase()
    } else {
        rel.to_string()
    }
}

/// Is this component a name Windows reserves for a device in every directory?
///
/// The comparison uses the part before the first `.` — `NUL.txt` is still the
/// `NUL` device — and ignores case, which NTFS does for these names too. It is
/// allocation-free: no uppercased copy of the component is made.
fn is_windows_device_name(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.']);
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"]
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
    {
        return true;
    }
    // `COM1`..`COM9` and `LPT1`..`LPT9`. Compare bytes, so a multi-byte UTF-8
    // stem can never be sliced through a character boundary.
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
        && matches!(bytes[3], b'1'..=b'9')
}

/// A base path resolved once, for callers that join many rel paths under it.
///
/// [`contained_join`] resolves the base on every call. A run that joins one rel
/// per planned file would pay for that same resolution — a `canonicalize` walk
/// up to the nearest existing ancestor — once per file, so the hot callers
/// build one `PathRoot` and call [`join`](Self::join) per file instead.
///
/// Caching the **base** does not make the check lexical: the joined path is
/// still resolved on the filesystem and compared with the cached resolved base,
/// so a symlink inside the install that points out of it is still refused.
///
/// A base that does not exist yet (the `--print-plan` run before `game_dir` is
/// created) resolves to the same "nearest existing ancestor + missing tail"
/// form as [`contained_join`] uses, so the missing-base lifecycle is unchanged.
/// The cached form is **not** refreshed implicitly: a caller that creates the
/// base later must build a fresh `PathRoot` after that.
#[derive(Debug, Clone)]
pub struct PathRoot {
    base: PathBuf,
    resolved: PathBuf,
}

impl PathRoot {
    /// Resolve `base` once.
    pub fn new(base: &Path) -> Result<Self> {
        let resolved = resolve_existing_prefix(base)
            .with_context(|| format!("resolve the game dir {}", base.display()))?;
        Ok(Self {
            base: base.to_path_buf(),
            resolved,
        })
    }

    /// Join a server-controlled relative path under this root, refusing
    /// anything that resolves outside it.
    ///
    /// This is the per-file half of [`contained_join`]: the lexical check is
    /// still [`contained_rel`], and the joined path is still resolved and
    /// compared with the resolved base.
    pub fn join(&self, rel: &str) -> Result<PathBuf> {
        self.join_rel(contained_rel(rel)?)
    }

    /// The filesystem half of [`join`](Self::join), after the lexical check.
    fn join_rel(&self, rel: PathBuf) -> Result<PathBuf> {
        let joined = self.base.join(rel);
        let joined_resolved = resolve_existing_prefix(&joined)
            .with_context(|| format!("resolve {}", joined.display()))?;
        anyhow::ensure!(
            joined_resolved.starts_with(&self.resolved),
            "path escapes {}: {}",
            self.base.display(),
            joined.display()
        );
        Ok(joined)
    }
}

/// Join a server-controlled relative path under `base`, refusing anything that
/// resolves outside it.
///
/// The lexical check is [`contained_rel`]. The filesystem check resolves the
/// **nearest existing ancestor** of both sides — so a missing `game_dir` (a
/// `--print-plan` run, a fresh install) still works — and requires the joined
/// path to stay under the base. That is what catches a symlink inside the
/// install that points out of it, which a lexical `starts_with` cannot.
///
/// This is the one-off form: it resolves `base` on every call. A caller that
/// joins many rels under one base should build a [`PathRoot`] once instead.
pub fn contained_join(base: &Path, rel: &str) -> Result<PathBuf> {
    // The lexical refusal comes first, as it always has: a bad rel is reported
    // without touching the filesystem, even when the base is also unreadable.
    let rel = contained_rel(rel)?;
    PathRoot::new(base)?.join_rel(rel)
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// G6: `clean_rel` builds the `/` form straight from the component walk, so
    /// both separators, doubled separators and `.` components all collapse to
    /// the same plain string, and the refusals still hold on that path.
    #[test]
    fn clean_rel_returns_the_plain_slash_form() {
        for (input, want) in [
            ("a/b.dat", "a/b.dat"),
            ("a\\b.dat", "a/b.dat"),
            ("a//b.dat", "a/b.dat"),
            ("a\\\\b.dat", "a/b.dat"),
            ("./a/b.dat", "a/b.dat"),
            ("a/./b.dat", "a/b.dat"),
            ("./a//./b.dat", "a/b.dat"),
            ("game/conifer.dat", "game/conifer.dat"),
            ("game/com10.dat", "game/com10.dat"),
        ] {
            assert_eq!(clean_rel(input).unwrap(), want, "{input:?}");
        }
        for bad in [
            "",
            ".",
            "..",
            "../outside.dat",
            "a/../outside.dat",
            "/abs/evil.dat",
            "C:evil.dat",
            "C:/evil.dat",
            "game/",
            "a\\",
            "a/./",
            "game/./",
            "NUL",
            "nul",
            "Game/Aux.txt",
            "game/CoM1",
            "game/lpt9",
            "game/trailing.",
            "game/trailing ",
            "dir./file.dat",
            "dir /file.dat",
            "game/a:b",
        ] {
            let got = clean_rel(bad);
            assert!(got.is_err(), "{bad:?} must be refused, got {got:?}");
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

    /// G5: the cached root resolves the base once, and a base that does not
    /// exist yet keeps the same "nearest existing ancestor + missing tail"
    /// form as `contained_join`. A root built after the base is created sees the
    /// created form; the cached one is the caller's to refresh.
    #[test]
    fn path_root_keeps_the_missing_base_lifecycle_and_refreshes_after_creation() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_path_root", "cache");
        let missing = dir.join("not-created-yet");
        let before = PathRoot::new(&missing).unwrap();
        assert_eq!(
            before.join("game/foo.dat").unwrap(),
            missing.join("game/foo.dat")
        );
        std::fs::create_dir_all(&missing).unwrap();
        let after = PathRoot::new(&missing).unwrap();
        assert_eq!(
            after.join("game/foo.dat").unwrap(),
            missing.join("game/foo.dat")
        );
        // The cached form is still a valid containment check after the base
        // appears; it just does not carry the new resolution.
        assert!(before.join("game/foo.dat").is_ok());
        // And an escape is refused through the cached form too.
        assert!(after.join("../outside.dat").is_err());
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
}
