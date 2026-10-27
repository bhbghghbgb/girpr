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
            if let Some(problem) = tag_shape_problem(tag) {
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
    /// HoYoPlay base override; `None` = the production base for [`Self::biz`].
    pub hyp_base: Option<String>,
    /// Sophon base override; `None` = the production base for [`Self::biz`].
    pub sophon_base: Option<String>,
    /// The argv as given, recorded in the artifact so a bundle says how it was made.
    ///
    /// Built by [`crate::cli::recorded_argv`] from `args_os`, so a non-Unicode
    /// path is recorded lossily rather than panicking after a successful parse.
    pub argv: Vec<String>,
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
pub fn capture_plan(args: &CaptureArgs, argv: Vec<String>) -> Result<CapturePlan> {
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
/// in a manifest and would otherwise be silently admitted and then missed.
///
/// **Repeats are a usage error**, for the same reason `--audio` rejects them: a
/// second identical `--files` entry finds the same candidate and is admitted
/// with the same capture id, which produces two admissions, two patchmap writes
/// (the second overwriting the first) and a `files_captured` count of two for
/// one file. Naming a path twice is never a different request.
fn validate_paths(paths: &[String], flag: &str) -> Result<()> {
    let mut seen: Vec<&str> = Vec::with_capacity(paths.len());
    for p in paths {
        if p.is_empty() {
            return Err(UsageError::new(format!("{flag} was given an empty path")));
        }
        if p.contains('\\') {
            return Err(UsageError::new(format!(
                "{flag} {p:?} uses backslashes; manifest paths are /-separated"
            )));
        }
        if p.starts_with('/') {
            return Err(UsageError::new(format!(
                "{flag} {p:?} is absolute; paths are relative to the game directory"
            )));
        }
        if p.contains(':') {
            return Err(UsageError::new(format!(
                "{flag} {p:?} looks like a drive-qualified path; paths are relative to \
                 the game directory"
            )));
        }
        if p.split('/').any(|c| c == "..") {
            return Err(UsageError::new(format!(
                "{flag} {p:?} escapes the game directory"
            )));
        }
        if p.split('/').any(|c| c.is_empty()) {
            return Err(UsageError::new(format!(
                "{flag} {p:?} has an empty component (a doubled or trailing separator)"
            )));
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
fn tag_shape_problem(tag: &str) -> Option<String> {
    if tag.is_empty() {
        return Some("was given an empty tag".to_string());
    }
    if tag.len() > 32 {
        return Some(format!(
            "{tag:?} is longer than 32 characters; that is not a version tag"
        ));
    }
    if tag.chars().any(char::is_control) {
        return Some(format!(
            "{tag:?} contains a control character; a tag is a bare version string"
        ));
    }
    if tag.contains('/') || tag.contains('\\') || tag.contains(' ') {
        return Some(format!(
            "{tag:?} contains a separator or a space; a tag is a bare version string"
        ));
    }
    if tag.contains(':') {
        return Some(format!(
            "{tag:?} looks like a drive-qualified or namespaced path; a tag is a \
             bare version string"
        ));
    }
    if tag.contains(['*', '?', '"', '<', '>', '|']) {
        return Some(format!(
            "{tag:?} contains a character Windows cannot use in a directory name; \
             a tag is a bare version string"
        ));
    }
    if tag.starts_with('.') || tag.ends_with('.') {
        return Some(format!(
            "{tag:?} starts or ends with `.`; Windows strips a trailing dot from a \
             directory name, so two tags could name one directory"
        ));
    }
    if tag.split(['/', '\\']).any(|c| c == "..") {
        return Some(format!("{tag:?} escapes the artifact directory"));
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
pub fn pair_dir(root: &Path, biz: Biz, old_tag: &str, new_tag: &str) -> PathBuf {
    root.join(biz.as_str())
        .join(format!("{old_tag}__{new_tag}"))
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
mod tests {
    use super::*;
    use crate::cli::{CaptureArgs, CommonArgs};

    fn args_with(mutate: impl FnOnce(&mut CaptureArgs)) -> CaptureArgs {
        let mut a = CaptureArgs {
            biz: Biz::Hk4eGlobal,
            out: PathBuf::from("artifacts"),
            new_tag: None,
            old_tag: None,
            per_class: 1,
            max_download_bytes: None,
            require_class: Vec::new(),
            files: Vec::new(),
            audio: vec![DEFAULT_AUDIO.to_string()],
            manifest_detail: ManifestDetail::Full,
            gzip_threshold: crate::cli::DEFAULT_GZIP_THRESHOLD,
            probe_tags: Vec::new(),
            no_chunk_blobs: false,
            jobs: 4,
            retries: 3,
            timeout: 60,
            max_response_bytes: crate::http::DEFAULT_MAX_RESPONSE_BYTES,
            hyp_base: None,
            sophon_base: None,
            common: CommonArgs {
                log_level: crate::logging::Level::Info,
                log_file_level: None,
                log_file: None,
                no_log_file: false,
                output: "text".into(),
            },
        };
        mutate(&mut a);
        a
    }

    fn plan(mutate: impl FnOnce(&mut CaptureArgs)) -> Result<CapturePlan> {
        capture_plan(&args_with(mutate), vec!["gircap".into(), "capture".into()])
    }

    #[test]
    fn a_zero_per_class_is_floored_once_in_the_plan() {
        // The floor belongs to the plan, not to each consumer: `select::decide`
        // and `emit::tree` both read this field, and a convention two places have
        // to remember is not a property of anything.
        //
        // Asserted on the *plan*, which is the point: a consumer reading
        // `plan.per_class` now gets a value that is already floored, and cannot
        // disagree with the other consumer about what `--per-class 0` means.
        let p = plan(|a| a.per_class = 0).expect("zero is floored, not rejected");
        assert_eq!(
            p.per_class, 1,
            "the plan must carry the floor, not the request"
        );

        // And it is the *same* reading for every other value, so nothing else
        // moved.
        for n in [1_u32, 2, 7] {
            let p = plan(|a| a.per_class = n).expect("valid");
            assert_eq!(p.per_class, n, "{n} must pass through untouched");
        }
    }

    #[test]
    fn a_misspelt_require_class_is_a_usage_error_before_anything_is_fetched() {
        // A misspelt class must be a usage error before a socket opens: the
        // test is in `config.rs` rather than in `run.rs` for exactly that reason,
        // because reaching it at all proves the check happens during planning.
        let err = plan(|a| a.require_class = vec!["identico".into()])
            .expect_err("an unknown class must not produce a plan");
        let msg = err.to_string();
        assert!(
            msg.contains("--require-class"),
            "the message must name the flag: {msg}"
        );
        assert!(
            msg.contains("identico"),
            "the message must name the value the user typed: {msg}"
        );
        assert!(
            msg.contains("identical"),
            "and the valid set, so the typo can be corrected without --help: {msg}"
        );
    }

    #[test]
    fn every_valid_require_class_survives_planning_and_keeps_its_order() {
        // Parsing early must not change what the plan carries. Empty stays empty:
        // the required set is the matrix's ordering, not a default of this flag.
        let p = plan(|a| a.require_class = vec![]).expect("empty is valid");
        assert!(
            p.require_class.is_empty(),
            "an empty --require-class must stay empty here, not be defaulted"
        );

        let all: Vec<String> = crate::select::Class::ALL
            .iter()
            .map(|c| c.id().to_string())
            .collect();
        let p = plan(|a| a.require_class = all.clone()).expect("every class is valid");
        assert_eq!(p.require_class.len(), all.len());
        assert_eq!(
            p.require_class.iter().map(|c| c.id()).collect::<Vec<_>>(),
            all.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "the order the user gave must be the order the plan carries"
        );
    }

    #[test]
    fn the_default_request_resolves_to_a_usable_plan() {
        let p = plan(|_| {}).expect("defaults are valid");
        assert_eq!(p.biz, Biz::Hk4eGlobal);
        assert_eq!(p.audio, AudioSelection::Codes(vec![DEFAULT_AUDIO.into()]));
        assert_eq!(p.jobs, 4);
        assert_eq!(p.retries, 3);
        assert_eq!(p.timeout, std::time::Duration::from_secs(60));
        assert_eq!(p.per_class, 1);
        assert!(!p.no_chunk_blobs);
        assert_eq!(p.gzip_threshold, crate::cli::DEFAULT_GZIP_THRESHOLD);
        assert_eq!(p.argv, vec!["gircap".to_string(), "capture".to_string()]);
    }

    #[test]
    fn audio_codes_are_normalized_and_case_folded() {
        let p = plan(|a| a.audio = vec!["EN_US".into(), "zh-cn".into()]).unwrap();
        assert_eq!(
            p.audio,
            AudioSelection::Codes(vec!["en-us".into(), "zh-cn".into()])
        );
    }

    #[test]
    fn audio_none_is_its_own_selection_and_mixes_with_nothing() {
        let p = plan(|a| a.audio = vec!["none".into()]).unwrap();
        assert_eq!(p.audio, AudioSelection::None);
        assert!(p.audio.codes().is_empty());
        assert_eq!(p.audio.to_string(), "none");

        let e = plan(|a| a.audio = vec!["none".into(), "ja-jp".into()]).unwrap_err();
        assert!(e.message().contains("none"), "{}", e.message());
        assert!(e.message().contains("ja-jp"), "{}", e.message());
    }

    #[test]
    fn a_repeated_audio_code_is_refused_rather_than_deduped() {
        let e = plan(|a| a.audio = vec!["en-us".into(), "en_us".into()]).unwrap_err();
        assert!(e.message().contains("twice"), "{}", e.message());
    }

    #[test]
    fn zero_jobs_zero_retries_and_zero_timeout_are_all_refused_by_name() {
        assert!(
            plan(|a| a.jobs = 0)
                .unwrap_err()
                .message()
                .contains("--jobs")
        );
        assert!(
            plan(|a| a.retries = 0)
                .unwrap_err()
                .message()
                .contains("--retries")
        );
        assert!(
            plan(|a| a.timeout = 0)
                .unwrap_err()
                .message()
                .contains("--timeout")
        );
    }

    #[test]
    fn absurd_concurrency_and_retry_counts_are_refused_with_a_reason() {
        assert!(plan(|a| a.jobs = 65).unwrap_err().message().contains("64"));
        assert!(
            plan(|a| a.retries = 11)
                .unwrap_err()
                .message()
                .contains("real failure")
        );
    }

    #[test]
    fn file_paths_that_cannot_name_a_manifest_entry_are_refused() {
        for bad in [
            r"C:\game\a.blk",
            "/abs/a.blk",
            "../escape",
            "a//b",
            "a/",
            "",
        ] {
            let e = plan(move |a| a.files = vec![bad.to_string()]).unwrap_err();
            assert_eq!(e.exit_code(), crate::exit::ExitCode::Usage, "{bad:?}");
            assert!(
                e.message().contains("--files"),
                "{bad:?} -> {}",
                e.message()
            );
        }
    }

    #[test]
    fn a_normal_manifest_relative_path_is_accepted() {
        let p = plan(|a| {
            a.files = vec![
                "GenshinImpact_Data/Plugins/HoYoChannel.dll".into(),
                "GenshinImpact_Data/StreamingAssets/AssetBundles/blocks/00/09801749.blk".into(),
            ]
        })
        .unwrap();
        assert_eq!(p.files.len(), 2);
    }

    #[test]
    fn a_repeated_files_path_is_a_usage_error() {
        // A duplicate finds the same candidate and would be admitted again with
        // the same capture id: two admissions, two patchmap writes (the second
        // overwriting the first) and a `files_captured` count of two for one
        // file. `--audio` already refuses a repeated value for the same reason.
        let e = plan(|a| {
            a.files = vec![
                "GenshinImpact_Data/Plugins/HoYoChannel.dll".into(),
                "GenshinImpact_Data/Plugins/HoYoChannel.dll".into(),
            ]
        })
        .unwrap_err();
        assert_eq!(e.exit_code(), crate::exit::ExitCode::Usage);
        assert!(e.message().contains("twice"), "{}", e.message());
        assert!(
            e.message().contains("HoYoChannel.dll"),
            "the message names the repeated path: {}",
            e.message()
        );
    }

    #[test]
    fn an_identical_pair_is_refused_because_it_would_exercise_nothing() {
        let e = plan(|a| {
            a.new_tag = Some("7.1.0".into());
            a.old_tag = Some("7.1.0".into());
        })
        .unwrap_err();
        assert!(e.message().contains("must differ"), "{}", e.message());
    }

    #[test]
    fn latest_is_accepted_for_new_tag_and_refused_for_a_probe() {
        assert!(
            plan(|a| a.new_tag = Some("latest".into())).is_ok(),
            "the sentinel belongs to --new-tag"
        );
        assert!(validate_tag("latest", "--probe-tags").is_err());
        assert!(plan(|a| a.probe_tags = vec!["latest".into()]).is_err());
    }

    #[test]
    fn a_tag_that_is_not_a_bare_version_string_is_refused() {
        for bad in ["", "7.1.0/../x", "7.1.0 beta", &"7".repeat(33)] {
            assert!(validate_tag(bad, "--old-tag").is_err(), "{bad:?}");
        }
        for good in ["7.1.0", "6.7.0", "5.5.0-nightly", "v7.1"] {
            assert!(validate_tag(good, "--old-tag").is_ok(), "{good:?}");
        }
    }

    #[test]
    fn a_tag_cannot_name_a_path_on_either_separator_or_carry_a_drive_prefix() {
        // A tag is joined into the artifact directory raw. On Windows `\` is a
        // separator and `C:` is a path prefix, so either one can move the
        // bundle - and `..\..` can move it out of the root. The same rule must
        // cover the pair flags and the probes, because all three reach the same
        // sink.
        for bad in [
            "C:",
            r"..\..",
            r"a\b",
            "a:b",
            "..",
            "7.1.0\u{7}",
            // Windows cannot create these directory names, and a late
            // `create_dir_all` failure would come after the network work.
            "7.1.0?",
            "7.1.0*",
            "7.1.0\"",
            "7.1.0<",
            "7.1.0>",
            "7.1.0|",
            // Windows strips a trailing dot, so this and `7.1.0` would collide.
            "7.1.0.",
            ".7.1.0",
        ] {
            assert!(
                validate_tag(bad, "--old-tag").is_err(),
                "--old-tag {bad:?} must be refused"
            );
            assert!(
                validate_new_tag(bad).is_err(),
                "--new-tag {bad:?} must be refused"
            );
            assert!(
                plan(|a| a.probe_tags = vec![bad.to_string()]).is_err(),
                "--probe-tags {bad:?} must be refused"
            );
        }
        // And the shape rule did not become a ban on ordinary tags.
        for good in ["7.1.0", "6.7.0", "5.5.0-nightly", "v7.1"] {
            assert!(validate_tag(good, "--old-tag").is_ok(), "{good:?}");
            assert!(validate_new_tag(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn metadata_only_is_a_flag_value_not_an_implicit_rejection() {
        let p = plan(|a| a.no_chunk_blobs = true).unwrap();
        assert!(p.no_chunk_blobs);
    }

    #[test]
    fn verify_and_materialize_resolve_their_own_arguments() {
        use crate::cli::{ListArgs, VerifyArgs};
        let v = verify_plan(&VerifyArgs {
            artifact: PathBuf::from("a"),
            common: CommonArgs {
                log_level: crate::logging::Level::Info,
                log_file_level: None,
                log_file: None,
                no_log_file: false,
                output: "json".into(),
            },
        })
        .unwrap();
        assert_eq!(v.artifact, PathBuf::from("a"));

        let m = materialize_plan(&MaterializeArgs {
            artifact: PathBuf::from("a"),
            out: None,
            common: CommonArgs {
                log_level: crate::logging::Level::Info,
                log_file_level: None,
                log_file: None,
                no_log_file: false,
                output: "text".into(),
            },
        })
        .unwrap();
        assert_eq!(m.out, None);

        let bad = materialize_plan(&MaterializeArgs {
            artifact: PathBuf::from("a"),
            out: Some(PathBuf::from("a")),
            common: CommonArgs {
                log_level: crate::logging::Level::Info,
                log_file_level: None,
                log_file: None,
                no_log_file: false,
                output: "text".into(),
            },
        })
        .unwrap_err();
        assert!(bad.message().contains("defeats it"), "{}", bad.message());

        // `list` needs no resolution beyond its root, but it must still parse.
        let l = ListArgs {
            root: PathBuf::from("artifacts"),
            common: CommonArgs {
                log_level: crate::logging::Level::Info,
                log_file_level: None,
                log_file: None,
                no_log_file: false,
                output: "text".into(),
            },
        };
        assert_eq!(l.root, PathBuf::from("artifacts"));
    }

    #[test]
    fn the_bundle_lives_under_biz_then_the_pair_then_the_id() {
        let d = pair_dir(Path::new("artifacts"), Biz::Hk4eGlobal, "7.0.0", "7.1.0");
        assert_eq!(
            d,
            PathBuf::from("artifacts")
                .join("hk4e_global")
                .join("7.0.0__7.1.0")
        );
    }

    /// The measured branch of `hk4e_global`, 2026-10-14.
    fn branch() -> crate::biz::BranchParams {
        crate::biz::BranchParams {
            package_id: "ScSYQBFhu9".into(),
            password: "bDL4JUHL625x".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec!["7.0.0".into(), "6.7.0".into()],
        }
    }

    fn pair_plan(new_tag: Option<&str>, old_tag: Option<&str>) -> CapturePlan {
        plan(move |a| {
            a.new_tag = new_tag.map(str::to_string);
            a.old_tag = old_tag.map(str::to_string);
        })
        .unwrap()
    }

    /// The default is `diff_tags[0]` — "current minus one" — measured on
    /// 2026-10-14 as 7.1.0 -> 7.0.0, and each half records which rule chose it.
    #[test]
    fn the_pair_defaults_to_the_branch_head_and_its_first_diff_tag() {
        let p = pair_plan(None, None);
        let v = VersionPair::select(&branch(), p.new_selector(), p.old_selector()).unwrap();
        assert_eq!(v.tags(), ["7.1.0", "7.0.0"]);
        assert_eq!(v.new.source, "main.tag");
        assert_eq!(v.old.source, "diff_tags[0]");
        assert!(!v.new.resolved(), "no build id before getBuild is asked");
        assert!(!v.old.resolved());
        assert_eq!(v.diff_tags, vec!["7.0.0", "6.7.0"]);
    }

    #[test]
    fn the_latest_sentinel_selects_the_branch_head_and_an_explicit_tag_does_not() {
        for (flag, want) in [
            (None, NewTagSelector::BranchHead),
            (Some("latest"), NewTagSelector::BranchHead),
            (Some("LATEST"), NewTagSelector::BranchHead),
            (Some("6.6.0"), NewTagSelector::Explicit("6.6.0")),
        ] {
            assert_eq!(pair_plan(flag, None).new_selector(), want, "{flag:?}");
        }
        assert_eq!(
            pair_plan(None, None).old_selector(),
            OldTagSelector::FirstDiffTag
        );
        assert_eq!(
            pair_plan(None, Some("5.5.0")).old_selector(),
            OldTagSelector::Explicit("5.5.0")
        );
    }

    /// The `source` string is a claim a reader checks against the captured
    /// `getGameBranches`, so it has to distinguish the two routes to each tag.
    #[test]
    fn an_explicit_tag_records_that_it_was_explicit() {
        let p = pair_plan(Some("6.6.0"), Some("5.5.0"));
        let v = VersionPair::select(&branch(), p.new_selector(), p.old_selector()).unwrap();
        assert_eq!(v.tags(), ["6.6.0", "5.5.0"]);
        assert_eq!(v.new.source, "--new-tag");
        assert_eq!(v.old.source, "--old-tag");
    }

    /// A branch with no `diff_tags` cannot supply a default old version, and
    /// inventing one would mean capturing the wrong version pair.
    #[test]
    fn an_empty_diff_tags_is_an_error_naming_the_remedy() {
        let mut b = branch();
        b.diff_tags.clear();
        let p = pair_plan(None, None);
        let e = VersionPair::select(&b, p.new_selector(), p.old_selector()).unwrap_err();
        assert!(e.contains("--old-tag"), "{e}");
        assert!(e.contains("diff_tags"), "{e}");
        // An explicit tag still works: the caller asked, and getBuild will say.
        let p2 = pair_plan(None, Some("5.5.0"));
        assert_eq!(
            VersionPair::select(&b, p2.new_selector(), p2.old_selector())
                .unwrap()
                .old
                .tag,
            "5.5.0"
        );
    }

    /// A hostile or compromised API can supply `main.tag`/`diff_tags[0]`
    /// without the caller ever passing a bad flag. Those strings reach
    /// `pair_dir`, so they must pass the same shape check the flags do.
    #[test]
    fn api_resolved_tags_are_validated_before_they_name_a_directory() {
        let p = pair_plan(None, None);

        let mut b = branch();
        b.tag = r"..\..\escape".into();
        let e = VersionPair::select(&b, p.new_selector(), p.old_selector()).unwrap_err();
        assert!(
            e.contains("main.tag") && e.contains("escape"),
            "the message names the API field and the offending tag: {e}"
        );

        let mut b = branch();
        b.diff_tags[0] = "C:evil".into();
        let e = VersionPair::select(&b, p.new_selector(), p.old_selector()).unwrap_err();
        assert!(
            e.contains("diff_tags[0]") && e.contains("drive-qualified"),
            "the default old tag is validated too: {e}"
        );

        // An explicit tag is validated in `capture_plan`, and again here for a
        // direct caller of `select`.
        let e = VersionPair::select(
            &branch(),
            NewTagSelector::Explicit("7.1.0?"),
            OldTagSelector::FirstDiffTag,
        )
        .unwrap_err();
        assert!(e.contains("--new-tag"), "{e}");
    }

    /// Swapping them would attach a build id to the wrong tag, and every chunk
    /// URL in the bundle is namespaced by build id.
    #[test]
    fn build_ids_attach_new_then_old() {
        let mut v = VersionPair::select(
            &branch(),
            NewTagSelector::BranchHead,
            OldTagSelector::FirstDiffTag,
        )
        .unwrap();
        v.attach_build_ids("sMXGW2ll3Fuu", "3XnPviMfoILi");
        assert_eq!(v.new.tag, "7.1.0");
        assert_eq!(v.new.build_id, "sMXGW2ll3Fuu");
        assert_eq!(v.old.build_id, "3XnPviMfoILi");
        assert!(v.new.resolved() && v.old.resolved());
    }

    #[test]
    fn base_overrides_must_be_absolute_and_https_unless_loopback() {
        for good in [
            "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api",
            "http://127.0.0.1:8080/hyp",
            "http://localhost:8080/hyp",
        ] {
            assert!(validate_base(Some(good), "--hyp-base").is_ok(), "{good}");
        }
        for bad in [
            "not a url",
            "ftp://x/api",
            "http://example.com/api",
            "/relative",
        ] {
            let e = validate_base(Some(bad), "--sophon-base").unwrap_err();
            assert_eq!(e.exit_code(), crate::exit::ExitCode::Usage, "{bad:?}");
        }
        let e = validate_base(Some("http://example.com/api"), "--hyp-base").unwrap_err();
        assert!(e.message().contains("plaintext"), "{}", e.message());
    }

    /// Normalised once, here, so no join site has to remember.
    #[test]
    fn a_base_override_loses_its_trailing_slash() {
        assert_eq!(
            validate_base(Some("https://x/api///"), "--hyp-base").unwrap(),
            Some("https://x/api".to_string())
        );
        assert_eq!(validate_base(None, "--hyp-base").unwrap(), None);
    }

    #[test]
    fn an_override_reaches_the_plan_and_defaults_to_none() {
        let p = plan(|a| {
            a.hyp_base = Some("http://127.0.0.1:9/hyp".into());
            a.sophon_base = None;
        })
        .unwrap();
        assert_eq!(p.hyp_base.as_deref(), Some("http://127.0.0.1:9/hyp"));
        assert_eq!(p.sophon_base, None);
        assert_eq!(plan(|_| {}).unwrap().hyp_base, None);
    }

    /// A directory listing of a root has to sort as a history, which is the whole
    /// reason the id is a timestamp and not a random or content-derived name.
    #[test]
    fn the_capture_id_is_a_sortable_utc_timestamp() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-10-14T10:15:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(capture_id(t), "20261014T101500.000000Z");
        let later = t + chrono::Duration::seconds(1);
        assert!(capture_id(later) > capture_id(t));

        let full = artifact_dir(Path::new("artifacts"), Biz::Hk4eGlobal, "7.0.0", "7.1.0", t);
        assert!(full.ends_with("20261014T101500.000000Z"));
        assert_eq!(
            full.parent().unwrap(),
            pair_dir(Path::new("artifacts"), Biz::Hk4eGlobal, "7.0.0", "7.1.0")
        );
    }

    #[test]
    fn document_paths_are_slash_separated_and_rooted_at_the_artifact() {
        let root = Path::new("artifacts/hk4e_global/7.0.0__7.1.0/20261014T101500.000000Z");
        let deep = root.join("game").join("GenshinImpact_Data").join("Plugins");
        assert_eq!(
            rel_path_string(root, &deep),
            "game/GenshinImpact_Data/Plugins"
        );
        // A path outside the root is reported whole rather than silently
        // relativised into something that looks in-tree.
        assert_eq!(
            rel_path_string(root, Path::new("/elsewhere/x")),
            "/elsewhere/x"
        );
    }
}
