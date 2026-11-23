//! Scan and `--check-only` cases: reporting damage without repairing it.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common::{
    ChunkSpec, FileSpec, FixtureSpec, LATEST_TAG, LOCAL_TAG, Lies, MockOpts, MockServer,
    PLAN_SCAN_BYTES_KEY, build_fixture, build_fixture_spec, build_moved_fixture, ctx_for, field,
    load_counters, logs_of, read_game_file, run, run_binary, temp_dir, write_game_file,
};
use girpr::{RunExit, Summary};
use girpr_repair_policy::RepairMode;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::Ordering;

// ---------------------------------------------------------------------------
// in-process plan/estimate helpers
// ---------------------------------------------------------------------------

/// Sum the per-file `est_download_bytes` recorded in a `--plan-out` document.
///
/// This is the figure the `PLAN` line folds the same document into, so reading
/// it from the artifact the library writes needs no binary spawn.
fn plan_out_estimate(path: &Path) -> u64 {
    let written = std::fs::read_to_string(path).expect("--plan-out file");
    let v: serde_json::Value = serde_json::from_str(&written).expect("valid JSON");
    v["repair"].as_array().unwrap().iter().fold(0u64, |n, e| {
        n.saturating_add(e["est_download_bytes"].as_u64().unwrap())
    })
}

/// Drive `girpr::run` in-process with a `--plan-out` artifact, the way the
/// estimate cases used to drive the binary. `print_plan` makes the run read-only
/// (it returns right after publishing the document); `scan`, `repair_mode` and
/// `ram_bytes` map the remaining flags. No process is spawned.
async fn run_plan(
    dir: &Path,
    mock: &MockServer,
    scan: bool,
    print_plan: bool,
    repair_mode: Option<RepairMode>,
    ram_bytes: Option<u64>,
    out: &Path,
) -> (u64, Summary, RunExit) {
    let mut ctx = ctx_for(dir.to_path_buf(), mock, 1);
    ctx.scan = scan;
    ctx.print_plan = print_plan;
    if let Some(mode) = repair_mode {
        ctx.repair_mode = mode;
        ctx.repair_mode_explicit = true;
    }
    if let Some(bytes) = ram_bytes {
        ctx.limits.ram_bytes = Some(bytes);
    }
    ctx.plan_out = Some(out.to_path_buf());
    let (summary, code) = run(ctx).await.expect("in-process plan run");
    (plan_out_estimate(out), summary, code)
}

// ---------------------------------------------------------------------------
// zero-length manifest entries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_only_clean_reports_success() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("check_clean");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("check-only runs");
    assert_eq!(code, RunExit::Success);
    let (skipped, _rep, failed, _, _) = load_counters(&summary);
    assert_eq!(failed, 0);
    assert_eq!(skipped, 2);

    mock.stop();
}

#[tokio::test]
async fn check_only_dirty_reports_damage_and_writes_nothing() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("check_dirty");
    let fx2 = build_fixture();
    let foo = &fx2.files[0];
    let bar = &fx2.files[1];
    write_game_file(&dir, &bar.rel, &bar.data);
    write_game_file(&dir, &foo.rel, b"definitely not the game data");

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    let (summary, code) = run(ctx).await.expect("check-only runs");
    assert_eq!(code, RunExit::CheckFailed, "check-only damage exit");
    let (_sk, _rep, failed, _, _) = load_counters(&summary);
    assert_eq!(failed, 1);

    // Read-only contract: corrupt bytes untouched, no config bump, no tmps.
    assert_eq!(
        read_game_file(&dir, &foo.rel).unwrap(),
        b"definitely not the game data"
    );
    assert!(!dir.join("config.ini").exists());
    assert!(!dir.join("game").join("foo.dat_tmp").exists());

    mock.stop();
}

