//! Flags resolved into a plan.
//!
//! A [`CaptureArgs`] is what the user typed. A [`CapturePlan`] is what the run
//! will do: the version pair resolved against a live API answer, the selection
//! pool widened by `--audio`, the budget made concrete, the paths normalised.
//! The split is what lets a test build a plan without a command line, and what
//! keeps one rule from being applied twice under two spellings.
//!
//! What needs an API answer — the version pair, the branch's `package_id` and
//! `password` — arrives with [`crate::hyp`], because a plan whose `old_tag`
//! came from `diff_tags[0]` cannot be built without one.

use std::path::{Path, PathBuf};

use crate::cli::{
    Biz, CaptureArgs, ManifestDetail, MaterializeArgs, Result, UsageError, VerifyArgs,
};

/// The audio language selected when `--audio` is not passed.
///
/// Both a launchable-install default and a deliberate widening of the selection
/// pool: every selected language adds a manifest to the corpus the matrix is
/// scored against, which is the backstop for a class with no `game`-field
/// candidate in a given version pair.
pub const DEFAULT_AUDIO: &str = "en-us";

/// The `--audio` spelling that means "game only, no voice packs".
pub const AUDIO_NONE: &str = "none";

/// How `--new-tag` chose a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewTagSelector<'a> {
    /// `--new-tag latest`, or the flag left off: whatever the branch serves.
    BranchHead,
    /// `--new-tag <tag>`: exactly what was written.
    Explicit(&'a str),
}

/// How `--old-tag` chose a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OldTagSelector<'a> {
    /// `--old-tag`, or the flag left off: `diff_tags[0]`.
    FirstDiffTag,
    /// `--old-tag <tag>`.
    Explicit(&'a str),
}

/// The version pair a capture is about, and where each half came from.
///
/// The `source` strings are the artifact's own vocabulary: a bundle says
/// `diff_tags[0]` because that is a claim a reader can check against the captured
/// `getGameBranches`, and it says so whether the default was taken or a flag was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionPair {
    /// The version the fixture is materialized at.
    pub old: VersionRef,
    /// The version it is patched to.
    pub new: VersionRef,
    /// The branch's patch-support list, verbatim.
    pub diff_tags: Vec<String>,
}

/// One half of the pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRef {
    /// The tag.
    pub tag: String,
    /// The build id, once `getBuild` has been asked. Empty until then.
    pub build_id: String,
    /// How the tag was chosen: `main.tag`, `diff_tags[0]`, `--new-tag`,
    /// `--old-tag`.
    pub source: String,
}

impl VersionRef {
    /// Whether the build id has been filled in.
    pub fn resolved(&self) -> bool {
        !self.build_id.is_empty()
    }
}

impl VersionPair {
    /// Choose the two tags, before anything has been fetched.
    ///
    /// Pure: it needs only the branch parameters and the flags, so the pair can
    /// be tested without a socket — which is also how a bad flag is rejected
    /// before the bundle's directory is named after it.
    pub fn select(
        params: &crate::biz::BranchParams,
        new: NewTagSelector<'_>,
        old: OldTagSelector<'_>,
    ) -> std::result::Result<Self, String> {
        let (new_tag, new_source) = match new {
            // The sentinel is resolved *here*, against a live answer, which is
            // the whole point of making it a flag value rather than a lookup.
            NewTagSelector::BranchHead => (params.tag.clone(), "main.tag".to_string()),
            NewTagSelector::Explicit(t) => (t.to_string(), "--new-tag".to_string()),
        };
        let (old_tag, old_source) = match old {
            OldTagSelector::FirstDiffTag => (
                params
                    .diff_tags
                    .first()
                    .cloned()
                    .ok_or_else(|| NO_DIFF_TAGS.to_string())?,
                "diff_tags[0]".to_string(),
            ),
            OldTagSelector::Explicit(t) => (t.to_string(), "--old-tag".to_string()),
        };
        // **The resolved tags are validated here, not only at the flags.** When
        // `new`/`old` are `BranchHead`/`FirstDiffTag`, these strings come from
        // the API's `main.tag` and `diff_tags[0]`, and `pair_dir` joins them raw
        // into `<root>/<biz>/<old>__<new>/`. The flag path already checks its
        // input; without this, a hostile or compromised API (or a local proxy)
        // could reach the same path sink with `..\..\escape` or `C:evil` even
        // though the flags were clean.
        for (tag, source) in [
            (&new_tag, new_source.as_str()),
            (&old_tag, old_source.as_str()),
        ] {
            // A tag the API named is escaped on disk rather than refused, so a
            // live device name is kept as evidence. A tag from a flag has no
            // bundle yet and stays refused at argument resolution.
            let api = matches!(source, "main.tag" | "diff_tags[0]");
            let problem = if api {
                tag_shape_problem_allowing_device(tag)
            } else {
                tag_shape_problem(tag)
            };
            if let Some(problem) = problem {
                return Err(format!("{source} {problem}"));
            }
        }
        Ok(VersionPair {
            old: VersionRef {
                tag: old_tag,
                build_id: String::new(),
                source: old_source,
            },
            new: VersionRef {
                tag: new_tag,
                build_id: String::new(),
                source: new_source,
            },
            diff_tags: params.diff_tags.clone(),
        })
    }

