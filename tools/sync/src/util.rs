//! Small path, time, and filesystem helpers with no girsync-specific state.
//!
//! ## The published surface
//!
//! A rule is published when its answer is public; the comparison a rule is
//! written in is not.
//!
//! - **identity** — [`same_path`] and [`path_within`]: whether two names open
//!   the same place on disk, and whether one path lies inside another.
//! - **keying** — [`path_key`], [`FoldedKey`] and [`folded_keys`]: which
//!   bucket of a map a relative path belongs to, and the index over that
//!   bucket. The answer is public —
//!   [`CasePairing::Lowercase`](crate::planner::CasePairing::Lowercase) pairs
//!   two sides by this key — so the rule behind that answer is published too
//!   rather than left for a caller to spell out, because the ASCII-only fold
//!   is the half of the rule that is easy to get wrong.
//! - **the cache vocabulary** — [`is_record_path`], [`is_cache_name`] and
//!   [`is_cache_rel`]: whether a path names the tool's own state. The answer
//!   is public too: [`crate::effective::Repopulate`].
//!
//! The comparisons those rules are written in stay inside the crate: the
//! component equality, the Unicode lowercase mapping, the ASCII-case
//! comparison and the cache prefix. They are how a rule is written, not rules
//! of their own.

use anyhow::{Result, bail};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

#[cfg(windows)]
use std::ffi::OsStr;

use crate::cache::CACHE_PREFIX;

/// Seconds since `t0`, for `elapsed_s` span fields.
pub fn elapsed_s(t0: std::time::Instant) -> f64 {
    t0.elapsed().as_secs_f64()
}

/// The crate's `/`-separated form of a relative path.
///
/// Every relative path the crate keeps — a walk's `rel`, a plan entry, a path
/// key — is `/`-separated whatever the host. On Windows the filesystem's
/// separator is `\`, so the host spelling has to be rewritten. On POSIX `/` is
/// already the separator and a backslash is an ordinary filename character:
/// rewriting it would turn `\lead.txt` into an absolute path, which replaces the
/// root it is joined onto, and `a\b.txt` into `a/b.txt`, which does not exist on
/// disk. There the input is returned unchanged.
pub(crate) fn slash_rel(rel: &str) -> String {
    slash_rel_for(rel, cfg!(windows))
}

/// The rewrite itself, with the host's separator taken as an argument so the
/// rule can be checked for both hosts on either one.
fn slash_rel_for(rel: &str, windows: bool) -> String {
    // The shared crate owns the rewrite mechanic; this call site owns the
    // policy (the host's separator) and keeps the wrapper whose two arguments
    // the pinned test below asks directly on either host.
    girpr_fs_names::forward_slash(rel, windows)
}