#[tokio::test]
async fn check_only_matches_between_one_and_many_jobs() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let fx2 = build_fixture();
    let foo = &fx2.files[0];
    let bar = &fx2.files[1];

    let mut runs = Vec::new();
    for jobs in [1usize, 4] {
        let dir = temp_dir(&format!("check_parity_{jobs}"));
        write_game_file(&dir, &bar.rel, &bar.data);
        // Same length, different bytes: a digest failure, not a size one.
        let mut bad_foo = foo.data.clone();
        for b in bad_foo.iter_mut().skip(64) {
            *b ^= 0xFF;
        }
        write_game_file(&dir, &foo.rel, &bad_foo);

        let mut ctx = ctx_for(dir.to_path_buf(), &mock, jobs);
        ctx.check_only = true;
        let (summary, code) = run(ctx).await.expect("check-only runs");
        runs.push((load_counters(&summary), code));
    }

    assert_eq!(
        runs[0], runs[1],
        "jobs must not change the check's verdict or counters"
    );
    assert_eq!(
        runs[0].1,
        RunExit::CheckFailed,
        "the corrupt file is still damage"
    );
    let (skipped, _repaired, failed, _, _) = runs[0].0;
    assert_eq!((skipped, failed), (1, 1));

    mock.stop();
}

#[tokio::test]
async fn check_only_scan_keeps_the_damaged_chunk_total_on_the_summary() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("check_damaged_chunks_summary");
    let fx2 = build_fixture();
    let foo = &fx2.files[0];
    let bar = &fx2.files[1];
    write_game_file(&dir, &bar.rel, &bar.data);
    // Same length, different bytes: a digest failure the scan can locate
    // chunk by chunk.
    let mut bad_foo = foo.data.clone();
    for b in bad_foo.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &foo.rel, &bad_foo);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    ctx.scan = true;
    let (summary, code) = run(ctx).await.expect("check-only runs");
    assert_eq!(
        code,
        RunExit::CheckFailed,
        "the corrupt file is still damage"
    );

    let checked = summary
        .check_damaged_chunks()
        .expect("the check measured the tally");
    assert!(checked > 0, "the check carries the chunks it found wrong");
    assert_eq!(
        Some(checked),
        summary.flags.damaged_chunks,
        "the check's tally matches the scan figure it was given"
    );

    mock.stop();
}

#[tokio::test]
async fn check_only_scan_treats_an_absent_empty_file_as_damage() {
    // The scan's verdict for an absent file carries the empty digest, and a
    // legitimate empty file's manifest entry has exactly that digest. The
    // md5 alone therefore cannot tell them apart.
    let spec = FixtureSpec::new(vec![FileSpec::new("game/empty.dat", vec![])]);
    let mock = MockServer::start(build_fixture_spec(&spec), MockOpts::default()).await;
    let dir = temp_dir("check_empty_absent");

    for scan in [false, true] {
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        ctx.check_only = true;
        ctx.scan = scan;
        let (summary, code) = run(ctx).await.expect("check-only runs");
        assert_eq!(
            code,
            RunExit::CheckFailed,
            "scan={scan}: an absent file is damage"
        );
        assert_eq!(
            summary.files_failed.load(Ordering::Relaxed),
            1,
            "scan={scan}"
        );
    }

    // The same manifest entry, with the file present and empty: clean.
    write_game_file(&dir, "game/empty.dat", &[]);
    for scan in [false, true] {
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        ctx.check_only = true;
        ctx.scan = scan;
        let (_summary, code) = run(ctx).await.expect("check-only runs");
        assert_eq!(
            code,
            RunExit::Success,
            "scan={scan}: an empty file that exists is clean"
        );
    }

    mock.stop();
}

#[tokio::test]
async fn the_scanned_estimate_knows_which_sources_the_ordered_tier_loses() {
    // N-17: the scan finds both moved sources intact, but the ordered tier's
    // write order keeps only one direction's worth. The estimate must model
    // the writer's order, not "the source verified", or the plan's estimate is
    // below the bytes the same run downloads.
    let fx = build_moved_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("scan_ordered");
    let (old_rel, old_bytes) = build_moved_fixture().old_data.expect("the old file");
    std::fs::write(
        dir.join("config.ini"),
        format!("[General]\ngame_version={LOCAL_TAG}\n"),
    )
    .unwrap();
    write_game_file(&dir, &old_rel, &old_bytes);
    // One scanned in-process run at the ordered budget: the plan document is
    // written before execution, so its per-file estimate and the SUMMARY's real
    // download come from a single run — no separate `--print-plan` spawn. The
    // document's per-file halves are the same split the PLAN total folds from,
    // so this checks the same estimate-vs-reality parity as the PLAN line.
    // `--max-ram-bytes 0` maps to a zero RAM ceiling, which forces the ordered
    // tier this case is about.
    let out = dir.join("plan.json");
    let (estimate, summary, code) = run_plan(&dir, &mock, true, false, None, Some(0), &out).await;
    assert_eq!(code, RunExit::Success);
    let downloaded = summary.download.settled();
    assert_eq!(
        estimate, downloaded,
        "the scanned estimate is the ordered tier's real cost: {estimate} vs {downloaded}"
    );
    assert_eq!(
        mock.chunk_hits().into_iter().collect::<BTreeSet<_>>(),
        ["chunk_p", "chunk_t"]
            .into_iter()
            .map(String::from)
            .collect::<BTreeSet<_>>(),
        "the source the losing direction needed is downloaded"
    );

    mock.stop();
}