    /// Record both build ids, after the two `getBuild`s are in.
    ///
    /// Takes them positionally in `new, old` order because that is the order the
    /// fetches happen in; swapping them would silently attach a build id to the
    /// wrong tag, and every chunk URL in the bundle is namespaced by it.
    pub fn attach_build_ids(&mut self, new_build_id: &str, old_build_id: &str) {
        self.new.build_id = new_build_id.to_string();
        self.old.build_id = old_build_id.to_string();
    }

    /// The tags, new first — the order a report reads in.
    pub fn tags(&self) -> [&str; 2] {
        [&self.new.tag, &self.old.tag]
    }
}

/// Why there is no default old version, stated as a remedy rather than a fact.
const NO_DIFF_TAGS: &str = "the branch lists no diff_tags, so there is no default old version; pass --old-tag with a \
     tag the API serves (see `gircap capture --probe-tags`, or spec section 3.4)";

impl CapturePlan {
    /// The `--new-tag` selector this plan asks for.
    ///
    /// The sentinel match is case-insensitive: `latest`, `LATEST` and `Latest`
    /// are the same request, and a flag that only honoured the exact spelling
    /// would turn a case slip into a capture of a tag called `LATEST` — which
    /// the API would answer `-202` for, and which the run would report as an
    /// unservable tag rather than as the typo it was.
    pub fn new_selector(&self) -> NewTagSelector<'_> {
        match self.new_tag.as_deref() {
            Some(t) if t.eq_ignore_ascii_case(crate::cli::NEW_TAG_LATEST) => {
                NewTagSelector::BranchHead
            }
            None => NewTagSelector::BranchHead,
            Some(t) => NewTagSelector::Explicit(t),
        }
    }

    /// The `--old-tag` selector this plan asks for.
    pub fn old_selector(&self) -> OldTagSelector<'_> {
        match self.old_tag.as_deref() {
            None => OldTagSelector::FirstDiffTag,
            Some(t) => OldTagSelector::Explicit(t),
        }
    }
}