/// The ASCII-case comparison: ASCII case folded, every other byte exact.
///
/// **The crate's one ASCII-case comparison.** It states the **path key's** rule
/// — the one [`path_key`] folds with, and the one every relative-path index
/// uses — as a comparison, for the sites that compare two spellings rather
/// than fold one of them into a key. The hash module's fold over an algorithm
/// name is the same rule, and the case that checks it asks this comparison
/// rather than one of its own.
///
/// **Written out rather than borrowed from `std`.** `eq_ignore_ascii_case`
/// would answer the same question, but a comparison borrowed from `std` is a
/// comparison that *follows* `std`: a `std` that changed which bytes fold
/// would change this rule along with it, and the case that checks the hash
/// module's fold would follow `std` instead of catching it. Writing the rule
/// out here keeps the crate's answer independent of `std`'s.
///
/// It is deliberately **not** the identity rule's comparison. [`component_eq`]
/// keeps a Unicode fallback because it asks whether two spellings open the same
/// place on disk, and on Windows the filesystem really does open `Ärger` and
/// `ärger` as one file. The cache reservation asks a narrower question — is this
/// component one of the tool's own names — and the prefix it matches is ASCII,
/// so the ASCII-only rule says it without borrowing a rule that answers
/// something else.
///
/// **Its users.** The cache reservation's two rules — the prefix helper
/// [`has_cache_prefix`] and its exact-name counterpart [`is_cache_name`] — ask
/// it on Windows only, where the filesystem folds case; on POSIX the two
/// spellings are distinct entries, so the reservation matches exactly there.
/// The borrowed key's equality ([`FoldedKey`]) asks it on every platform,
/// because a key is a key everywhere. And the hash module's fold is checked
/// against it, so the crate's answer to "do these two spellings fold alike" is
/// written once and two sites cannot drift apart over it.
pub(crate) const fn ascii_case_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if ascii_lower(a[i]) != ascii_lower(b[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// One byte, folded to lower case if it is an ASCII upper-case letter.
///
/// The fold is written out rather than borrowed, and so is the classification
/// it rests on: an ASCII upper-case letter is 32 above its lower-case
/// counterpart, and every other byte is itself.
///
/// `to_ascii_lowercase` would do the fold, and `is_ascii_uppercase` would do
/// the classification. Neither is used, because the rule must not borrow
/// `std`'s answer for either half: a `std` that changed which bytes count as
/// upper-case would then be followed by the rule and by the case that checks
/// it at once, and the case would agree with `std` instead of catching it.
#[allow(clippy::manual_is_ascii_check)]
const fn ascii_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' { b + 32 } else { b }
}

/// True when one path component names cache state: it starts with `girpr-cache`,
/// covering the live DB, its backups and its `old-*` snapshots.
///
/// The comparison folds case on Windows only. There the filesystem cannot tell
/// `GIRPR-CACHE` from `girpr-cache`, so a case-only rename of the tool's own
/// cache is the same file and must be reserved the same way. On POSIX the two
/// spellings are distinct entries, and an upper-case one is ordinary content
/// that must not be swept up by the reservation.
///
/// **The fold is the path key's, not the identity rule's.** On Windows the
/// comparison is `ascii_case_eq` — the crate's one ASCII-case comparison, asked
/// here because on POSIX the reservation is an exact prefix match — and it
/// folds ASCII case and compares every other byte exactly: the rule
/// [`path_key`] states. The identity rule ([`component_eq`]) keeps a Unicode
/// fallback, because it answers a different question: do two spellings open
/// the same place on disk. The reservation asks about the tool's own names,
/// and the prefix it matches is ASCII, so the ASCII-only rule is the one that
/// states it. The identity rule's fold would answer a question nobody is
/// asking here.
///
/// **Stated with its exact-name counterpart** [`is_cache_name`]. The two
/// reserve one vocabulary under one spelling rule: this one reserves the whole
/// `girpr-cache*` prefix, and that one asks whether the component is the
/// cache's own name, `girpr-cache`, rather than one of its copies. Both state
/// the ASCII-only comparison, on Windows only, so a reader who follows one
/// finds the other.
fn has_cache_prefix(component: &str) -> bool {
    #[cfg(windows)]
    {
        component
            .get(..CACHE_PREFIX.len())
            .is_some_and(|head| ascii_case_eq(head, CACHE_PREFIX))
    }
    #[cfg(not(windows))]
    {
        component.starts_with(CACHE_PREFIX)
    }
}

/// True when the final path component starts with `girpr-cache`, i.e. the path is
/// a record (a DB directory) rather than a folder root.
///
/// **Published with its exact-name counterpart** [`is_cache_name`]. The two are
/// one vocabulary — *a record* is any `girpr-cache*` name, and *a folder's own
/// cache* is exactly `girpr-cache` — so a caller that can ask one can ask the
/// other, and neither half is left to be spelled twice outside the crate.
pub fn is_record_path(p: &Path) -> bool {
    p.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(has_cache_prefix)
}

/// True when the final path component is the cache's own name, `girpr-cache`.
///
/// A record *is* a cache, and a folder's cache is named exactly `girpr-cache`;
/// a `girpr-cache-backup-*` copy or an `old-` snapshot is a copy of one and not
/// one. This is the exact-name counterpart of [`is_record_path`], which
/// reserves the whole prefix.
///
/// The same spelling rule as that one: case is folded on Windows only, where
/// the filesystem cannot tell `GIRPR-CACHE` from `girpr-cache`, and the two are
/// different names elsewhere. The prefix helper `has_cache_prefix` states the
/// same comparison, so the two halves of the vocabulary cannot drift apart.
///
/// **Public, and published with its counterpart.** The two rules are one
/// vocabulary, and the crate publishes both halves of it: the answer this rule
/// decides is public too — [`crate::effective::Repopulate`] says whether a side
/// is a folder's own cache or not — so a caller that can see the answer can ask
/// the rule behind it. Publishing one half and hiding the other would leave an
/// outside caller to spell this half out again, and this is the half whose
/// spelling is easy to get wrong: the fold is ASCII, and it applies on Windows
/// only.
///
/// The rule has one caller today, [`crate::effective::side_folder`], which
/// decides from it whether a side has a folder to repopulate at all.
pub fn is_cache_name(p: &Path) -> bool {
    p.file_name().and_then(|s| s.to_str()).is_some_and(|name| {
        #[cfg(windows)]
        {
            ascii_case_eq(name, CACHE_PREFIX)
        }
        #[cfg(not(windows))]
        {
            name == CACHE_PREFIX
        }
    })
}

/// True for any relative path with a `girpr-cache*` component: the live DB,
/// its backups, and its `old-*` snapshots. These are never part of a mirror.
///
/// Every component, not only the first. A cache belongs to the root it sits in,
/// so a nested root carries one at `<root>/sub/girpr-cache`, and a followed
/// link exposes its target's cache as `link/girpr-cache`. Both are tool state,
/// not content; copying them would at best duplicate a database and at worst
/// fail on its live lock. The prefix is reserved at every depth, exactly as
/// [`is_record_path`] reserves it for a record's own name. The same spelling
/// rule applies to both: case is folded on Windows only, through
/// `has_cache_prefix`.
pub fn is_cache_rel(rel: &str) -> bool {
    rel.split('/').any(has_cache_prefix)
}

/// The canonical form of an existing path, or the path itself when it cannot
/// be resolved (a missing root still has to reach the caller's own error).
///
/// Used for *identity* checks: two spellings of one target must compare equal
/// before either side is opened, and a canonical root joined with a cache name
/// is a path that need not exist yet. Both sync roots must already be
/// directories by the time this is called, so the final component exists; only
/// the cache name joined afterwards may be absent.
///
/// Canonicalization goes through the shared `girpr_fs_names::canonical_no_verbatim`
/// helper, which resolves links like `std::fs::canonicalize` but leaves off the
/// Windows `\\?\` prefix. Every canonical path this crate produces must be in
/// that one spelling: a junction's own target has no verbatim prefix, so a
/// canonical path that kept one could not be compared with it component-wise.
pub fn canonical_or_raw(p: &Path) -> PathBuf {
    girpr_fs_names::canonical_no_verbatim(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Whether two paths name the same location.
///
/// Component-wise, so `C:\A` does not match `C:\AB`; case-insensitive on
/// Windows, where the filesystem is. Everywhere else this is exact.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let mut ai = a.components();
    let mut bi = b.components();
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if component_eq(x, y) => {}
            _ => return false,
        }
    }
}