#[tokio::test]
async fn the_scanned_estimate_is_the_bytes_the_repair_really_downloads() {
    // The assertion that did not exist before `--scan`: the plan's estimate
    // and the repair's actual cost are the same number. Two modes share the
    // parity assertion below; the in-place path keeps the correct chunk where
    // it lies (so an unscanned estimate is an upper bound), while the temp
    // path re-hashes a candidate and fetches the corrupt chunk instead.
    let mock = MockServer::start(build_fixture(), MockOpts::default()).await;

    for (name, tag, temp, upper_bound) in [
        ("inplace", LOCAL_TAG, None, true),
        ("temp", LATEST_TAG, Some(RepairMode::Temp), false),
    ] {
        let dir = temp_dir(&format!("scan_exact_{name}"));
        if temp.is_some() {
            // config.ini names the installed build, so the temp path has
            // candidates to re-hash. The in-place case has none.
            std::fs::write(
                dir.join("config.ini"),
                format!("[General]\ngame_version={tag}\n"),
            )
            .unwrap();
        }
        let fx = build_fixture();
        write_game_file(&dir, &fx.files[1].rel, &fx.files[1].data);
        let mut half_bad = fx.files[0].data.clone();
        for b in half_bad.iter_mut().skip(64) {
            *b ^= 0xFF;
        }
        write_game_file(&dir, &fx.files[0].rel, &half_bad);

        let upper = if upper_bound {
            // The unscanned plan is read-only: print_plan returns right after
            // publishing its document, so the install is still dirty for the
            // scanned repair below.
            let up = dir.join("unscanned.json");
            let (n, _, c) = run_plan(&dir, &mock, false, true, None, None, &up).await;
            assert_eq!(c, RunExit::Success, "{name}");
            n
        } else {
            0
        };

        // One scanned run per mode writes its plan document before execution,
        // so the estimate and the SUMMARY's real download are from the same
        // run. The document's per-file halves are the PLAN total's split.
        let hits_before = mock.chunk_hits().len();
        let out = dir.join("plan.json");
        let (estimate, summary, code) = run_plan(&dir, &mock, true, false, temp, None, &out).await;
        assert_eq!(code, RunExit::Success, "{name}");
        let downloaded = summary.download.settled();
        assert_eq!(
            estimate, downloaded,
            "{name}: the scanned estimate is the real cost: {estimate} vs {downloaded}"
        );
        assert!(
            estimate > 0,
            "{name}: a bad chunk must be priced, not reused"
        );
        if upper_bound {
            assert!(
                estimate < upper,
                "{name}: the scan knows the intact chunk is kept: {estimate} vs {upper}"
            );
            assert_eq!(
                summary.flags.damaged_chunks,
                Some(1),
                "{name}: one chunk was found wrong"
            );
        }
        if temp.is_some() {
            assert_eq!(
                &mock.chunk_hits()[hits_before..],
                &["chunk_foo1".to_string()],
                "{name}: the corrupt candidate is fetched, the intact one is reused"
            );
        }
    }

    // One process-level run stays for the PLAN line: its conditional
    // scan_bytes key and its est_download_bytes total are rendered by the
    // binary's record writer, with no in-process seam. Reusing this same run
    // to pin the plan-out document's sum against the line adds no spawn. The
    // shared mock serves it, so no extra server is started either.
    let dir = temp_dir("scan_exact_plan_line");
    let fx = build_fixture();
    write_game_file(&dir, &fx.files[1].rel, &fx.files[1].data);
    let mut half_bad = fx.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx.files[0].rel, &half_bad);
    let game = dir.to_string_lossy().into_owned();
    let plan_out = dir.join("plan.json");
    let plan_out_arg = plan_out.to_string_lossy().into_owned();
    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game,
            "--biz",
            "hk4e_global",
            "--jobs",
            "1",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
            "--scan",
            "--print-plan",
            "--plan-out",
            &plan_out_arg,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "{stderr}");
    let plan_line = stdout
        .lines()
        .find(|l| l.starts_with("PLAN "))
        .expect("the PLAN line");
    assert!(
        plan_line.contains(&format!("{PLAN_SCAN_BYTES_KEY}={}", 128 + 48)),
        "the scan reports the bytes it read: {plan_line}"
    );
    assert_eq!(
        plan_out_estimate(&plan_out),
        field(plan_line, "est_download_bytes"),
        "the document's per-file halves sum to the scanned PLAN total: {plan_line}"
    );

    mock.stop();
}

