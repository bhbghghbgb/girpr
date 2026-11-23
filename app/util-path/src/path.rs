//! Rel-path spelling for on-disk walks and server input.
//!
//! One rule: an on-disk rel is the walk path stripped of its base, converted
//! lossily to a string and spelled with `/` ([`rel_from_base`]); a server rel is
//! validated and normalised by [`clean_rel`]; and a rel directory joined to a
//! name uses [`join_rel`]. The two spellings meet at the scope, which matches
//! the `/` form both produce.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

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
    Ok(PathBuf::from(validate_rel(rel)?))
}

/// girpr's rule: the full reserved set (including `CLOCK$`) with trailing
/// dot/space stripped before comparing. The shared validator owns the shape
/// checks and their order; this is this program's device rule.
const DEVICE_NAMES: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"];

/// Validate a server-controlled rel and return the cleaned `/`-spelled form.
///
/// The normalisation (`\` → `/`), the `.` removal and the empty-component
/// collapse happen in the shared validator; the caller stores the string it
/// returns. A leading `.` must not survive: the plan stores this spelling and
/// the purge walk reports `game/foo.dat`, so a `./` left in the plan would
/// make the cleanup delete the file this run just repaired.
fn validate_rel(rel: &str) -> Result<String> {
    let policy = girpr_path_shape::ShapePolicy {
        normalization: girpr_path_shape::Normalization::Normalizing,
        device_names: DEVICE_NAMES,
        device_trim: true,
        reject_device: true,
    };
    girpr_path_shape::validate(rel, &policy).map_err(|problem| rel_problem(problem, rel))
}

/// Map a lexical reason to this program's error wording.
fn rel_problem(problem: girpr_path_shape::ShapeProblem, rel: &str) -> anyhow::Error {
    use girpr_path_shape::ShapeProblem as P;
    match problem {
        P::Empty => anyhow::anyhow!("path is empty"),
        P::TrailingSeparator => anyhow::anyhow!("path has a trailing separator: {rel}"),
        P::DrivePrefix => anyhow::anyhow!("path has a drive prefix: {rel}"),
        P::Absolute => anyhow::anyhow!("path is absolute: {rel}"),
        P::WindowsFold => anyhow::anyhow!("path has a Windows-trailing dot or space: {rel}"),
        P::Colon => anyhow::anyhow!("path has a Windows alternate-stream separator: {rel}"),
        P::Device => anyhow::anyhow!("path names a Windows device: {rel}"),
        P::Parent | P::DotComponent | P::Backslash | P::Control | P::EmptyComponent => {
            anyhow::anyhow!("path is not a plain relative path: {rel}")
        }
        P::NoName => anyhow::anyhow!("path names no file: {rel}"),
    }
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
    validate_rel(rel)
}

/// The `/`-separated rel spelling of an on-disk path under `base`.
///
/// This is the on-disk counterpart of [`clean_rel`]: `base` is the walk root
/// the path came from, and the result is what remains after stripping it, with
/// `\` reported as `/`. The purge and the post-phase walk the same tree, so
/// they must spell it the same way.
///
/// The string conversion is **lossy on purpose**. An install can hold a name
/// that is not UTF-8, while the data plane and the log need a string; the path
/// on disk stays the authority for the operation, and this is only how the rel
/// is spelled in a plan, a log line or a scope match. `to_str().unwrap()` would
/// refuse the whole walk instead of reporting it.
///
/// Returns `None` when `path` is not under `base`, so a caller can skip a path
/// it cannot spell. A path equal to `base` yields `Some(String::new())`: it is
/// under the base, but the empty rel names no file, and the walks skip the
/// base itself.
pub fn rel_from_base(path: &Path, base: &Path) -> Option<String> {
    Some(rel_pair_from_base(path, base)?.0)
}

/// Strip `base` once and return both spellings a caller needs: the lossy `/`
/// rel for the scope key, and the byte-exact relative path for the filesystem
/// operation.
///
/// [`rel_from_base`] is the key-only half. A caller that keys a path and then
/// moves it must not strip the base twice: the second strip could only
/// disagree, and rebuilding the destination from the lossy string would
/// rewrite a name that is not UTF-8. The path half keeps the source's own
/// bytes, so the move lands on the same name.
///
/// The contract matches [`rel_from_base`]: `None` outside `base`, and
/// `Some(("", ""))` for `base` itself.
pub fn rel_pair_from_base<'a>(path: &'a Path, base: &Path) -> Option<(String, &'a Path)> {
    let rel = path.strip_prefix(base).ok()?;
    Some((lossy_slash(rel), rel))
}