/// Whether `child` is `parent` or lies beneath it.
///
/// Component-wise for the same reason as [`same_path`]: a textual prefix would
/// treat `C:\AB` as inside `C:\A`. Case-insensitive on Windows.
pub fn path_within(child: &Path, parent: &Path) -> bool {
    let mut child = child.components();
    for parent in parent.components() {
        match child.next() {
            Some(c) if component_eq(c, parent) => {}
            _ => return false,
        }
    }
    true
}

/// Whether two path components name the same place on disk.
///
/// The **identity** rule, and it is not the path key's rule: the key asks which
/// bucket of a run's map a path belongs to, this asks whether two spellings
/// open the same file.
///
/// Case is folded on Windows, where the filesystem folds it, and only there.
/// The fold the fallback uses is Unicode — deliberately *not* the ASCII-only
/// fold of [`path_key`], which is the key every relative-path index uses. The
/// two rules answer different questions and are meant to disagree. The mapping
/// the fallback compares with is `unicode_lowercase`, which the on-disk
/// spelling lookup compares with too: one implementation for two questions,
/// not one rule.
fn component_eq(a: Component<'_>, b: Component<'_>) -> bool {
    #[cfg(windows)]
    {
        let (a, b) = (a.as_os_str(), b.as_os_str());
        if a == b {
            return true;
        }
        unicode_case_eq(a, b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// The Unicode lowercase mapping the **identity** rule's fallback and the
/// **on-disk spelling** lookup's comparison share.
///
/// The comparison that uses it is [`unicode_case_eq`], so the two sites share
/// the mapping *and* the way they compare with it.
///
/// **A shared implementation, not a shared rule.** The two sites ask different
/// questions, and each keeps its own statement of which: the identity rule
/// asks whether two spellings name the same place on disk, and the on-disk
/// lookup asks which directory entry the operating system would open. What
/// they share is the mapping they compare with, so the mapping is written once
/// here rather than twice in two modules — two copies of the same value can
/// drift apart, and nothing would say so.
///
/// **It is deliberately not the path key's fold.** [`path_key`] folds ASCII
/// case only and is the key every relative-path index uses; this mapping
/// answers a different question — do two spellings name the same place on
/// disk? — and on Windows the filesystem really does treat `Ärger` and `ärger`
/// as one file. The two rules are allowed to disagree, and that is the point:
/// the key keeps those two spellings apart, and this says they are one place.
/// Unifying them would change what a run does — a Unicode key would merge
/// spellings the disk keeps apart, and an ASCII identity rule would stop the
/// overlap and containment checks from seeing a case difference Windows really
/// has.
///
/// It is a *lowercase* mapping rather than a full case fold, and an
/// approximation of the filesystem's own table rather than a copy of it: the
/// lossy conversion also folds two distinct byte sequences that are not valid
/// UTF-8 onto each other, since both become the replacement character. It is
/// the approximation both of its users have always made — the identity rule's
/// fallback and the on-disk lookup's comparison — and it is kept rather than
/// guessed at.
///
/// Windows only, because neither site folds case anywhere else: on POSIX the
/// two spellings are distinct entries.
#[cfg(windows)]
pub(crate) fn unicode_lowercase(s: &OsStr) -> String {
    s.to_string_lossy().to_lowercase()
}

/// The Unicode lowercase comparison: are these two names the same under the
/// mapping [`unicode_lowercase`] applies?
///
/// **The comparison the two sites that share the mapping ask it with.** The
/// identity rule's fallback and the on-disk spelling lookup both ask whether
/// two spellings are one under that mapping, and they ask it here, so the
/// rule is written once rather than twice in two modules.
///
/// **The ASCII fast path.** Two ASCII names are compared without allocating:
/// the mapping folds ASCII letters and leaves every other byte alone, so
/// `eq_ignore_ascii_case` answers exactly what the mapping would. A name
/// outside ASCII — or one that is not UTF-8 at all — keeps the Unicode fold,
/// because that mapping is not a per-character rule: the final sigma depends
/// on the letters around it, so two names cannot be compared letter by letter
/// without answering a different question.
///
/// It is deliberately **not** the path key's comparison. The key folds ASCII
/// case only and answers which bucket of a run's map a path belongs to; this
/// one asks whether two spellings name the same place on disk.
#[cfg(windows)]
pub(crate) fn unicode_case_eq(a: &OsStr, b: &OsStr) -> bool {
    if let (Some(a), Some(b)) = (a.to_str(), b.to_str())
        && a.is_ascii()
        && b.is_ascii()
    {
        return a.eq_ignore_ascii_case(b);
    }
    unicode_lowercase(a) == unicode_lowercase(b)
}

/// The key a case-insensitive path comparison uses when two spellings may name
/// one file.
///
/// Insensitive mode pairs paths by folding case, so `Readme.TXT` on one side and
/// `readme.txt` on the other are one path. The fold is **ASCII-only**. Manifests
/// are ASCII in practice, and `str::to_lowercase` is a Unicode case mapping
/// rather than the filesystem's relation: it maps spellings the filesystem keeps
/// apart onto one key (the Kelvin sign `\u{212A}` and `k`, for example), which
/// would pair paths that are not the same file. The rule is "fold ASCII case,
/// compare every other byte exactly", so a non-ASCII case variant is
/// deliberately a distinct key.
///
/// This is a map key, not a containment check: the component equality
/// (`component_eq`) and [`same_path`] own the filesystem-identity decision and
/// keep their Unicode fallback for names outside ASCII. Callers use this where
/// the run pairs paths case-insensitively.
///
/// **Published, because the answer it decides is.** The pairing a run reports
/// is [`CasePairing::Lowercase`](crate::planner::CasePairing::Lowercase),
/// which pairs two sides by this key, so a caller that reads that pairing has
/// to be able to ask the same question the same way. Publishing the rule is
/// what keeps that caller from spelling the fold out again — and the fold is
/// the half of the rule that is easy to get wrong, because a Unicode fold
/// merges spellings the disk keeps apart.
pub fn path_key(rel: &str) -> String {
    // The shared crate owns the ASCII-fold mechanic; this call site owns the
    // rule: fold on every host, because case-insensitive pairing is a user
    // option here, not a property of the host filesystem. The repair binary's
    // `path_key` folds only on Windows; the two behaviours stay separate.
    girpr_fs_names::fold_ascii_case(rel, true)
}

/// A path's folded key, borrowed.
///
/// **The path key's rule, stated as a key that borrows the path it folds.**
/// Two paths are one key here exactly when their [`path_key`] forms are equal —
/// ASCII case folded, every other byte exact — so an index built from these
/// keys is the index the folded forms would build, and a lookup in it costs no
/// allocation.
///
/// [`path_key`] hands back a `String`, so an index of folded keys holds one
/// allocation per path it covers, and every lookup folds the path it looks up
/// into another one. This is that index's key, borrowed from the map it
/// indexes, so the index holds no copy of the tree's paths at all.
///
/// The fold is **not** the identity rule's: that one keeps a Unicode fallback
/// because it asks whether two spellings open the same place on disk, which
/// is a different question from the one a map key answers.
///
/// **Published with the rule it states.** It is the borrowed form of
/// [`path_key`], and an index a caller builds over a map the crate gave it
/// has to fold the way the crate folds; the borrowed form is the one that
/// costs no allocation per path.
pub struct FoldedKey<'a>(pub &'a str);

impl Hash for FoldedKey<'_> {
    /// The fold, hashed the way the comparison states it: two paths that are
    /// one key hash the same way.
    fn hash<H: Hasher>(&self, state: &mut H) {
        for &b in self.0.as_bytes() {
            state.write_u8(b.to_ascii_lowercase());
        }
    }
}

impl PartialEq for FoldedKey<'_> {
    /// The crate's ASCII-case comparison, which is the rule the borrowed key
    /// states: ASCII case folded, every other byte exact.
    fn eq(&self, other: &Self) -> bool {
        ascii_case_eq(self.0, other.0)
    }
}