#[tokio::test]
async fn check_only_trips_a_ceiling_not_damage() {
    // A ceiling narrows *what is verified*; `CheckFailed` must not be masked by
    // narrowing — but a ceiling is a different fact entirely.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_check");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, b"damaged");
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.check_only = true;
    ctx.limits.files = Some(1);
    let (summary, code) = run(ctx).await.expect("refusal is not an error");
    assert_eq!(
        code,
        RunExit::Limit,
        "the ceiling is asked before the verification"
    );
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "nothing verified"
    );

    // Without the ceiling, the same run reports damage.
    let mut ctx2 = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx2.check_only = true;
    let (_, code2) = run(ctx2).await.expect("check runs");
    assert_eq!(code2, RunExit::CheckFailed);

    mock.stop();
}

#[tokio::test]
async fn check_only_and_print_plan_issue_no_deprecated_file_call() {
    // The deprecated list is fetched during the plan phase, and these modes
    // never reach the post-phase, so they skip that network call:
    // `--check-only` and `--print-plan` must issue zero
    // `getGameDeprecatedFileConfigs` requests. (`--dry-run` does fetch it,
    // because it lists the deprecated deletes — but `--print-plan` returns
    // before the execution phase and is not widened by the combination.)
    for mode in ["check", "plan", "plan-dry"] {
        let fx = build_fixture();
        let mock = MockServer::start(
            fx,
            MockOpts {
                deprecated_files: vec!["old/legacy.dll".to_string()],
                ..Default::default()
            },
        )
        .await;
        let dir = temp_dir("readonly_deprecated");
        let fx2 = build_fixture();
        for f in &fx2.files {
            write_game_file(&dir, &f.rel, &f.data);
        }
        write_game_file(&dir, "old/legacy.dll", &[0x7Eu8; 50]);

        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
        match mode {
            "check" => ctx.check_only = true,
            "plan-dry" => {
                ctx.print_plan = true;
                ctx.dry_run = true;
            }
            _ => ctx.print_plan = true,
        }
        run(ctx).await.expect("run");
        assert!(
            mock.first_target_with("getGameDeprecatedFileConfigs")
                .is_none(),
            "--{mode} must not fetch the deprecated list it would discard"
        );
        assert!(read_game_file(&dir, "old/legacy.dll").is_some());
        mock.stop();
    }
}

