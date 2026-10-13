//! Flags resolved into a plan.
//!
//! A [`CaptureArgs`] is what the user typed. A [`CapturePlan`] is what the run
//! will do: the version pair resolved against a live API answer, the selection
//! pool widened by `--audio`, the budget made concrete, the paths normalised.
//! The split is what lets a test build a plan without a command line, and what
//! keeps one rule from being applied twice under two spellings.
//!
//! This milestone lands the vocabulary and the flag-level validation. The parts
//! that need an API answer — the version pair, the branch's `package_id` and
//! `password` — arrive with [`crate::hyp`] in M2, because a plan whose `old_tag`
//! came from `diff_tags[0]` cannot be built without one.

use std::path::{Path, PathBuf};

use crate::cli::{Biz, CaptureArgs, ManifestDetail, MaterializeArgs, Result, UsageError, VerifyArgs};
use crate::report::OutputFormat;

/// The audio language selected when `--audio` is not passed.
///
/// Both a launchable-install default and a deliberate widening of the selection
/// pool: every selected language adds a manifest to the corpus the matrix is
/// scored against, which is the backstop for a class with no `game`-field
/// candidate in a given version pair.
pub const DEFAULT_AUDIO: &str = "en-us";

/// The `--audio` spelling that means "game only, no voice packs".
pub const AUDIO_NONE: &str = "none";

/// The gzip threshold applied when `--gzip-threshold` is not passed.
pub const DEFAULT_GZIP_THRESHOLD: u64 = 1_048_576;

/// A `--no-chunk-blobs` run.
pub const METADATA_ONLY_NOTE: &str =
    "metadata-only run: assertions A7-A10 are not provable without chunk blobs";

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
    /// Files admitted per matrix class.
    pub per_class: u32,
    /// Summed-cost ceiling over admitted files.
    pub max_download_bytes: Option<u64>,
    /// Classes whose absence must fail the run.
    pub require_class: Vec<String>,
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
    /// HoYoPlay base override; `None` = the production base for [`Self::biz`].
    pub hyp_base: Option<String>,
    /// Sophon base override; `None` = the production base for [`Self::biz`].
    pub sophon_base: Option<String>,
    /// stdout rendering.
    pub output: OutputFormat,
    /// The argv as given, recorded in the artifact so a bundle says how it was made.
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
    let loopback = matches!(url.host_str(), Some("127.0.0.1") | Some("localhost") | Some("::1"));
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

    /// Whether a voice pack is written into the fixture.
    pub fn is_some(&self) -> bool {
        matches!(self, AudioSelection::Codes(_))
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

/// The concurrency a capture is allowed to use, validated once.
#[derive(Debug, Clone, Copy)]
pub struct Transport {
    /// Concurrent chunk downloads.
    pub jobs: usize,
    /// Attempts per request, including the first.
    pub retries: u32,
    /// Per-request timeout.
    pub timeout: std::time::Duration,
}

impl Transport {
    /// A transport that performs exactly one attempt and never waits.
    ///
    /// For tests driving a local mock, where a retry would turn a fast failure
    /// into a slow one and hide the cause.
    pub const fn immediate() -> Self {
        Transport {
            jobs: 1,
            retries: 1,
            timeout: std::time::Duration::from_secs(10),
        }
    }
}

/// Validate a flag-level request into a [`CapturePlan`].
///
/// This is the whole of M0's share of the work: it cannot resolve a version pair
/// (no API yet), but everything that is wrong in the *request* is caught here, so
/// a bad flag is rejected before a socket is opened rather than after.
pub fn capture_plan(args: &CaptureArgs, argv: Vec<String>) -> Result<CapturePlan> {
    let transport = validate_transport(args.jobs, args.retries, args.timeout)?;
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
        per_class: args.per_class,
        max_download_bytes: args.max_download_bytes,
        // An empty `--require-class` means "all required"; the set of required
        // classes is fixed by the matrix, so resolving it here would duplicate
        // the matrix's ordering. It stays a `Vec` and is defaulted in M5, where
        // the matrix it refers to exists.
        require_class: args.require_class.clone(),
        files: args.files.clone(),
        audio,
        manifest_detail: args.manifest_detail,
        gzip_threshold: args.gzip_threshold,
        probe_tags: args.probe_tags.clone(),
        no_chunk_blobs: args.no_chunk_blobs,
        jobs: transport.jobs,
        retries: transport.retries,
        timeout: transport.timeout,
        hyp_base: validate_base(args.hyp_base.as_deref(), "--hyp-base")?,
        sophon_base: validate_base(args.sophon_base.as_deref(), "--sophon-base")?,
        output: args.common.output.parse::<OutputFormat>().map_err(UsageError::new)?,
        argv,
    })
}