impl Eq for FoldedKey<'_> {}

/// The folded-key index over one side's map: `path_key(key) → (key, value)`.
///
/// One shape for the sites that look a path up by its case-folded spelling, so
/// that they cannot drift on how such an index is built. The fold itself is
/// [`path_key`]'s; this owns only the *index* over it. The entries borrow the
/// map's own keys and values, so the index holds one folded spelling per key
/// and no second copy of the tree's entries.
///
/// **The index is unambiguous.** A mixed-case collision within one side is
/// already fatal before any caller reaches here — the mixed-case guard
/// (`check_mixed_case_keys`) rejects two spellings of one path — so a folded
/// key names exactly one path per side.
///
/// **It pairs nothing.** Which side a lookup resolves to, which direction a
/// rename moves, and which spelling a record is written under all stay with
/// the caller, as does the pairing rule itself: the planner pairs the two sides
/// as they are on disk
/// ([`CasePairing::Lowercase`](crate::planner::CasePairing::Lowercase)) and the
/// diff pairs them after the rename pass has re-keyed dst
/// ([`CasePairing::Exact`](crate::planner::CasePairing::Exact)).
///
/// **Published with the rule it indexes.** The answer this index serves is
/// public — the pairing is
/// [`CasePairing::Lowercase`](crate::planner::CasePairing::Lowercase) — so the
/// index over the path key is published with it, and a caller that builds one
/// over a map the crate gave it builds the crate's index rather than a shape
/// of its own.
pub fn folded_keys<V>(map: &HashMap<String, V>) -> HashMap<String, (&String, &V)> {
    map.iter().map(|(k, v)| (path_key(k), (k, v))).collect()
}