#[tokio::test]
async fn a_hostile_chunk_range_refuses_every_read_only_mode_before_it_runs() {
    // Option (b): the range gate is in `build_plan`, so `--check-only`,
    // `--dry-run` and `--print-plan` all refuse the manifest at exit `2`
    // instead of one of them reporting a false clean or printing an unrunnable
    // plan. Nothing is written in any mode. The "no PLAN / no CHECK" pins are
    // process-level — the binary is the only thing that renders those lines —
    // so they stay on one binary run. The other modes assert the same verdict
    // and immutability in-process.
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new(
            "game/foo.dat",
            vec![ChunkSpec::new("chunk_foo0", vec![0x41u8; 64], 0)],
        )])
        .with_lies(Lies {
            huge_offset: vec!["chunk_foo0".into()],
            ..Default::default()
        }),
    );
    // The file on disk is the one the manifest's size and MD5 describe, so
    // without the plan gate `--check-only` would call it intact and exit
    // `0` for the wrong reason. All three modes share one mock: the gate
    // fetches nothing, so `chunk_hits` stays empty across the loop.
    let expected = fx.files[0].data.clone();
    let mock = MockServer::start(fx, MockOpts::default()).await;

    for (mode, on_binary) in [
        ("--check-only", false),
        ("--dry-run", false),
        ("--print-plan", true),
    ] {
        let dir = temp_dir("readonly_hostile_range");
        let game = dir.join("install");
        write_game_file(&game, "game/foo.dat", &expected);

        let code = if on_binary {
            let game_arg = game.to_string_lossy().into_owned();
            let (code, stdout, stderr) = run_binary(
                &[
                    "--game-path",
                    &game_arg,
                    "--biz",
                    "hk4e_global",
                    "--jobs",
                    "1",
                    mode,
                    "--hyp-base",
                    &mock.hyp_base,
                    "--sophon-base",
                    &mock.sophon_base,
                ],
                &[],
            )
            .await;
            assert!(!stdout.contains("PLAN "), "{mode}: no plan: {stdout}");
            assert!(!stdout.contains("CHECK"), "{mode}: no CHECK line: {stdout}");
            assert_eq!(code, 2, "{mode}: a hostile range is metadata: {stderr}");
            code
        } else {
            let mut ctx = ctx_for(game.to_path_buf(), &mock, 1);
            match mode {
                "--check-only" => ctx.check_only = true,
                "--dry-run" => ctx.dry_run = true,
                _ => unreachable!("only the two in-process modes reach here"),
            }
            let code = match run(ctx).await {
                Ok((_, c)) => c.code(),
                Err(f) => f.exit_code.code(),
            };
            assert_eq!(code, 2, "{mode}: a hostile range is metadata");
            code
        };
        assert_eq!(code, 2, "{mode}: the gate refuses before any mode runs");
        assert_eq!(
            read_game_file(&game, "game/foo.dat").as_deref(),
            Some(expected.as_slice()),
            "{mode}: the read-only run must not touch the target"
        );
        assert!(
            !dir.join("config.ini").exists(),
            "{mode}: config.ini must not be written"
        );
        assert!(
            mock.chunk_hits().is_empty(),
            "{mode}: no chunk may be fetched"
        );
    }

    mock.stop();
}

#[tokio::test]
async fn a_legitimate_empty_file_passes_every_mode_untouched() {
    // A `size = 0` entry whose digest is `EMPTY_MD5` describes a real, empty
    // file. The fixture builder derives both from the (empty) layout, so this
    // is the honest shape. Every mode must accept it, and the repair must not
    // rewrite a file that is already correct.
    let fx = build_fixture_spec(&FixtureSpec::new(vec![FileSpec::new(
        "game/empty.dat",
        vec![],
    )]));
    assert_eq!(fx.files[0].data, Vec::<u8>::new());
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("empty_legit");
    let game = dir.join("install");
    // Present and empty, exactly as the manifest describes it.
    write_game_file(&game, "game/empty.dat", b"");

    // The three data-plane verdicts run in-process; only the PLAN line needs
    // the binary (a record rendered by the process, with no library seam).
    let mut ctx = ctx_for(game.to_path_buf(), &mock, 1);
    ctx.check_only = true;
    let (_, code) = run(ctx).await.expect("check-only runs");
    assert_eq!(code, RunExit::Success, "--check-only must call it clean");
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "the check must not touch the file"
    );

    let mut ctx = ctx_for(game.to_path_buf(), &mock, 1);
    ctx.dry_run = true;
    let (_, code) = run(ctx).await.expect("dry-run runs");
    assert_eq!(code, RunExit::Success, "--dry-run accepts the empty file");

    let game_arg = game.to_string_lossy().into_owned();
    let (code, stdout, stderr) = run_binary(
        &[
            "--game-path",
            &game_arg,
            "--biz",
            "hk4e_global",
            "--jobs",
            "1",
            "--hyp-base",
            &mock.hyp_base,
            "--sophon-base",
            &mock.sophon_base,
            "--print-plan",
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "--print-plan: {stderr}");
    assert!(
        stdout.lines().any(|l| l.starts_with("PLAN ")),
        "an empty file is still a planned file:\n{stdout}"
    );

    let ctx = ctx_for(game.to_path_buf(), &mock, 1);
    let (_, code) = run(ctx).await.expect("repair runs");
    assert_eq!(
        code,
        RunExit::Success,
        "the repair must accept the correct empty file"
    );
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "the repair must leave the empty file empty"
    );
    assert!(!game.join("game").join("empty.dat_tmp").exists());

    mock.stop();
}