/// `gircap capture` resolved and checked.
#[derive(Debug, Clone)]
pub struct CapturePlan {
    /// Which region to capture.
    pub biz: Biz,
    /// Artifact root; the bundle lands in `<out>/<biz>/<old>__<new>/<id>/`.
    pub out: PathBuf,
    /// `--new-tag`, with the `latest` sentinel *not yet* resolved.
    pub new_tag: Option<String>,
    /// `--old-tag`, with the `diff_tags[0]` default *not yet* resolved.
    pub old_tag: Option<String>,
    /// Files admitted per matrix class. **At least 1** — the floor is applied in
    /// [`capture_plan`], so a consumer may read this without defending against
    /// zero.
    pub per_class: u32,
    /// Summed-cost ceiling over admitted files.
    pub max_download_bytes: Option<u64>,
    /// Classes whose absence must fail the run.
    ///
    /// **Parsed, not raw, and not defaulted.** Both matter and they are separate
    /// decisions. Parsing happens here so that `--require-class identicalo` is a
    /// usage error before a socket opens, naming the flag and the bad value.
    /// Defaulting does *not* happen here, because the set of required classes is
    /// the matrix's own ordering and duplicating it here would be a second source
    /// of truth for one fact. An empty list therefore means "nothing is required",
    /// and `select::require` is where an unmet class becomes exit `4`.
    pub require_class: Vec<crate::select::Class>,
    /// Explicit paths, bypassing matrix selection.
    pub files: Vec<String>,
    /// Selected audio: either codes or the `none` sentinel, never both.
    pub audio: AudioSelection,
    /// How much of each decoded manifest to keep.
    pub manifest_detail: ManifestDetail,
    /// Documents above this size are gzipped.
    pub gzip_threshold: u64,
    /// Extra tags to probe.
    pub probe_tags: Vec<String>,
    /// Skip every chunk download.
    pub no_chunk_blobs: bool,
    /// Concurrent chunk downloads.
    pub jobs: usize,
    /// Attempts per request, including the first.
    pub retries: u32,
    /// Per-request timeout.
    pub timeout: std::time::Duration,
    /// Ceiling on one response body.
    ///
    /// Not the same as `--max-download-bytes`, which is a *budget* over the files
    /// a run admits to fetch; this limits any single response, whether or not the
    /// file was admitted.
    pub max_response_bytes: u64,
    /// Ceiling on one decompressed body, in bytes. Applied to every zstd decode
    /// and recorded in `MANIFEST.json` so a bundle can be decoded the same way.
    pub max_decompressed_bytes: u64,
    /// HoYoPlay base override; `None` = the production base for [`Self::biz`].
    pub hyp_base: Option<String>,
    /// Sophon base override; `None` = the production base for [`Self::biz`].
    pub sophon_base: Option<String>,
    /// The argv as given, recorded in the artifact so a bundle says how it was made.
    ///
    /// Built by [`crate::cli::recorded_argv`] from `args_os`, so a non-Unicode
    /// path is recorded lossily rather than panicking after a successful parse.
    /// The exact bytes are kept beside the lossy form; see
    /// [`crate::cli::RecordedArgv`].
    pub argv: crate::cli::RecordedArgv,
}

/// Validate an API base override.
///
/// This is a **testing seam**, and the two things that makes it safe to have are
/// both checked here: the URL must be absolute, and `https` unless it is loopback.
/// Without the second check a mistyped override could silently downgrade a
/// production capture to plaintext, which is the one failure mode a seam like
/// this has to earn the right to have.
fn validate_base(raw: Option<&str>, flag: &str) -> Result<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let url = reqwest::Url::parse(raw)
        .map_err(|e| UsageError::new(format!("{flag} {raw:?} is not a URL: {e}")))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err(UsageError::new(format!(
            "{flag} {raw:?} must be http or https, not {}",
            url.scheme()
        )));
    }
    let loopback = matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("::1")
    );
    if url.scheme() != "https" && !loopback {
        return Err(UsageError::new(format!(
            "{flag} {raw:?} is plaintext to a non-loopback host; only https is allowed \
             outside 127.0.0.1"
        )));
    }
    // Trailing slashes are stripped once here rather than trimmed at every join
    // site, so there is exactly one place that decides how a base meets a path.
    Ok(Some(url.as_str().trim_end_matches('/').to_string()))
}

/// `--audio` resolved: the codes selected, or the `none` sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioSelection {
    /// These languages are selected; each adds a manifest to the pool.
    Codes(Vec<String>),
    /// Game only. The scan file is written empty.
    None,
}

impl AudioSelection {
    /// The codes as the selection pool names them: empty under [`AudioSelection::None`].
    pub fn codes(&self) -> &[String] {
        match self {
            AudioSelection::Codes(v) => v,
            AudioSelection::None => &[],
        }
    }
}

impl std::fmt::Display for AudioSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioSelection::None => f.write_str(AUDIO_NONE),
            AudioSelection::Codes(v) => f.write_str(&v.join(",")),
        }
    }
}