/// Group `keys` by case-folded spelling and abort when a group has more than one
/// spelling.
///
/// An exact duplicate is not a collision, so each group is sorted and
/// deduplicated before it is measured. The reported spellings are sorted for
/// the same reason: an error message must not depend on map iteration order.
/// `prefix` and `label` are joined to name the side in the message.
///
/// Shared by the disk walk and the record read: both compare paths by name, and
/// neither owns the rule.
pub(crate) fn check_mixed_case_keys<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    prefix: &str,
    label: &str,
) -> Result<()> {
    let mut groups: HashMap<String, Vec<&'a str>> = HashMap::new();
    for key in keys {
        groups.entry(path_key(key)).or_default().push(key);
    }
    for (_, mut group) in groups {
        group.sort_unstable();
        group.dedup();
        if group.len() > 1 {
            bail!(
                "mixed-case collision in {}{}: {}",
                prefix,
                label,
                group.join(" vs ")
            );
        }
    }
    Ok(())
}

/// Modification time as nanoseconds since the epoch (0 if unavailable or
/// before the epoch).
///
/// `filetime` owns the seconds/nanos split — including the platform-specific
/// Windows epoch — so this shim does no calendar arithmetic of its own. It only
/// folds the two documented accessors into the i64 the cache stores; no step
/// rounds away sub-second precision.
pub fn mtime_ns_of(meta: &std::fs::Metadata) -> i64 {
    let Ok(t) = meta.modified() else {
        return 0;
    };
    // A `FileTime` can hold pre-epoch seconds, but folding those into one i64
    // would produce a negative value. The cache has always used 0 for that
    // case, so keep the convention explicit instead of relying on a failed
    // `duration_since`.
    if t < std::time::UNIX_EPOCH {
        return 0;
    }
    let ft = filetime::FileTime::from_system_time(t);
    let total = (ft.unix_seconds() as u128) * 1_000_000_000 + ft.nanoseconds() as u128;
    total.min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache value is exact nanoseconds: a sub-second mtime must survive
    /// as the same count of nanoseconds, with no seconds-only intermediate.
    #[test]
    fn mtime_ns_of_keeps_exact_nanoseconds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exact.bin");
        std::fs::write(&path, b"x").unwrap();
        // 100 ns is the coarsest precision shared by NTFS and common POSIX
        // filesystems, so the stamp round-trips through the filesystem intact.
        let stamp = filetime::FileTime::from_unix_time(1_700_000_000, 123_456_700);
        #[allow(deprecated)] // filetime 0.2.30 defers these setters to std
        filetime::set_file_mtime(&path, stamp).unwrap();
        assert_eq!(
            mtime_ns_of(&std::fs::metadata(&path).unwrap()),
            1_700_000_000_123_456_700
        );
    }

    /// Pre-epoch mtimes keep the cache's long-standing 0 convention rather
    /// than becoming a negative i64.
    #[test]
    fn mtime_ns_of_maps_pre_epoch_to_zero() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("past.bin");
        std::fs::write(&path, b"x").unwrap();
        let stamp = filetime::FileTime::from_unix_time(-10, 0);
        #[allow(deprecated)] // filetime 0.2.30 defers these setters to std
        filetime::set_file_mtime(&path, stamp).unwrap();
        assert_eq!(mtime_ns_of(&std::fs::metadata(&path).unwrap()), 0);
    }

    /// Containment is component-wise, not a string prefix: `C:\AB` is not
    /// inside `C:\A`, and the root itself counts as within itself.
    #[test]
    fn containment_is_component_wise() {
        let a = Path::new("C:/A");
        assert!(path_within(Path::new("C:/A"), a), "a path contains itself");
        assert!(path_within(Path::new("C:/A/b"), a));
        assert!(!path_within(Path::new("C:/AB"), a));
        assert!(!path_within(Path::new("C:/"), a));
    }

    /// Separator rewriting belongs to the host whose separator is a backslash.
    /// On POSIX a backslash is part of a name — leading or in the middle — and
    /// must survive unchanged, or a relative path becomes absolute or names a
    /// file that is not on disk. Windows keeps the old rewrite.
    #[test]
    fn slash_rel_follows_the_host_separator() {
        let expected = if cfg!(windows) { "a/b.txt" } else { "a\\b.txt" };
        assert_eq!(slash_rel("a\\b.txt"), expected);
    }

    /// The POSIX answer, asked directly so it is pinned on every host: a
    /// backslash is a literal character and the relative path stays relative.
    /// A leading one must never become `/lead.txt`, which would replace the
    /// root it is joined onto. Windows still rewrites its own separator.
    #[test]
    fn slash_rel_leaves_posix_relative_paths_untouched() {
        for rel in ["a\\b.txt", "dir\\sub\\f.txt", "\\lead.txt"] {
            assert_eq!(slash_rel_for(rel, false), rel);
        }
        assert_eq!(slash_rel_for("dir\\sub\\f.txt", true), "dir/sub/f.txt");
    }

    /// A cache belongs to the root it sits in, so the reservation is per
    /// component rather than per path: a nested root's cache and a cache seen
    /// through a followed link are just as much tool state as the top-level one.
    #[test]
    fn cache_paths_are_reserved_at_any_depth() {
        for rel in [
            "girpr-cache",
            "girpr-cache/rows",
            "girpr-cache-backup-1",
            "girpr-cache-old-2/inner",
            "girpr-cache-partial-3",
            "sub/girpr-cache",
            "sub/girpr-cache/rows",
            "sub/deeper/girpr-cache-backup-3/x",
            "link/girpr-cache",
            "girpr-cache-notes/a.txt",
        ] {
            assert!(is_cache_rel(rel), "`{rel}` is tool state, not content");
        }
        for rel in ["a.txt", "sub/a.txt", "docs/my-girpr-cache.md", "girprcache"] {
            // The prefix is reserved as the start of a *component*, so a name
            // that merely contains it is ordinary content.
            assert!(!is_cache_rel(rel), "`{rel}` is ordinary content");
        }
    }

    /// The cache reservation is spelling-insensitive exactly where the
    /// filesystem is. On Windows a case-only rename of `girpr-cache` is the
    /// same live DB, so `GIRPR-CACHE` and its backups must be tool state too;
    /// on POSIX they are distinct entries and genuine content.
    #[test]
    fn the_cache_prefix_folds_case_on_windows_only() {
        for rel in [
            "girpr-cache",
            "girpr-cache/rows",
            "sub/girpr-cache-backup-1",
        ] {
            assert!(is_cache_rel(rel), "`{rel}` is tool state");
        }
        assert!(is_record_path(Path::new("x/girpr-cache")));
        assert!(is_record_path(Path::new("girpr-cache-backup-1")));

        for rel in [
            "GIRPR-CACHE",
            "sub/GIRPR-CACHE/rows",
            "GIRPR-CACHE-backup-1",
            "sub/Girpr-Cache-old-2",
        ] {
            #[cfg(windows)]
            assert!(
                is_cache_rel(rel),
                "`{rel}` names the same file as `girpr-cache`"
            );
            #[cfg(not(windows))]
            assert!(!is_cache_rel(rel), "`{rel}` is a distinct POSIX entry");
        }

        #[cfg(windows)]
        {
            assert!(is_record_path(Path::new("x/GIRPR-CACHE")));
            assert!(is_record_path(Path::new("GIRPR-CACHE-old-2")));
        }
        #[cfg(not(windows))]
        {
            assert!(!is_record_path(Path::new("x/GIRPR-CACHE")));
            assert!(!is_record_path(Path::new("GIRPR-CACHE-old-2")));
        }
    }

    /// **The cache's own name, and not the prefix the record rule reserves.**
    ///
    /// A record is anything named `girpr-cache*`, but only the exact name is a
    /// folder's cache — the one `girsync update --dir` rebuilds. A backup or a
    /// snapshot is a copy of one, and a repopulation would not rebuild it.
    ///
    /// The spelling rule is the record rule's: case is folded on Windows only,
    /// where the filesystem cannot tell the two spellings apart.
    #[test]
    fn the_cache_name_is_the_exact_name_not_the_prefix() {
        assert!(is_cache_name(Path::new("x/girpr-cache")));
        assert!(is_cache_name(Path::new("girpr-cache")));
        for path in [
            "x/girpr-cache-backup-1",
            "x/girpr-cache-old-2",
            "x/girpr-caches",
            "x/girpr-cachex",
            "x",
        ] {
            assert!(!is_cache_name(Path::new(path)), "`{path}` is not a cache");
        }

        #[cfg(windows)]
        assert!(
            is_cache_name(Path::new("x/GIRPR-CACHE")),
            "on Windows that is the same file"
        );
        #[cfg(not(windows))]
        assert!(
            !is_cache_name(Path::new("x/GIRPR-CACHE")),
            "on POSIX that is a distinct entry"
        );
    }

    /// On Windows the same tree can be spelled with different case, so an
    /// identity check that compared bytes would miss the collision.
    #[cfg(windows)]
    #[test]
    fn path_identity_ignores_case_on_windows() {
        assert!(same_path(
            Path::new("C:/Users/ACER"),
            Path::new("c:/users/acer")
        ));
        assert!(path_within(Path::new("C:/ROOT/Sub"), Path::new("c:/root")));
        assert!(!path_within(Path::new("C:/ROOTER"), Path::new("c:/root")));
    }

    /// The fast path covers ASCII spelling differences, and names outside
    /// ASCII still fold exactly as before: the filesystem treats both as one.
    #[cfg(windows)]
    #[test]
    fn component_equality_folds_ascii_and_non_ascii_case() {
        assert!(same_path(
            Path::new("C:/Users/ACER/Readme.TXT"),
            Path::new("c:/users/acer/readme.txt")
        ));
        assert!(path_within(
            Path::new("C:/ROOT/Sub/Ärger"),
            Path::new("c:/root/sub/ärger")
        ));
        assert!(!same_path(Path::new("C:/Ärger"), Path::new("c:/Arger")));
    }

    /// The path key folds ASCII case only. A Unicode fold would map the Kelvin
    /// sign onto `k` and pair two paths the filesystem keeps apart.
    #[test]
    fn path_key_folds_ascii_case_only() {
        assert_eq!(path_key("Game/Readme.TXT"), "game/readme.txt");
        assert_eq!(path_key("Ärger"), "Ärger");
        assert_ne!(path_key("\u{212A}"), path_key("k"));
    }

    /// **The oracle's own rule, over the bytes it decides.**
    ///
    /// The comparison is the crate's answer to "which bytes fold", so the
    /// answer is written out here as well as in the code that uses it: the 26
    /// ASCII letters fold to their lower-case counterparts, and every other
    /// byte is itself. The boundaries are the ones a reader would get wrong —
    /// `@` and `[` are ASCII but not letters, and a byte outside ASCII is not
    /// a letter either.
    #[test]
    fn the_ascii_case_comparison_folds_ascii_letters_and_nothing_else() {
        assert_eq!(ascii_lower(b'A'), b'a', "the first letter");
        assert_eq!(ascii_lower(b'Z'), b'z', "the last one");
        assert_eq!(ascii_lower(b'a'), b'a', "a lower-case letter is itself");
        assert_eq!(ascii_lower(b'@'), b'@', "ASCII, but not a letter");
        assert_eq!(ascii_lower(b'['), b'[', "ASCII, and just above the letters");
        assert_eq!(ascii_lower(b'0'), b'0', "a digit is not a letter");
        assert_eq!(
            ascii_lower(0xC4),
            0xC4,
            "and a byte outside ASCII is not a letter either"
        );

        // And the comparison that folds with it says the same thing about
        // two spellings: the letters fold alike, and nothing else does.
        assert!(ascii_case_eq("MD5", "md5"), "the letters fold alike");
        assert!(!ascii_case_eq("md5", "md-5"), "a non-letter does not");
        assert!(
            !ascii_case_eq("md5", "md5 "),
            "and a different length is not alike"
        );
        assert!(
            !ascii_case_eq("bla\u{212A}e3", "blake3"),
            "a byte outside ASCII is itself, so the Kelvin sign is not a `k`"
        );
    }

    /// The borrowed folded key is the path key's rule, so two paths are one key
    /// under one exactly when they are under the other: the ASCII pair is one,
    /// the non-ASCII pair is two, and the Kelvin sign stays apart from `k`.
    #[test]
    fn the_borrowed_folded_key_is_the_path_key_s_rule() {
        let spellings = [
            "a.txt",
            "A.txt",
            "Ärger.txt",
            "ärger.txt",
            "\u{212A}.txt",
            "k.txt",
        ];
        for a in spellings {
            for b in spellings {
                assert_eq!(
                    FoldedKey(a) == FoldedKey(b),
                    path_key(a) == path_key(b),
                    "`{a}` and `{b}` are one key under one rule or the other"
                );
            }
        }
    }

    /// A borrowed key is only usable as a key if it hashes the way it compares,
    /// so an index built from one finds the spelling it holds under any
    /// spelling of the same path.
    #[test]
    fn a_borrowed_folded_key_is_found_under_another_spelling() {
        let mut index: HashMap<FoldedKey<'_>, &str> = HashMap::new();
        index.insert(FoldedKey("Readme.TXT"), "Readme.TXT");
        assert_eq!(
            index.get(&FoldedKey("readme.txt")),
            Some(&"Readme.TXT"),
            "the case variant is the same key"
        );
        assert_eq!(
            index.get(&FoldedKey("other.txt")),
            None,
            "another path is another key"
        );
    }

    /// The folded-key index holds every key under its folded spelling, and each
    /// entry carries the key it was built from together with the value the map
    /// holds under it — the two things a caller needs to go on to pair, sort or
    /// report, and the reason the index is not a map of folded keys to values
    /// alone.
    #[test]
    fn the_folded_key_index_holds_every_key_under_its_folded_spelling() {
        let map: HashMap<String, u32> = [
            ("Readme.TXT".to_string(), 1),
            ("sub/Dir/File.bin".to_string(), 2),
            ("plain.txt".to_string(), 3),
        ]
        .into_iter()
        .collect();
        let index = folded_keys(&map);
        assert_eq!(index.len(), map.len(), "one entry per key");
        for (key, value) in &map {
            let (folded_key, entry) = index
                .get_key_value(&path_key(key))
                .expect("every key is indexed under its folded spelling");
            assert_eq!(
                folded_key,
                &path_key(key),
                "the index key is the folded one"
            );
            let (entry_key, entry_value) = *entry;
            assert_eq!(entry_key, key, "the entry keeps the key it came from");
            assert_eq!(
                entry_value, value,
                "the entry keeps the value the map holds"
            );
        }
    }

    /// Two spellings the path key folds together are one entry, and two it
    /// keeps apart are two. A map still holding two spellings of one path is a
    /// map the mixed-case guard has already refused, so which spelling survives
    /// the fold is not this index's business — it pairs nothing — and the count
    /// is what is pinned here.
    #[test]
    fn the_folded_key_index_merges_exactly_what_the_path_key_merges() {
        let map: HashMap<String, u32> = [
            ("a.txt".to_string(), 1),
            ("A.txt".to_string(), 2),
            ("Ärger.txt".to_string(), 3),
            ("ärger.txt".to_string(), 4),
        ]
        .into_iter()
        .collect();
        let index = folded_keys(&map);
        assert_eq!(
            index.len(),
            3,
            "the ASCII pair is one path and the non-ASCII pair is two"
        );
        assert!(
            index.contains_key(&path_key("a.txt")),
            "the merged pair is indexed under its folded spelling"
        );
        assert!(
            index.contains_key(&path_key("Ärger.txt")),
            "a non-ASCII spelling is indexed under its own key"
        );
        assert!(
            index.contains_key(&path_key("ärger.txt")),
            "and so is the other one, because the key folds ASCII case only"
        );
        assert_ne!(
            path_key("Ärger.txt"),
            path_key("ärger.txt"),
            "the key keeps the two spellings apart"
        );
    }

    /// The mixed-case guard groups by the same key, so its verdict for a
    /// non-ASCII pair is "two distinct paths" rather than a collision. The ASCII
    /// spelling difference still collides, and an exact repeat still does not.
    #[test]
    fn mixed_case_check_groups_by_the_ascii_only_key() {
        let err = check_mixed_case_keys(["Readme.TXT", "readme.txt"], "", "src").unwrap_err();
        assert_eq!(
            err.to_string(),
            "mixed-case collision in src: Readme.TXT vs readme.txt"
        );
        check_mixed_case_keys(["same.txt", "same.txt"], "", "src")
            .expect("an exact repeat is one spelling, not a collision");
        check_mixed_case_keys(["Ärger.txt", "ärger.txt"], "", "src")
            .expect("a non-ASCII case variant is a distinct path");
        check_mixed_case_keys(["\u{212A}", "k"], "", "src").expect("the Kelvin sign is not `k`");
    }

    /// The path key and the identity rule answer different questions, and for a
    /// non-ASCII name they deliberately disagree: the key keeps `Ärger` and
    /// `ärger` apart, because a Unicode fold would merge spellings the
    /// filesystem keeps apart, while the filesystem on Windows treats the two
    /// as one file and so does `same_path`. Unifying the two rules would
    /// change one of them without anything saying so, so the boundary is
    /// pinned here.
    #[test]
    fn the_path_key_and_the_identity_rule_disagree_deliberately() {
        assert_ne!(
            path_key("Ärger"),
            path_key("ärger"),
            "the key folds ASCII case only, so a non-ASCII variant is a distinct key"
        );
        #[cfg(windows)]
        assert!(
            same_path(Path::new("C:/Ärger"), Path::new("C:/ärger")),
            "Windows opens either spelling as the same file"
        );
        #[cfg(not(windows))]
        assert!(
            !same_path(Path::new("/Ärger"), Path::new("/ärger")),
            "POSIX keeps the two spellings apart, so the rules agree there"
        );
    }

    /// A name that is not valid UTF-8 has no `to_str` answer, so the comparison
    /// must fall through to the lossy Unicode fold. The unpaired surrogate is
    /// valid UTF-16 storage but not valid UTF-8, which is the only way an
    /// `OsStr` component reaches that branch on Windows.
    #[cfg(windows)]
    #[test]
    fn a_non_utf8_component_still_compares_through_the_lossy_fold() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;

        // `C:\a<0xD800>b`, the surrogate standing in for a byte sequence the
        // platform allows but UTF-8 cannot represent.
        let name = |first: u16, last: u16| {
            let mut units: Vec<u16> = r"C:\".encode_utf16().collect();
            units.extend([first, 0xD800, last]);
            PathBuf::from(OsString::from_wide(&units))
        };
        let lower = name(b'a' as u16, b'b' as u16);
        let upper = name(b'A' as u16, b'B' as u16);
        let other = name(b'a' as u16, b'c' as u16);
        assert!(
            lower.as_os_str().to_str().is_none(),
            "the fixture must reach the fallback, not `to_str`"
        );
        assert!(
            same_path(&lower, &upper),
            "the lossy fold still ignores case around the undecodable component"
        );
        assert!(
            !same_path(&lower, &other),
            "and it still tells two different tails apart"
        );
    }
}