#[tokio::test]
async fn an_empty_file_with_a_non_empty_digest_is_refused_by_the_md5_gate() {
    // A zero-length entry whose declared digest is not `EMPTY_MD5` cannot
    // describe any byte string, so the size gate passes and the digest gate
    // is the one that has to refuse. The read-only modes disagree on purpose:
    // `--check-only` reports the file bad (exit `4`), `--dry-run` changes
    // nothing (exit `0`), `--print-plan` still prints the plan (exit `0`); a
    // real repair fails the file (exit `3`) without publishing a wrong one.
    let fx = build_fixture_spec(
        &FixtureSpec::new(vec![FileSpec::new("game/empty.dat", vec![])]).with_lies(Lies {
            wrong_file_md5: vec!["game/empty.dat".into()],
            ..Default::default()
        }),
    );
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("empty_bad_md5");
    let game = dir.join("install");
    write_game_file(&game, "game/empty.dat", b"");

    // The three read-only verdicts run in-process. For the plan case the
    // binary's "PLAN printed" line is process-rendered, so its data-level
    // counterpart is the plan document the library still publishes even with
    // a wrong digest.
    let mut ctx = ctx_for(game.to_path_buf(), &mock, 1);
    ctx.check_only = true;
    let (_, code) = run(ctx).await.expect("check-only runs");
    assert_eq!(
        code,
        RunExit::CheckFailed,
        "--check-only must report the mismatch"
    );
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "the check must not touch the file"
    );

    let mut ctx = ctx_for(game.to_path_buf(), &mock, 1);
    ctx.dry_run = true;
    let (_, code) = run(ctx).await.expect("dry-run runs");
    assert_eq!(code, RunExit::Success, "--dry-run");
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "--dry-run must not touch the file"
    );

    let plan = dir.join("plan.json");
    let mut ctx = ctx_for(game.to_path_buf(), &mock, 1);
    ctx.print_plan = true;
    ctx.plan_out = Some(plan.clone());
    let (_, code) = run(ctx).await.expect("print-plan runs");
    assert_eq!(code, RunExit::Success, "--print-plan");
    assert!(
        std::fs::read_to_string(&plan)
            .expect("plan-out file")
            .contains("\"repair\""),
        "the plan is still published even though the digest is wrong"
    );

    // The terminal repair stays on the binary: its stderr carries the retry
    // wording ("attempt failed", the whole-file digest gate) that only the
    // process boundary renders, and the stdout summary pins stay with it.
    let game_arg = game.to_string_lossy().into_owned();
    let base = [
        "--game-path",
        &game_arg,
        "--biz",
        "hk4e_global",
        "--jobs",
        "1",
        "--hyp-base",
        &mock.hyp_base,
        "--sophon-base",
        &mock.sophon_base,
    ];
    // The digest mismatch is deterministic: the file has no chunks, so the
    // next attempt would assemble the same empty bytes and fail the same gate.
    // It is therefore terminal, not retried — exactly one attempt is made even
    // without `--fail-fast` — and the failure names the whole-file digest gate
    // rather than some other write error.
    let (code, stdout, stderr) = run_binary(&base, &[]).await;
    assert_eq!(
        code, 3,
        "the repair must fail the one file it cannot make true: {stderr}"
    );
    assert!(
        stderr.contains("final md5 mismatch for game/empty.dat"),
        "the repair must fail on the whole-file digest gate: {stderr}"
    );
    assert!(
        stderr.contains("(the bytes outside the chunks are wrong)"),
        "the failure must name the part the plan cannot write: {stderr}"
    );
    assert_eq!(
        stderr.matches("attempt failed").count(),
        1,
        "a deterministic digest mismatch must not be attempted again: {stderr}"
    );
    assert!(
        !stderr.contains("after 5 attempts"),
        "the terminal failure must not be reported as exhausted retries: {stderr}"
    );
    assert!(
        stdout.contains("failed=1") && stdout.contains("exit=3"),
        "the summary must carry the failed file and the write exit: {stdout}"
    );
    assert_eq!(
        read_game_file(&game, "game/empty.dat"),
        Some(Vec::new()),
        "no wrong bytes may be published in its place"
    );
    assert!(
        !game.join("game").join("empty.dat_tmp").exists(),
        "a failed empty-file repair must leave no artefact"
    );

    mock.stop();
}

