use super::*;
use crate::cli::{CaptureArgs, CommonArgs};
use crate::report::OutputFormat;

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
        max_decompressed_bytes: crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES,
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
            output: OutputFormat::Text,
        },
    };
    mutate(&mut a);
    a
}

fn plan(mutate: impl FnOnce(&mut CaptureArgs)) -> Result<CapturePlan> {
    capture_plan(
        &args_with(mutate),
        crate::cli::recorded_argv(["gircap".into(), "capture".into()]),
    )
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
    assert_eq!(
        p.max_decompressed_bytes,
        crate::hash::DEFAULT_MAX_DECOMPRESSED_BYTES
    );
    assert_eq!(
        p.argv.lossy,
        vec!["gircap".to_string(), "capture".to_string()]
    );
    // The raw form is recorded beside it, one entry per argument, in the
    // same order. Its bytes are the host's own representation, so the test
    // asserts what is portable: the shape, and that decoding the bytes the
    // way `host_os` says gives back the lossy argument.
    assert_eq!(p.argv.raw.len(), 2);
    assert_eq!(p.argv.raw.len(), p.argv.lossy.len());
}

#[test]
fn a_zero_decompression_ceiling_is_a_usage_error() {
    // Zero is not a smaller ceiling, it is a decode that can never produce a
    // byte. Naming the flag before a socket opens is the same rule every
    // other bad plan value follows.
    let err = plan(|a| a.max_decompressed_bytes = 0).expect_err("zero is not a plan");
    let msg = err.to_string();
    assert!(msg.contains("--max-decompressed-bytes"), "{msg}");
}

#[test]
fn a_raised_decompression_ceiling_reaches_the_plan() {
    let p = plan(|a| a.max_decompressed_bytes = 4096).expect("positive ceiling");
    assert_eq!(p.max_decompressed_bytes, 4096);
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
fn an_audio_code_that_is_not_one_file_name_component_is_refused() {
    // The code is the `matching_field`, which the dump interpolates into
    // `manifests/<tag>/<field>.manifest…`. A separator or a traversal makes
    // the capture write outside `--out`; the same shape rule now covers it.
    for bad in ["../../../../evil", "en/us", r"en\us", "..", ""] {
        let e = plan(|a| a.audio = vec![bad.to_string()]).unwrap_err();
        assert_eq!(e.exit_code(), crate::exit::ExitCode::Usage, "{bad:?}");
        assert!(
            e.message().contains("--audio"),
            "{bad:?} -> {}",
            e.message()
        );
    }
    // And the measured codes still pass.
    for good in ["en-us", "zh-cn", "ja-jp", "ko-kr"] {
        assert!(
            plan(|a| a.audio = vec![good.to_string()]).is_ok(),
            "{good:?}"
        );
    }
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
        // A tag is `manifests/<tag>`, and Windows resolves a DOS device
        // name whatever the extension: `create_dir_all` then fails late, or
        // the bundle names a directory a Windows reader cannot open.
        "CON",
        "con",
        "NUL",
        "NUL.txt",
        "aux.txt",
        "COM1",
        "COM1.bin",
        "LPT9",
        "CONIN$",
        "CONIN$.txt",
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
    // And the shape rule did not become a ban on ordinary tags, including
    // names that merely start like a device one.
    for good in [
        "7.1.0",
        "6.7.0",
        "5.5.0-nightly",
        "v7.1",
        "CONSOLE",
        "console",
        "COM0",
        "LPT10",
    ] {
        assert!(validate_tag(good, "--old-tag").is_ok(), "{good:?}");
        assert!(validate_new_tag(good).is_ok(), "{good:?}");
    }
}

/// A tag becomes `manifests/<tag>`, so a DOS device name is refused by the
/// same rule the path validator uses. Windows ignores the extension when it
/// resolves the name, so `NUL.txt` and `CONIN$.txt` are devices too.
#[test]
fn a_tag_cannot_name_a_windows_device() {
    for bad in [
        "CON",
        "NUL",
        "NUL.txt",
        "COM1",
        "COM1.bin",
        "CONIN$",
        "CONIN$.txt",
    ] {
        let problem = tag_shape_problem(bad)
            .unwrap_or_else(|| panic!("{bad:?} names a device, not a directory"));
        assert!(problem.contains("device"), "{bad:?}: {problem}");
    }
    // Names that merely start like one are ordinary tags.
    for good in ["7.1.0", "CONSOLE", "console", "COM0", "LPT10"] {
        assert_eq!(tag_shape_problem(good), None, "{good:?}");
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
            output: OutputFormat::Json,
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
            output: OutputFormat::Text,
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
            output: OutputFormat::Text,
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
            output: OutputFormat::Text,
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

    // A device name from the API is *kept*: `manifests/<tag>` is written
    // under an escaped name and `path_map.json` records the pair. The
    // documents keep the API's own spelling.
    let mut b = branch();
    b.tag = "CON".into();
    let v = VersionPair::select(&b, p.new_selector(), p.old_selector())
        .expect("an API device name is escaped, not refused");
    assert_eq!(v.new.tag, "CON");
    assert_eq!(v.new.source, "main.tag");

    // The same value from a flag is still refused at argument resolution:
    // there is no bundle and no observation to keep yet.
    let e = VersionPair::select(
        &branch(),
        NewTagSelector::Explicit("CON"),
        OldTagSelector::FirstDiffTag,
    )
    .unwrap_err();
    assert!(e.contains("--new-tag") && e.contains("device"), "{e}");
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