/// Spell a path as a lossy string with `/` separators.
///
/// The conversion is shared by [`rel_from_base`] and [`join_rel`], so both
/// halves of a rel use the same separator rule on every host.
fn lossy_slash(path: &Path) -> String {
    // The shared crate owns the rewrite mechanic; this call site owns the
    // policy: rels are `/`-spelled on every host.
    girpr_fs_names::forward_slash(&path.to_string_lossy(), true)
}

/// Join a rel directory and a name into the `/`-separated form the scope
/// matches on.
///
/// `dir_rel` is already in the clean `/` form ([`clean_rel`]) and must not end
/// with a separator: that is the form [`clean_rel`] returns, and this helper
/// adds the one separator itself. `name` is a path fragment from a walk and is
/// converted lossily, with `\` reported as `/`. An empty `dir_rel` yields
/// `name` alone, so the result never starts with `/`; a non-empty one adds
/// exactly one separator, never `//`.
pub fn join_rel(dir_rel: &str, name: &Path) -> String {
    let name = lossy_slash(name);
    if dir_rel.is_empty() {
        name
    } else {
        format!("{dir_rel}/{name}")
    }
}

/// The key a path comparison uses when two spellings may name one file.
///
/// Windows paths are case-insensitive, so the plan's dedup and the purge's
/// keep comparison both fold case there; on Unix they name distinct files and
/// the key is the path itself. The two must agree, so the rule lives here
/// instead of once per module.
///
/// The Windows fold is **ASCII-only**. Server manifest paths are ASCII in
/// practice, and `str::to_lowercase` is a Unicode case mapping rather than the
/// filesystem's relation: it maps spellings Windows keeps apart onto one key
/// (the Kelvin sign `\u{212A}` and `k`, for example), which over-merges in the
/// plan and over-keeps in the purge. So the rule is "fold ASCII case, compare
/// every other byte exactly".
///
/// Non-ASCII case variants are therefore deliberately **distinct** keys, even
/// where the filesystem may consider them one file. That is the trade: this is
/// a map key, not a containment check, and a non-ASCII install gets exact-byte
/// equality rather than a guess. [`contained_join`] owns the filesystem
/// decision.
///
/// The choice is compile-time (`cfg!`), so both branches are type-checked on
/// every host. Keep them in sync: a change to the Windows fold must be
/// reflected in the Unix branch, or the platforms stop answering the same
/// question.
///
/// This is **not** the folding `--include` / `--exclude` matching uses. That
/// match is case-insensitive through the `glob` crate's per-character
/// `eq_ignore_ascii_case` (the product's `girpr-repair-plan` path scope), which is
/// ASCII-only like this one but applies on every host. The two rules differ
/// deliberately. A glob is a containment predicate over a user-written pattern,
/// where matching broadly is the point. This function is a map key for path
/// *identity*, where over-merging silently drops or keeps the wrong file, so it
/// folds only where the filesystem does (Windows) and stays ASCII-only there.
pub fn path_key(rel: &str) -> String {
    // The shared crate owns the ASCII-fold mechanic; this call site owns the
    // rule: fold only where the filesystem does (Windows), never on a host whose
    // names are case-sensitive. The sync tool's `path_key` passes `true` for its
    // case-insensitive pairing option; the two behaviours stay separate.
    girpr_fs_names::fold_ascii_case(rel, cfg!(windows))
}

/// Does this name end in a dot or a space, which Windows strips when it
/// resolves the name?
///
/// Windows aliases `config.` and `config ` to `config`, so a spelling with
/// either trailing character names the same file as the stripped form. The
/// check is lexical and runs on every host: the install is a Windows tree even
/// when girpr itself runs elsewhere. [`contained_rel`] applies it to every
/// component, and [`any_component_has_windows_trailing_dot_or_space`] applies
/// it to every component of a writer's target.
pub(crate) fn has_windows_trailing_dot_or_space(part: &str) -> bool {
    // The shared crate owns the trailing-character check; this call site owns
    // the rule: apply it to every component name, on every host.
    girpr_fs_names::has_trailing_dot_or_space(part)
}