#[tokio::test]
async fn a_manifest_that_does_not_match_its_checksum_is_refused() {
    // The checksum is the MD5 of the **decompressed** bytes, and it is
    // verified before the protobuf parse. A manifest that does not match it
    // cannot be planned from: the run fails as metadata, before a single byte
    // is written.
    //
    // The blob the mock serves is unchanged — only the checksum the API
    // advertises is wrong — so the checksum gate is the only thing that can
    // catch it, and the decompressed bytes are the only thing it can be
    // compared against.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            wrong_manifest_checksum: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("manifest_checksum");
    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("a manifest that does not match its checksum cannot be planned from");
    assert_eq!(
        err.exit_code,
        RunExit::Metadata,
        "metadata, not a write failure"
    );
    assert!(!dir.join("config.ini").exists(), "nothing was written");
    mock.stop();
}

#[tokio::test]
async fn an_overlapping_layout_reaches_the_manifest_digest_after_the_redo() {
    // Two chunks share bytes 2..4. The manifest lays them out in order, so the
    // file it describes is A's head followed by B (`B` wins the overlap). On
    // disk, B is already correct where it belongs and only A's head is wrong:
    // the whole-file digest therefore fails and the patch writes A — which
    // clobbers part of B. The verify digest is what notices, its scan names B,
    // and the redo downloads B so the file ends at the manifest's bytes.
    //
    // The test is deterministic: no timing and no failure injection. It pins
    // the end-to-end path the overlap guard exists for — a retry that has to
    // end at the manifest digest, not at the patch's last write.
    let a = vec![0x41u8, 0x42, 0x43, 0x44];
    let b = vec![0x51u8, 0x52, 0x53, 0x54];
    let spec = FixtureSpec::new(vec![FileSpec::new(
        "game/overlap.dat",
        vec![
            ChunkSpec::new("chunk_overlap_a", a.clone(), 0),
            ChunkSpec::new("chunk_overlap_b", b.clone(), 2),
        ],
    )]);
    let fx = build_fixture_spec(&spec);
    let manifest = fx.files[0].data.clone();
    // A's head is wrong; B's range [2,6) already holds B.
    let on_disk = vec![0x00u8, 0x00, b[0], b[1], b[2], b[3]];
    assert_ne!(on_disk, manifest, "the file starts damaged");
    assert_eq!(
        &on_disk[2..],
        b.as_slice(),
        "B is already correct where it belongs"
    );

    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("overlap_redo_digest");
    write_game_file(&dir, "game/overlap.dat", &on_disk);

    let (result, logs) = logs_of(run(ctx_for(dir.to_path_buf(), &mock, 1))).await;
    let (summary, code) = result.expect("the overlap repairs");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        summary.files_repaired.load(Ordering::Relaxed),
        1,
        "the file is repaired, not skipped"
    );
    assert_eq!(
        read_game_file(&dir, "game/overlap.dat").as_deref(),
        Some(manifest.as_slice()),
        "the file ends at the manifest's bytes, not the patch's last write"
    );
    assert!(
        logs.contains("final md5 mismatch -> redoing the mismatching chunks"),
        "the whole-file digest gate is what drove the redo: {logs}"
    );
    assert!(
        !logs.contains("attempt failed"),
        "the redo must settle the file inside the first attempt, not through a \
         file-level retry: {logs}"
    );
    assert_eq!(
        mock.chunk_hits().into_iter().collect::<BTreeSet<_>>(),
        ["chunk_overlap_a", "chunk_overlap_b"]
            .into_iter()
            .map(String::from)
            .collect::<BTreeSet<_>>(),
        "A is patched, and B is fetched again only because the digest failed"
    );

    mock.stop();
}