/// Validate a flag-level request into a [`CapturePlan`].
///
/// This cannot resolve a version pair — that needs an API answer — but everything
/// that is wrong in the *request* is caught here, so a bad flag is rejected
/// before a socket is opened rather than after.
pub fn capture_plan(args: &CaptureArgs, argv: crate::cli::RecordedArgv) -> Result<CapturePlan> {
    let transport = validate_transport(
        args.jobs,
        args.retries,
        args.timeout,
        args.max_response_bytes,
    )?;
    let audio = validate_audio(&args.audio)?;
    validate_paths(&args.files, "--files")?;
    validate_probe_tags(&args.probe_tags)?;

    if let Some(n) = args.new_tag.as_deref() {
        validate_new_tag(n)?;
    }
    if let Some(t) = args.old_tag.as_deref() {
        validate_tag(t, "--old-tag")?;
    }
    if args.max_decompressed_bytes == 0 {
        return Err(UsageError::new(
            "--max-decompressed-bytes must be at least 1; a zero ceiling makes every \
             zstd decode fail",
        ));
    }
    if let (Some(new), Some(old)) = (&args.new_tag, &args.old_tag)
        && new.eq_ignore_ascii_case(old)
    {
        // Same tag twice would make every file `identical` by construction and
        // would exercise none of the branches the matrix exists to cover.
        return Err(UsageError::new(format!(
            "--new-tag and --old-tag are both {old}; the pair must differ"
        )));
    }

    Ok(CapturePlan {
        biz: args.biz,
        out: args.out.clone(),
        new_tag: args.new_tag.clone(),
        old_tag: args.old_tag.clone(),
        // **The floor is applied here, once**, so it is a property of the plan
        // rather than a convention each consumer has to remember.
        //
        // **Still a floor, not a usage error**: asking for zero captures of a
        // class is not a request for an empty bundle, it is a request to capture
        // nothing while still charging admission for it. `select::decide` keeps its
        // own floor as defence, because it is a pure function called directly by
        // tests with whatever they like.
        per_class: args.per_class.max(1),
        max_download_bytes: args.max_download_bytes,
        // Parsed here, even though the empty case is not defaulted: a typo must be
        // a usage error before any network work, not a finding discovered after
        // both manifests per field, the diff and the whole selection have run.
        require_class: parse_classes(&args.require_class)?,
        files: args.files.clone(),
        audio,
        manifest_detail: args.manifest_detail,
        gzip_threshold: args.gzip_threshold,
        probe_tags: args.probe_tags.clone(),
        no_chunk_blobs: args.no_chunk_blobs,
        jobs: args.jobs,
        retries: transport.retries,
        timeout: transport.timeout,
        max_response_bytes: transport.max_response_bytes,
        max_decompressed_bytes: args.max_decompressed_bytes,
        hyp_base: validate_base(args.hyp_base.as_deref(), "--hyp-base")?,
        sophon_base: validate_base(args.sophon_base.as_deref(), "--sophon-base")?,
        argv,
    })
}

/// Parse every `--require-class` value, naming the flag and the bad one.
fn parse_classes(raw: &[String]) -> Result<Vec<crate::select::Class>> {
    raw.iter()
        .map(|c| {
            crate::select::Class::parse(c)
                .map_err(|e| UsageError::new(format!("--require-class {c:?}: {e}")))
        })
        .collect()
}

/// Validate the transport flags together, naming the one at fault.
///
/// `--jobs` is range-checked here but is **not** part of the returned value:
/// `http::Transport` is per-fetcher patience, and the concurrency a capture may
/// use is the caller's business — the chunk fetcher reads `plan.jobs` where it
/// builds its own queue.
fn validate_transport(
    jobs: usize,
    retries: u32,
    timeout_secs: u64,
    max_response_bytes: u64,
) -> Result<crate::http::Transport> {
    if jobs == 0 {
        return Err(UsageError::new("--jobs must be at least 1"));
    }
    if jobs > 64 {
        return Err(UsageError::new(format!(
            "--jobs {jobs} exceeds 64; the CDNs rate-limit well below that and the \
             concurrency would only convert a throttle into failures"
        )));
    }
    if retries == 0 {
        return Err(UsageError::new(
            "--retries must be at least 1 (it counts the first attempt)",
        ));
    }
    if retries > 10 {
        return Err(UsageError::new(format!(
            "--retries {retries} exceeds 10; past that a failure is a real failure"
        )));
    }
    if timeout_secs == 0 {
        return Err(UsageError::new("--timeout must be at least 1 second"));
    }
    if max_response_bytes == 0 {
        // The other three are checked here for the same reason, and a zero ceiling
        // is the same kind of nonsense: it would refuse the very first byte of every
        // response, which is a tool that cannot fetch anything rather than one with
        // a limit.
        return Err(UsageError::new("--max-response-bytes must be at least 1"));
    }
    Ok(crate::http::Transport {
        retries,
        timeout: std::time::Duration::from_secs(timeout_secs),
        max_response_bytes,
    })
}