/// Validate the transport flags together, naming the one at fault.
fn validate_transport(jobs: usize, retries: u32, timeout_secs: u64) -> Result<Transport> {
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
        return Err(UsageError::new("--retries must be at least 1 (it counts the first attempt)"));
    }
    if retries > 10 {
        return Err(UsageError::new(format!(
            "--retries {retries} exceeds 10; past that a failure is a real failure"
        )));
    }
    if timeout_secs == 0 {
        return Err(UsageError::new("--timeout must be at least 1 second"));
    }
    Ok(Transport {
        jobs,
        retries,
        timeout: std::time::Duration::from_secs(timeout_secs),
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
fn validate_paths(paths: &[String], flag: &str) -> Result<()> {
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
fn validate_tag_shape(tag: &str, flag: &str) -> Result<()> {
    if tag.is_empty() {
        return Err(UsageError::new(format!("{flag} was given an empty tag")));
    }
    if tag.len() > 32 {
        return Err(UsageError::new(format!(
            "{flag} {tag:?} is longer than 32 characters; that is not a version tag"
        )));
    }
    if tag.contains('/') || tag.contains(' ') {
        return Err(UsageError::new(format!(
            "{flag} {tag:?} contains a separator or a space; a tag is a bare version string"
        )));
    }
    Ok(())
}

/// `gircap verify` resolved and checked.
#[derive(Debug, Clone)]
pub struct VerifyPlan {
    /// The bundle to re-check.
    pub artifact: PathBuf,
    /// stdout rendering.
    pub output: OutputFormat,
}

/// `gircap materialize` resolved and checked.
#[derive(Debug, Clone)]
pub struct MaterializePlan {
    /// The bundle to rebuild from.
    pub artifact: PathBuf,
    /// Where to write, if not beside the bundle.
    pub out: Option<PathBuf>,
    /// stdout rendering.
    pub output: OutputFormat,
}

/// Resolve `gircap verify`'s arguments.
pub fn verify_plan(args: &VerifyArgs) -> Result<VerifyPlan> {
    Ok(VerifyPlan {
        artifact: args.artifact.clone(),
        output: args.common.output.parse::<OutputFormat>().map_err(UsageError::new)?,
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
        output: args.common.output.parse::<OutputFormat>().map_err(UsageError::new)?,
    })
}

/// `artifacts/<biz>/<old>__<new>/` — the directory a bundle's id sits in.
pub fn pair_dir(root: &Path, biz: Biz, old_tag: &str, new_tag: &str) -> PathBuf {
    root.join(biz.as_str())
        .join(format!("{old_tag}__{new_tag}"))
}

/// The capture id: a UTC timestamp, `YYYYMMDDTHHMMSSZ`.
///
/// Time-ordered so a directory listing of a root sorts as a history, and named
/// for the moment the capture started rather than finished, so a long run's id
/// still says when the run began.
pub fn capture_id(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y%m%dT%H%M%SZ").to_string()
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
            gzip_threshold: DEFAULT_GZIP_THRESHOLD,
            probe_tags: Vec::new(),
            no_chunk_blobs: false,
            jobs: 4,
            retries: 3,
            timeout: 60,
            hyp_base: None,
            sophon_base: None,
            common: CommonArgs {
                log_level: "info".into(),
                log_file: None,
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
    fn the_default_request_resolves_to_a_usable_plan() {
        let p = plan(|_| {}).expect("defaults are valid");
        assert_eq!(p.biz, Biz::Hk4eGlobal);
        assert_eq!(p.audio, AudioSelection::Codes(vec![DEFAULT_AUDIO.into()]));
        assert_eq!(p.jobs, 4);
        assert_eq!(p.retries, 3);
        assert_eq!(p.timeout, std::time::Duration::from_secs(60));
        assert_eq!(p.per_class, 1);
        assert!(!p.no_chunk_blobs);
        assert_eq!(p.gzip_threshold, DEFAULT_GZIP_THRESHOLD);
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
        assert!(!p.audio.is_some());
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
        assert!(plan(|a| a.jobs = 0).unwrap_err().message().contains("--jobs"));
        assert!(plan(|a| a.retries = 0)
            .unwrap_err()
            .message()
            .contains("--retries"));
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
            plan(|a| a.new_tag = Some("latest".into()))
                .is_ok(),
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
    fn metadata_only_is_a_flag_value_not_an_implicit_rejection() {
        let p = plan(|a| a.no_chunk_blobs = true).unwrap();
        assert!(p.no_chunk_blobs);
        assert!(METADATA_ONLY_NOTE.contains("A7-A10"));
    }

    #[test]
    fn verify_and_materialize_resolve_their_own_arguments() {
        use crate::cli::{ListArgs, VerifyArgs};
        let v = verify_plan(&VerifyArgs {
            artifact: PathBuf::from("a"),
            common: CommonArgs {
                log_level: "info".into(),
                log_file: None,
                output: "json".into(),
            },
        })
        .unwrap();
        assert_eq!(v.artifact, PathBuf::from("a"));
        assert_eq!(v.output, OutputFormat::Json);

        let m = materialize_plan(&MaterializeArgs {
            artifact: PathBuf::from("a"),
            out: None,
            common: CommonArgs {
                log_level: "info".into(),
                log_file: None,
                output: "text".into(),
            },
        })
        .unwrap();
        assert_eq!(m.out, None);

        let bad = materialize_plan(&MaterializeArgs {
            artifact: PathBuf::from("a"),
            out: Some(PathBuf::from("a")),
            common: CommonArgs {
                log_level: "info".into(),
                log_file: None,
                output: "text".into(),
            },
        })
        .unwrap_err();
        assert!(bad.message().contains("defeats it"), "{}", bad.message());

        // `list` needs no resolution beyond its root, but it must still parse.
        let l = ListArgs {
            root: PathBuf::from("artifacts"),
            common: CommonArgs {
                log_level: "info".into(),
                log_file: None,
                output: "text".into(),
            },
        };
        assert_eq!(l.root, PathBuf::from("artifacts"));
    }

    #[test]
    fn the_bundle_lives_under_biz_then_the_pair_then_the_id() {
        let d = pair_dir(Path::new("artifacts"), Biz::Hk4eGlobal, "7.0.0", "7.1.0");
        assert_eq!(d, PathBuf::from("artifacts").join("hk4e_global").join("7.0.0__7.1.0"));
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
        for bad in ["not a url", "ftp://x/api", "http://example.com/api", "/relative"] {
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
        assert_eq!(capture_id(t), "20261014T101500Z");
        let later = t + chrono::Duration::seconds(1);
        assert!(capture_id(later) > capture_id(t));

        let full = artifact_dir(Path::new("artifacts"), Biz::Hk4eGlobal, "7.0.0", "7.1.0", t);
        assert!(full.ends_with("20261014T101500Z"));
        assert_eq!(full.parent().unwrap(), pair_dir(
            Path::new("artifacts"),
            Biz::Hk4eGlobal,
            "7.0.0",
            "7.1.0"
        ));
    }

    #[test]
    fn document_paths_are_slash_separated_and_rooted_at_the_artifact() {
        let root = Path::new("artifacts/hk4e_global/7.0.0__7.1.0/20261014T101500Z");
        let deep = root.join("game").join("GenshinImpact_Data").join("Plugins");
        assert_eq!(
            rel_path_string(root, &deep),
            "game/GenshinImpact_Data/Plugins"
        );
        // A path outside the root is reported whole rather than silently
        // relativised into something that looks in-tree.
        assert_eq!(rel_path_string(root, Path::new("/elsewhere/x")), "/elsewhere/x");
    }
}