/// Does any component of `path` that names a file end in a Windows-trailing
/// dot or space?
///
/// This is `has_windows_trailing_dot_or_space` over a whole path, for the
/// writers whose target did not come through [`contained_rel`]: a caller-built
/// path can carry the alias spelling in any directory component, not only in
/// the final one. Windows strips the character when it resolves each
/// component, so a directory spelled `dir.` aliases `dir` just as a file name
/// does.
///
/// A prefix (a drive or a UNC share) and the root name a filesystem, not a
/// file, so they are not checked. A `..` parent is not a name either: it walks
/// out of a directory rather than naming one, and it would otherwise match this
/// rule because it ends in a dot, refusing every path that climbs. `components()`
/// drops `.` and repeated separators for the same reason.
pub fn any_component_has_windows_trailing_dot_or_space(path: &Path) -> bool {
    path.components().any(|c| match c {
        Component::Normal(part) => {
            let bytes = part.as_encoded_bytes();
            // Common case: an all-ASCII component's encoded bytes *are* the
            // string, so a trailing `.` or ` ` is exactly its last byte. This
            // avoids the lossy conversion on every ordinary path component.
            if bytes.is_ascii() {
                matches!(bytes.last(), Some(b'.' | b' '))
            } else {
                // Non-ASCII (or ill-formed) names keep the original rule. `.`
                // and ` ` are ASCII and can never be the tail of a multi-byte
                // character, but deferring to the string keeps every edge case
                // — including Windows WTF-8 unpaired surrogates — unchanged.
                has_windows_trailing_dot_or_space(&part.to_string_lossy())
            }
        }
        _ => false,
    })
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
///
/// The resolved form of each parent directory is cached as well, keyed by the
/// unresolved parent, so files that share a directory resolve that directory
/// once. The cache is owned by this root and shared only by its clones, so it
/// lives for one phase of one invocation and cannot carry a resolution from a
/// previous run into this one. Its entries are valid containment references
/// even when the tree grows during a run: a directory that appears later only
/// shortens the "missing tail" a cached parent carries, and either spelling
/// stays under (or outside) the resolved base.
#[derive(Debug, Clone)]
pub struct PathRoot {
    base: PathBuf,
    resolved: PathBuf,
    /// One resolution per parent directory, shared by every clone of the root.
    parents: Arc<Mutex<HashMap<PathBuf, PathBuf>>>,
}

impl PathRoot {
    /// Resolve `base` once.
    pub fn new(base: &Path) -> Result<Self> {
        let resolved = resolve_existing_prefix(base)
            .with_context(|| format!("resolve the game dir {}", base.display()))?;
        Ok(Self {
            base: base.to_path_buf(),
            resolved,
            parents: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Build a root for a path **already known to be canonical**.
    ///
    /// The caller must have obtained `base` from a successful `canonicalize`
    /// (or know it is one, e.g. the canonicalizing half of a phase that just
    /// resolved it). Skipping the resolution here avoids a second canonicalize
    /// of a path whose canonical form is already in hand. Passing a non-canonical
    /// base would make the containment comparison use a non-canonical reference,
    /// so this is a separate, narrowly-scoped constructor rather than a default.
    pub fn new_canonical(base: &Path) -> Self {
        Self {
            base: base.to_path_buf(),
            resolved: base.to_path_buf(),
            parents: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Join a server-controlled relative path under this root, refusing
    /// anything that resolves outside it.
    ///
    /// This is the per-file half of [`contained_join`]: the lexical check is
    /// still [`contained_rel`], and the joined path is still resolved and
    /// compared with the resolved base.
    pub fn join(&self, rel: &str) -> Result<PathBuf> {
        self.join_clean(&contained_rel(rel)?)
    }

    /// Join a rel that `build_plan` already cleaned and validated.
    ///
    /// This skips the lexical half of [`join`](Self::join) — the `contained_rel`
    /// → `validate_rel` → path-shape walk that allocates three collections per
    /// file. The filesystem half is unchanged: the parent is resolved through
    /// this root's cache and the leaf is still resolved and checked against the
    /// base, so a symlink planted inside the install is still refused.
    ///
    /// Only pass rels returned by [`clean_rel`](crate::clean_rel) or produced by
    /// `build_plan`. An untrusted rel must use [`join`](Self::join), which runs
    /// the full lexical validation. A debug build asserts the precondition so a
    /// misuse fails loudly in tests; release builds pay only for the join.
    pub fn join_validated(&self, rel: &str) -> Result<PathBuf> {
        #[cfg(debug_assertions)]
        debug_assert!(
            clean_rel(rel).is_ok_and(|cleaned| cleaned == rel),
            "join_validated requires a rel already cleaned by build_plan"
        );
        self.join_clean(Path::new(rel))
    }

    /// Resolve a parent directory, caching its resolved form for this root's
    /// lifetime. `resolve_existing_prefix` resolves the nearest existing
    /// ancestor and re-appends the missing tail, so an absent parent is handled
    /// exactly as it is for the whole joined path.
    fn resolve_parent(&self, parent: &Path) -> Result<PathBuf> {
        if let Some(hit) = self
            .parents
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(parent)
        {
            return Ok(hit.clone());
        }
        let resolved = resolve_existing_prefix(parent)
            .with_context(|| format!("resolve {}", parent.display()))?;
        self.parents
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(parent.to_path_buf(), resolved.clone());
        Ok(resolved)
    }

    /// The filesystem half of [`join`](Self::join) and
    /// [`join_validated`](Self::join_validated), after the lexical check.
    ///
    /// The parent is resolved once per directory via [`Self::resolve_parent`].
    /// The leaf is still resolved on its own: a file that is itself a symlink to
    /// a path outside the install must be refused even though its parent stays
    /// inside it. Only after both halves are resolved is the containment
    /// comparison made, so the check is exactly as strict as resolving the
    /// whole joined path was — the cache removes repeated walks, not the check.
    fn join_clean(&self, rel: &Path) -> Result<PathBuf> {
        let joined = self.base.join(rel);
        let (parent, name) = match (joined.parent(), joined.file_name()) {
            (Some(parent), Some(name)) => (parent, name),
            _ => anyhow::bail!("path names no file: {}", joined.display()),
        };
        let resolved_parent = self.resolve_parent(parent)?;
        let leaf = resolved_parent.join(name);
        // A missing leaf is the common case for a repair (the file is about to
        // be written). The cached parent already carries any missing tail, so
        // that spelling is the resolved result. An existing leaf is resolved
        // here so a symlink at the leaf cannot slip past the comparison.
        let resolved = match std::fs::canonicalize(&leaf) {
            Ok(resolved) => resolved,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => resolved_parent,
            Err(e) => return Err(e).with_context(|| format!("resolve {}", leaf.display())),
        };
        anyhow::ensure!(
            resolved.starts_with(&self.resolved),
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
    PathRoot::new(base)?.join_clean(&rel)
}

/// The legacy `.girpr_tmp` sibling name, kept only as a test fixture.
///
/// Production publishing no longer names a temp itself: `tempfile` owns the
/// name and the cleanup for the config/scan/plan writers and the audio copy
/// fallback. This helper remains under `cfg(test)` because a few tests still
/// build the old spelling as a fixture (a purge-classification row, and the
/// "no temp sibling" assertions). It is **not** a description of any temp the
/// product creates — a fresh temp has a crate-chosen, random name.
///
/// `with_extension` replaces the real extension (`scan.txt` ->
/// `scan.girpr_tmp`) and has its own surprises on multi-dot names; appending to
/// the `OsString` keeps the whole name, so `config.ini` becomes
/// `config.ini.girpr_tmp` and `plan.v1.json` becomes
/// `plan.v1.json.girpr_tmp`. It also stays lossless for a non-UTF-8 name.
///
/// A path with **no file name** still gets a name, but it is not a sibling of a
/// file. The fallback prefix is empty, and `with_file_name` only removes a real
/// file name before pushing the suffix; the path's directories stay as the
/// prefix. So `Audio/res/..` becomes `Audio/res/../.girpr_tmp`, which still
/// contains the `..` before resolution and resolves to `Audio/.girpr_tmp` —
/// beside `res`, not inside it — and the empty path becomes `.girpr_tmp` in the
/// current directory.
#[cfg(any(test, feature = "test-support"))]
pub fn temp_sibling(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(TEMP_SUFFIX);
    path.with_file_name(name)
}

/// The suffix [`temp_sibling`] appends.
#[cfg(any(test, feature = "test-support"))]
const TEMP_SUFFIX: &str = ".girpr_tmp";

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
    // Both fixture families report a host refusal the shared way, so the
    // import is used on every host.
    #[cfg(windows)]
    use crate::junction_support::{junction_fixture_decision, require_junction, try_junction_dir};
    #[cfg(unix)]
    use crate::symlink_support::{require_symlink, symlink_fixture_decision, try_symlink_dir};
    use crate::test_support::FixtureRefusal;

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

    /// The byte-based fast path in `any_component_has_windows_trailing_dot_or_space`
    /// is defined as equal to the original lossy check. Compare the two directly
    /// over ordinary, non-ASCII and structural-component spellings.
    fn lossy_trailing_reference(path: &Path) -> bool {
        path.components().any(|c| match c {
            Component::Normal(part) => has_windows_trailing_dot_or_space(&part.to_string_lossy()),
            _ => false,
        })
    }

    #[test]
    fn any_component_trailing_check_matches_lossy_reference() {
        for input in [
            "Audio/res/foo.pck",
            "Audio/res/foo.",
            "Audio/res/foo ",
            "dir./foo.pck",
            "Audio/res/dir /foo.pck",
            "Audio/res/../foo.pck",
            "Audio/res/./foo.pck",
            "a..b",
            "a. ",
            "a .",
            "trailing\\.",
            "café.pck",
            "café.",
            "café ",
            "café",
            "日本語/",
            "",
            ".",
            "..",
        ] {
            let path = Path::new(input);
            assert_eq!(
                any_component_has_windows_trailing_dot_or_space(path),
                lossy_trailing_reference(path),
                "disagreement for {input:?}"
            );
        }
    }

    /// Ill-formed Unix names exercise the fallback: invalid bytes are replaced
    /// with U+FFFD by the lossy string, and the byte path must not disagree.
    #[cfg(unix)]
    #[test]
    fn any_component_trailing_check_matches_lossy_reference_on_non_utf8() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        for bytes in [
            b"foo.".to_vec(),
            b"foo ".to_vec(),
            vec![0xFF, b'.'],
            vec![0xFF, b' '],
            vec![0xFF],
            vec![b'c', 0xC3, 0xA9],
            vec![b'c', 0xC3, 0xA9, b'.'],
            vec![0xED, 0xA0, 0x80, b'.'],
        ] {
            let path = Path::new(OsStr::from_bytes(&bytes));
            assert_eq!(
                any_component_has_windows_trailing_dot_or_space(path),
                lossy_trailing_reference(path),
                "disagreement for {bytes:?}"
            );
        }
    }

    /// A Windows component is WTF-8; an unpaired surrogate becomes U+FFFD in
    /// the lossy string, but an ASCII trailing byte still decides the answer.
    #[cfg(windows)]
    #[test]
    fn any_component_trailing_check_matches_lossy_reference_on_wtf8() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        for wide in [
            vec![0xD800, b'.' as u16],
            vec![0xD800, b' ' as u16],
            vec![0xD800],
            vec![b'f' as u16, b'o' as u16, b'o' as u16, b'.' as u16],
        ] {
            let os = OsString::from_wide(&wide);
            let path = Path::new(&os);
            assert_eq!(
                any_component_has_windows_trailing_dot_or_space(path),
                lossy_trailing_reference(path),
                "disagreement for {wide:?}"
            );
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

    /// The on-disk rel spelling is one rule for both walks: strip the walk
    /// base, then report `/`. A path outside the base has no spelling and
    /// reports `None`; the base itself reports the empty rel.
    #[test]
    fn rel_from_base_strips_the_walk_base_and_uses_slashes() {
        let base = Path::new("Game");
        assert_eq!(
            rel_from_base(&base.join("a.pck"), base).as_deref(),
            Some("a.pck")
        );
        assert_eq!(
            rel_from_base(&base.join("Audio/cache/sub/a.pck"), base).as_deref(),
            Some("Audio/cache/sub/a.pck")
        );
        // The base names no file: it is under itself, and the empty rel says so.
        assert_eq!(rel_from_base(base, base).as_deref(), Some(""));
        // A sibling, or an ancestor, cannot be spelled relative to the base.
        assert_eq!(rel_from_base(Path::new("Other/a.pck"), base), None);
        assert_eq!(rel_from_base(base, &base.join("sub")), None);
        // Stripping is by path component, not by shared prefix text: `GameX`
        // shares the letters with `Game` but is a sibling, not a descendant.
        assert_eq!(rel_from_base(Path::new("GameX/a.pck"), base), None);
        // A true component prefix still strips, including a nested base.
        let nested_base = base.join("Audio");
        assert_eq!(
            rel_from_base(&nested_base.join("cache/a.pck"), &nested_base).as_deref(),
            Some("cache/a.pck")
        );
    }

    /// The pair helper strips the base once and returns both halves a moving
    /// caller needs: the `/` key and the byte-exact path. A name that is not
    /// UTF-8 is lossy in the key and untouched in the path.
    #[test]
    fn rel_pair_from_base_returns_the_key_and_the_byte_exact_path() {
        let base = Path::new("Game");
        let nested = base.join("Audio/cache/sub/a.pck");
        let (key, rel) = rel_pair_from_base(&nested, base).unwrap();
        assert_eq!(key, "Audio/cache/sub/a.pck");
        assert_eq!(rel, Path::new("Audio/cache/sub/a.pck"));

        // The base itself yields the empty rel in both spellings.
        let (key, rel) = rel_pair_from_base(base, base).unwrap();
        assert_eq!(key, "");
        assert_eq!(rel, Path::new(""));

        // Outside the base there is no pair to return.
        assert_eq!(rel_pair_from_base(Path::new("Other/a.pck"), base), None);
        // Shared prefix text is not a shared component: `GameX` is a sibling.
        assert_eq!(rel_pair_from_base(Path::new("GameX/a.pck"), base), None);
        // A true component prefix still yields the pair, also for a nested base.
        let nested_base = base.join("Audio");
        let nested_path = nested_base.join("cache/a.pck");
        let (key, rel) = rel_pair_from_base(&nested_path, &nested_base).unwrap();
        assert_eq!(key, "cache/a.pck");
        assert_eq!(rel, Path::new("cache/a.pck"));

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::OsStr::from_bytes(b"bad\xff.pck");
            let path = base.join(name);
            let (key, rel) = rel_pair_from_base(&path, base).unwrap();
            assert_eq!(key, "bad\u{fffd}.pck");
            assert_eq!(rel.as_os_str().as_bytes(), b"bad\xff.pck");
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::{OsStrExt, OsStringExt};
            let name =
                std::ffi::OsString::from_wide(&[0x62, 0x61, 0x64, 0xD800, 0x2E, 0x70, 0x63, 0x6B]);
            let path = base.join(&name);
            let (key, rel) = rel_pair_from_base(&path, base).unwrap();
            assert_eq!(key, "bad\u{fffd}.pck");
            assert_eq!(
                rel.as_os_str().encode_wide().collect::<Vec<u16>>(),
                name.encode_wide().collect::<Vec<u16>>()
            );
        }
    }

    /// A name that is not UTF-8 is still reported: the rel is lossy by design,
    /// so the scope and the log get a string instead of the walk refusing the
    /// file. Both platforms can build the fixture from an unpaired surrogate or
    /// an invalid byte.
    #[test]
    fn rel_from_base_reports_a_non_utf8_name_lossily() {
        let base = Path::new("Game");
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::OsStr::from_bytes(b"bad\xff.pck");
            assert_eq!(
                rel_from_base(&base.join(name), base).unwrap(),
                "bad\u{fffd}.pck"
            );
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            // An unpaired surrogate has no UTF-8 encoding.
            let name =
                std::ffi::OsString::from_wide(&[0x62, 0x61, 0x64, 0xD800, 0x2E, 0x70, 0x63, 0x6B]);
            assert_eq!(
                rel_from_base(&base.join(&name), base).unwrap(),
                "bad\u{fffd}.pck"
            );
        }
    }

    /// The join helper is the post-phase's spelling: a rel dir and a walk name,
    /// one separator, `/` on every platform, and an empty dir adds none.
    #[test]
    fn join_rel_adds_one_separator_and_normalises_the_name() {
        assert_eq!(join_rel("", Path::new("a.pck")), "a.pck");
        assert_eq!(
            join_rel("Audio/cache", Path::new("sub/a.pck")),
            "Audio/cache/sub/a.pck"
        );
        // A name Windows spelled with `\` is reported in the `/` form.
        assert_eq!(
            join_rel("Audio/cache", Path::new("sub\\a.pck")),
            "Audio/cache/sub/a.pck"
        );
        // The clean form clean_rel returns has no trailing separator, so the
        // one join adds is the only one in the result.
        let dir = clean_rel("Audio//cache").unwrap();
        assert!(!dir.ends_with('/'));
        assert_eq!(
            join_rel(&dir, Path::new("sub/a.pck")),
            "Audio/cache/sub/a.pck"
        );
    }

    /// Every atomic writer names its temp the same way: the suffix is appended
    /// to the whole file name, so no extension is replaced and a multi-dot name
    /// keeps its shape.
    #[test]
    fn temp_sibling_appends_to_the_whole_name() {
        for (input, want) in [
            ("config.ini", "config.ini.girpr_tmp"),
            ("scan.txt", "scan.txt.girpr_tmp"),
            ("a.b.txt", "a.b.txt.girpr_tmp"),
            ("plan.v1.json", "plan.v1.json.girpr_tmp"),
            ("scan", "scan.girpr_tmp"),
        ] {
            assert_eq!(
                temp_sibling(Path::new(input)),
                PathBuf::from(want),
                "{input:?}"
            );
        }
        // The sibling stays beside the target, in the same directory.
        assert_eq!(
            temp_sibling(Path::new("Audio/scan.txt")),
            PathBuf::from("Audio/scan.txt.girpr_tmp")
        );
    }

    /// A path with no file name still names its sibling, but the shape depends
    /// on what the path already says: `with_file_name` only pops a real file
    /// name, so a path that ends in `..` keeps that component and gains the
    /// suffix after it.
    ///
    /// `""` gains the suffix directly (`.girpr_tmp`). `Audio/res/..` becomes
    /// `Audio/res/../.girpr_tmp`, which resolves to `Audio/.girpr_tmp` — beside
    /// `res`, not inside it — so the result is not "beside a name" for such a
    /// path. The post-phase's copy writer refuses a no-file-name target before
    /// asking for a temp for exactly that reason.
    #[test]
    fn temp_sibling_names_the_no_file_name_fallback() {
        let mut want = std::ffi::OsString::new();
        want.push(TEMP_SUFFIX);
        let empty = Path::new("");
        assert!(empty.file_name().is_none());
        assert_eq!(temp_sibling(empty).file_name(), Some(want.as_os_str()));
        assert_eq!(temp_sibling(empty), PathBuf::from(want.as_os_str()));

        // The `..` survives into the returned spelling before resolution.
        let dotdot = Path::new("Audio/res/..");
        assert!(dotdot.file_name().is_none());
        assert_eq!(
            temp_sibling(dotdot),
            PathBuf::from("Audio/res/../.girpr_tmp")
        );

        // Resolved, that spelling lands beside `res`, not inside it: the
        // fallback name is in the directory the `..` walks up to. The fixture
        // makes both sides resolvable, so the landing location is pinned
        // instead of only its unresolved spelling.
        let dir = girpr_tempdir::TempDir::new("girpr_test_temp_sibling", "dotdot");
        std::fs::create_dir_all(dir.join("Audio/res")).unwrap();
        let in_fixture = dir.join("Audio/res/..");
        assert!(in_fixture.file_name().is_none());
        let landed = resolve_existing_prefix(&temp_sibling(&in_fixture)).unwrap();
        let want = resolve_existing_prefix(&dir.join("Audio/.girpr_tmp")).unwrap();
        assert_eq!(
            landed, want,
            "the fallback temp lands beside `res`, not inside it"
        );
        assert!(
            landed.ends_with(Path::new("Audio").join(".girpr_tmp")),
            "the resolved location is Audio/.girpr_tmp: {}",
            landed.display()
        );
    }

    /// The suffix is pushed onto the `OsString`, so a name that is not UTF-8
    /// keeps every byte instead of being round-tripped through a lossy string.
    #[test]
    fn temp_sibling_keeps_a_non_utf8_name() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::OsStr::from_bytes(b"scan\xff.txt");
            let got = temp_sibling(Path::new(name));
            assert_eq!(
                got.file_name().unwrap().as_bytes(),
                b"scan\xff.txt.girpr_tmp"
            );
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::{OsStrExt, OsStringExt};
            // An unpaired surrogate has no UTF-8 encoding.
            let name = std::ffi::OsString::from_wide(&[0x73, 0x63, 0xD800, 0x2e, 0x74, 0x78, 0x74]);
            let got = temp_sibling(Path::new(&name));
            let mut want: Vec<u16> = name.encode_wide().collect();
            want.extend(".girpr_tmp".encode_utf16());
            assert_eq!(
                got.file_name().unwrap().encode_wide().collect::<Vec<u16>>(),
                want
            );
        }
    }

    /// The key is the plan's and the purge's only path-equality rule, so the
    /// fold it applies is pinned here: ASCII case where the platform folds it,
    /// and no Unicode case mapping on either platform.
    #[test]
    fn path_key_folds_ascii_case_only_on_windows() {
        // ASCII case variants name one file on Windows and two on Unix.
        let ascii_same = path_key("Game/A.DAT") == path_key("game/a.dat");
        assert_eq!(
            ascii_same,
            cfg!(windows),
            "ASCII case folding must follow the host"
        );

        // `str::to_lowercase` maps the Kelvin sign to `k`, but Windows keeps
        // the two names apart, so the key must not collapse them.
        assert_ne!(path_key("\u{212A}"), path_key("k"));

        // A non-ASCII name is compared byte for byte; only its ASCII letters
        // are folded.
        let mixed = path_key("Game/Ä.DAT");
        if cfg!(windows) {
            assert_eq!(mixed, "game/Ä.dat");
        } else {
            assert_eq!(mixed, "Game/Ä.DAT");
        }
        assert_ne!(path_key("Ä"), path_key("ä"));
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

    /// The opt-in fast path skips only lexical revalidation: for a rel already
    /// cleaned by `build_plan` it lands on the same path as the full `join`,
    /// against an existing base, a missing base and a canonical-built root.
    #[test]
    fn join_validated_matches_join_for_clean_rels() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_path_root", "validated");
        let root = PathRoot::new(&dir).unwrap();
        for rel in ["a.dat", "game/foo.dat", "game/sub/deep.dat"] {
            assert_eq!(
                root.join(rel).unwrap(),
                root.join_validated(rel).unwrap(),
                "{rel}"
            );
        }

        let missing = dir.join("not-created-yet");
        let missing_root = PathRoot::new(&missing).unwrap();
        assert_eq!(
            missing_root.join("game/foo.dat").unwrap(),
            missing_root.join_validated("game/foo.dat").unwrap()
        );

        let canonical = dir.canonicalize().unwrap();
        let canonical_root = PathRoot::new_canonical(&canonical);
        assert_eq!(
            canonical_root.join_validated("game/foo.dat").unwrap(),
            PathRoot::new(&canonical)
                .unwrap()
                .join_validated("game/foo.dat")
                .unwrap()
        );
    }

    /// Siblings in one directory reuse the cached parent resolution, and a
    /// parent created later only shortens its missing tail — the resolved form
    /// and the containment answer stay the same.
    #[test]
    fn join_validated_shares_a_parent_and_survives_parent_creation() {
        let dir = girpr_tempdir::TempDir::new("girpr_test_path_root", "shared_parent");
        let root = PathRoot::new(&dir).unwrap();
        let a = root.join_validated("game/sub/a.dat").unwrap();
        let b = root.join_validated("game/sub/b.dat").unwrap();
        assert_eq!(a.parent(), b.parent());
        assert_eq!(a, dir.join("game/sub/a.dat"));
        assert_eq!(b, dir.join("game/sub/b.dat"));

        std::fs::create_dir_all(dir.join("game/sub")).unwrap();
        assert_eq!(root.join_validated("game/sub/a.dat").unwrap(), a);
    }

    /// The cache removes repeated parent walks, not the containment check: a
    /// leaf that is itself a symlink out of the install is still refused, even
    /// though its parent resolved inside it.
    #[cfg(unix)]
    #[test]
    fn join_validated_refuses_a_leaf_symlink_out_of_the_base() {
        let outside = girpr_tempdir::TempDir::new("girpr_test_path_root", "outside_leaf");
        let dir = girpr_tempdir::TempDir::new("girpr_test_path_root", "leaf");
        if let Err(e) = try_symlink_dir(&outside, &dir.join("escape")) {
            symlink_fixture_decision(e, require_symlink(), FixtureRefusal::Host);
            return;
        }
        let root = PathRoot::new(&dir).unwrap();
        // Warm the cache for the parent (the base itself) with a clean sibling.
        assert!(root.join_validated("ok.dat").is_ok());
        assert!(root.join("escape").is_err());
        assert!(root.join_validated("escape").is_err());
    }

    /// A symlink inside the install that points out of it is an escape even
    /// though the path is lexically relative. The case needs a symlink, and
    /// a host that provably refuses one cannot confirm the escape, so the
    /// case is skipped with the reason rather than failed — unless
    /// `GIRPR_TEST_REQUIRE_SYMLINK` says it must run. Every other failure is
    /// a broken test environment and fails loudly.
    #[cfg(unix)]
    #[test]
    fn contained_join_resolves_a_symlink_and_refuses_the_escape() {
        let outside = girpr_tempdir::TempDir::new("girpr_test_contained", "outside");
        let dir = girpr_tempdir::TempDir::new("girpr_test_contained", "link");
        if let Err(e) = try_symlink_dir(&outside, &dir.join("game")) {
            // The shared decision: a proven host refusal skips the case, and
            // every other failure fails it loudly.
            symlink_fixture_decision(e, require_symlink(), FixtureRefusal::Host);
            return;
        }
        let got = contained_join(&dir, "game/evil.dat");
        assert!(got.is_err(), "a symlink out of the base must be refused");
        // A symlink that stays inside is still fine.
        std::fs::create_dir_all(dir.join("real")).unwrap();
        let alias = dir.join("alias");
        // The first link proved this host can make one, so a refusal here is
        // a broken fixture, not a host skip.
        try_symlink_dir(&dir.join("real"), &alias).expect("this host already created one symlink");
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
    /// Junctions need no privilege (unlike symlinks), but a host that provably
    /// refuses to create one cannot confirm the escape here, so that case is
    /// skipped rather than failed unless `GIRPR_TEST_REQUIRE_JUNCTION` is
    /// present. A failure whose cause cannot be proved a host refusal fails
    /// loudly instead of skipping.
    /// Link support is host-dependent — see
    /// docs/03-repair-rust-implementation-plan.md ("hard-link dedup": NTFS
    /// only, exFAT has no hard links).
    #[cfg(windows)]
    #[test]
    fn contained_join_resolves_a_junction_and_refuses_the_escape() {
        let outside = girpr_tempdir::TempDir::new("girpr_test_contained", "junction_out");
        let dir = girpr_tempdir::TempDir::new("girpr_test_contained", "junction_in");
        let link = dir.join("game");
        if let Err(e) = try_junction_dir(&outside, &link) {
            // The shared decision: a proven host refusal skips the case, and
            // every other failure fails it loudly.
            junction_fixture_decision(e, require_junction(), FixtureRefusal::Host);
            return;
        }
        let got = contained_join(&dir, "game/evil.dat");
        assert!(
            got.is_err(),
            "a junction out of the base must be refused: {got:?}"
        );
        // A junction that stays inside is still fine. The first creation proved
        // this host can make one, so a refusal here is a broken fixture, not a
        // host skip.
        std::fs::create_dir_all(dir.join("real")).unwrap();
        let alias = dir.join("alias");
        try_junction_dir(&dir.join("real"), &alias)
            .expect("this host already created one junction");
        assert!(contained_join(&dir, "alias/ok.dat").is_ok());
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