/// Validate `--audio`, which is repeatable *and* has a default, so "was it
/// passed?" is answered by the value rather than by the flag's presence.
fn validate_audio(raw: &[String]) -> Result<AudioSelection> {
    if raw.is_empty() {
        return Err(UsageError::new("--audio was given no value"));
    }
    let normalised: Vec<String> = raw.iter().map(|a| normalize_audio(a)).collect();

    if normalised.iter().any(|a| a == AUDIO_NONE) {
        if normalised.len() > 1 {
            return Err(UsageError::new(format!(
                "--audio none cannot be combined with language codes ({})",
                normalised.join(", ")
            )));
        }
        return Ok(AudioSelection::None);
    }

    let mut seen: Vec<&String> = Vec::new();
    for code in &normalised {
        // The code is the `matching_field`, and the dump interpolates it raw into
        // `manifests/<tag>/<field>.manifest…`. It is a file-name component, not
        // free text: `..`, a separator or a Windows alias would name a file
        // outside the manifest directory. The same rule guards the manifest
        // entries, so an audio code is held to it too.
        if let Some(problem) = crate::paths::field_component_problem(code) {
            return Err(UsageError::new(format!("--audio {problem}")));
        }
        if seen.contains(&code) {
            return Err(UsageError::new(format!(
                "--audio {code} was given twice; a repeated language adds no manifest \
                 to the selection pool"
            )));
        }
        seen.push(code);
    }
    Ok(AudioSelection::Codes(normalised))
}

/// Lowercase, and accept `_` for `-`, so `EN_US` and `en-us` are the same language.
fn normalize_audio(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

/// Reject a path that cannot be a manifest-relative path, naming the flag.
///
/// Manifest paths are `/`-separated and rooted at the game directory, so an
/// absolute path, a drive letter, a parent hop or a backslash cannot name a file
/// in a manifest and would otherwise be silently admitted and then missed. The
/// shape rule itself lives in [`crate::paths`], because the same rule has to
/// cover the manifest, the bundle index and the launcher's scan directory — not
/// only this flag.
///
/// **Repeats are a usage error**, for the same reason `--audio` rejects them: a
/// second identical `--files` entry finds the same candidate and is admitted
/// with the same capture id, which produces two admissions, two patchmap writes
/// (the second overwriting the first) and a `files_captured` count of two for
/// one file. Naming a path twice is never a different request.
fn validate_paths(paths: &[String], flag: &str) -> Result<()> {
    let mut seen: Vec<&str> = Vec::with_capacity(paths.len());
    for p in paths {
        if let Some(problem) = crate::paths::relative_path_problem(p) {
            return Err(UsageError::new(format!("{flag} {problem}")));
        }
        if seen.contains(&p.as_str()) {
            return Err(UsageError::new(format!(
                "{flag} {p:?} was given twice; a repeated path adds no capture"
            )));
        }
        seen.push(p.as_str());
    }
    Ok(())
}

/// Validate `--probe-tags`.
fn validate_probe_tags(tags: &[String]) -> Result<()> {
    for t in tags {
        validate_tag(t, "--probe-tags")?;
    }
    Ok(())
}

/// A version tag, as the API spells it: `7.1.0`.
///
/// `latest` is a sentinel the flag accepts and this rejects, because it is a
/// *flag* value and not a tag: resolving it is a lookup against a live answer,
/// and a probe has no such thing to resolve against. [`validate_new_tag`] is
/// the one exception, and it exists only because `--new-tag` is the flag that
/// defines the sentinel.
pub fn validate_tag(tag: &str, flag: &str) -> Result<()> {
    if crate::cli::NEW_TAG_LATEST.eq_ignore_ascii_case(tag) {
        return Err(UsageError::new(format!(
            "{flag} {tag:?} is the --new-tag sentinel; it is not a version tag and \
             cannot be probed"
        )));
    }
    validate_tag_shape(tag, flag)
}

/// `--new-tag`, which additionally accepts the `latest` sentinel.
pub fn validate_new_tag(tag: &str) -> Result<()> {
    if crate::cli::NEW_TAG_LATEST.eq_ignore_ascii_case(tag) {
        return Ok(());
    }
    validate_tag_shape(tag, "--new-tag")
}

/// The shape every tag must have, sentinel or not.
///
/// A tag is interpolated raw into `<root>/<biz>/<old>__<new>/` (`pair_dir`) and
/// into the capture directory, so it is a path component on every host. The
/// checks here are the same rule `--files` applies to a manifest path
/// ([`validate_paths`]): no separator of either flavour, no drive prefix, no
/// parent component, no control character. `/` and space were the only ones
/// checked before, so `C:`, `..\..` and `a\b` reached the path sink intact.
///
/// **The Windows rules are here too.** `* ? " < > |` cannot appear in a Windows
/// directory name, and a trailing `.` is stripped by the filesystem, so `7.1.0.`
/// and `7.1.0` would otherwise be two strings that name one directory. Both are
/// rejected with the same message class as the traversal checks: a tag is a
/// directory component, and the directory is named before the network work is
/// useful, so a late `create_dir_all` failure would waste the whole run.
///
/// A reserved DOS device name is refused by the same rule the path validator
/// uses ([`crate::paths`]): the tag becomes `manifests/<tag>`, so `CON` names a
/// device whatever the extension on Windows, and a bundle built where it is
/// accepted would carry a directory a Windows reader cannot open.
///
/// Split on both separators even though both are rejected above: the rules are
/// then stated independently, which is what stops a future relaxation of one
/// from silently reopening the other.
fn validate_tag_shape(tag: &str, flag: &str) -> Result<()> {
    match tag_shape_problem(tag) {
        Some(problem) => Err(UsageError::new(format!("{flag} {problem}"))),
        None => Ok(()),
    }
}

/// Why `tag` cannot be a directory component, or `None` when it can.
///
/// The message names the tag and the rule, so both the flag path (which prefixes
/// the flag) and the API path (which prefixes `main.tag` or `diff_tags[0]`) say
/// which string was refused and why.
///
/// `manifest::dump::write` calls it too: it is the last place a tag becomes a
/// path, and a caller that builds a `CapturedManifest` by hand must meet the
/// same rule as the capture path.
pub(crate) fn tag_shape_problem(tag: &str) -> Option<String> {
    tag_shape_rule(tag).map(|(_, problem)| problem)
}

/// [`tag_shape_problem`] with the rule's stable identifier.
pub(crate) fn tag_shape_rule(tag: &str) -> Option<(&'static str, String)> {
    tag_shape_rule_inner(tag, false)
}

/// [`tag_shape_problem`] for a tag that came from the API, where a device name
/// is escaped on disk instead of refused.
///
/// **Argument resolution still refuses.** A `--new-tag CON` names a directory
/// before any request, so there is no observation to keep and refusing is the
/// honest answer. But `main.tag` and `diff_tags[0]` are the API's own words: a
/// live `CON` is a fact about the branch, so it is preserved in the documents
/// and written under `manifests/%CON` (see [`crate::paths::escape_component`]).
pub(crate) fn tag_shape_problem_allowing_device(tag: &str) -> Option<String> {
    tag_shape_rule_inner(tag, true).map(|(_, problem)| problem)
}

fn tag_shape_rule_inner(tag: &str, allow_device: bool) -> Option<(&'static str, String)> {
    if tag.is_empty() {
        return Some(("tag.empty", "was given an empty tag".to_string()));
    }
    if tag.len() > 32 {
        return Some((
            "tag.too-long",
            format!("{tag:?} is longer than 32 characters; that is not a version tag"),
        ));
    }
    if tag.chars().any(char::is_control) {
        return Some((
            "tag.control",
            format!("{tag:?} contains a control character; a tag is a bare version string"),
        ));
    }
    if tag.contains('/') || tag.contains('\\') || tag.contains(' ') {
        return Some((
            "tag.separator",
            format!("{tag:?} contains a separator or a space; a tag is a bare version string"),
        ));
    }
    if tag.contains(':') {
        return Some((
            "tag.drive",
            format!(
                "{tag:?} looks like a drive-qualified or namespaced path; a tag is a \
                 bare version string"
            ),
        ));
    }
    if tag.contains(['*', '?', '"', '<', '>', '|']) {
        return Some((
            "tag.windows-character",
            format!(
                "{tag:?} contains a character Windows cannot use in a directory name; \
                 a tag is a bare version string"
            ),
        ));
    }
    if tag.starts_with('.') || tag.ends_with('.') {
        return Some((
            "tag.edge-dot",
            format!(
                "{tag:?} starts or ends with `.`; Windows strips a trailing dot from a \
                 directory name, so two tags could name one directory"
            ),
        ));
    }
    if !allow_device && crate::paths::is_windows_device_name(tag) {
        return Some((
            "tag.device",
            format!(
                "{tag:?} names a Windows device; a tag is a bare version string and \
                 becomes a directory name"
            ),
        ));
    }
    if tag.split(['/', '\\']).any(|c| c == "..") {
        return Some((
            "tag.parent",
            format!("{tag:?} escapes the artifact directory"),
        ));
    }
    None
}

/// `gircap verify` resolved and checked.
#[derive(Debug, Clone)]
pub struct VerifyPlan {
    /// The bundle to re-check.
    pub artifact: PathBuf,
}

/// `gircap materialize` resolved and checked.
#[derive(Debug, Clone)]
pub struct MaterializePlan {
    /// The bundle to rebuild from.
    pub artifact: PathBuf,
    /// Where to write, if not beside the bundle.
    pub out: Option<PathBuf>,
}

/// Resolve `gircap verify`'s arguments.
pub fn verify_plan(args: &VerifyArgs) -> Result<VerifyPlan> {
    Ok(VerifyPlan {
        artifact: args.artifact.clone(),
    })
}

/// Resolve `gircap materialize`'s arguments.
pub fn materialize_plan(args: &MaterializeArgs) -> Result<MaterializePlan> {
    let out = match &args.out {
        None => None,
        Some(dir) if *dir == args.artifact => {
            return Err(UsageError::new(
                "--out must differ from --artifact; a rebuild is a check against the bundle, \
                 so writing it into the bundle it is proving defeats it",
            ));
        }
        Some(dir) => Some(dir.clone()),
    };
    Ok(MaterializePlan {
        artifact: args.artifact.clone(),
        out,
    })
}

/// `artifacts/<biz>/<old>__<new>/` — the directory a bundle's id sits in.
///
/// The joined pair is escaped as one component: `CON.txt__7.1.0` would otherwise
/// have the stem `CON` and open the console on Windows, so it is written as
/// `%CON.txt__7.1.0`. Because [`crate::paths::escape_component`] doubles every
/// `%`, a tag containing one also changes the directory name (`a%b` always
/// becomes `a%%b`). The tags themselves are kept in `MANIFEST.json → versions`;
/// `diagnostics/path_map.json` maps renames *inside* the bundle and cannot name
/// this directory, which sits above it.
pub fn pair_dir(root: &Path, biz: Biz, old_tag: &str, new_tag: &str) -> PathBuf {
    let pair = format!("{old_tag}__{new_tag}");
    root.join(biz.as_str())
        .join(crate::paths::escape_component(&pair).into_owned())
}

/// The capture id: a UTC timestamp, `YYYYMMDDTHHMMSS.ffffffZ`.
///
/// Time-ordered so a directory listing of a root sorts as a history, and named
/// for the moment the capture started rather than finished, so a long run's id
/// still says when the run began.
pub fn capture_id(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y%m%dT%H%M%S%.6fZ").to_string()
}

/// The full bundle directory: `<out>/<biz>/<old>__<new>/<id>/`.
pub fn artifact_dir(
    out: &Path,
    biz: Biz,
    old_tag: &str,
    new_tag: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> PathBuf {
    pair_dir(out, biz, old_tag, new_tag).join(capture_id(now))
}

/// Render a filesystem path with forward slashes, for a document field.
///
/// Bundle documents carry `/`-separated paths regardless of host, so an artifact
/// captured on Windows and one captured on Linux compare byte-for-byte. The
/// component walk is spelled out rather than done with a `replace('\\', "/")`
/// because a Windows `Path`'s root component renders as `\`, and the naive
/// substitution turns `/elsewhere/x` into `\/elsewhere/x`.
pub fn rel_path_string(root: &Path, path: &Path) -> String {
    use std::path::Component as C;
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut out = String::new();
    for c in rel.components() {
        match c {
            C::Prefix(p) => {
                out.push_str(&p.as_os_str().to_string_lossy());
                out.push('/');
            }
            C::RootDir => out.push('/'),
            C::CurDir => {}
            C::ParentDir => {
                out.push_str("../");
            }
            C::Normal(n) => {
                if !out.is_empty() && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(&n.to_string_lossy());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
