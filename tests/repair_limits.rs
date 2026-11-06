//! Resource-ceiling and fail-fast cases: refusing or stopping a run at a
//! declared limit.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    ChunkSpec, FileSpec, FixtureSpec, MockOpts, MockServer, absent_game_dir, build_fixture,
    build_fixture_spec, ctx_for, logs_of, read_game_file, scope, temp_dir, write_game_file,
};
use girpr::test_support::summary_record;
use girpr::{OutputFormat, RunExit, run};
use std::sync::atomic::Ordering;

/// How many times `stream_chunk_to` asks a *retryable* origin — the linear
/// 1+2+3+4 s backoff, five attempts. Kept next to the test that depends on it so
/// the two cannot drift.
const ATTEMPTS: usize = 5;

// ---------------------------------------------------------------------------
// resource ceilings
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_files_refuses_before_touching_anything() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let parent = temp_dir("max_files");
    let dir = parent.join("game");
    let fx2 = build_fixture();
    // bar corrupt (would be repaired), foo missing (would be downloaded).
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.limits.files = Some(1); // the plan is 2 files
    let (summary, code) = run(ctx).await.expect("a refusal is not an error");
    assert_eq!(code, RunExit::Limit, "a tripped ceiling is a limit");
    assert!(summary.flags.limited);
    assert_eq!(summary.files_total, 2, "plan_files is still reported");
    assert_eq!(
        summary.files_repaired.load(Ordering::Relaxed),
        0,
        "nothing was repaired"
    );
    assert_eq!(summary.deleted_extra_bytes.load(Ordering::Relaxed), 0);
    assert!(
        mock.chunk_hits().is_empty(),
        "not one chunk may be fetched on a refusal"
    );
    assert_eq!(
        read_game_file(&dir, &fx2.files[1].rel).unwrap(),
        b"not the game data",
        "the damaged file is untouched"
    );
    assert!(read_game_file(&dir, &fx2.files[0].rel).is_none());

    // The same refusal against a game dir that does not exist yet: not even
    // the directory may be created, because the ceiling asserts before
    // `create_dir_all`.
    let missing = absent_game_dir(&parent);
    let mut ctx = ctx_for(missing.clone(), &mock, 1);
    ctx.limits.files = Some(1);
    let (_, code) = run(ctx).await.expect("a refusal is not an error");
    assert_eq!(code, RunExit::Limit);
    assert!(!missing.exists(), "a refusal must not even create game_dir");

    mock.stop();
}

#[tokio::test]
async fn max_files_at_the_boundary_does_not_refuse() {
    // "> N", not ">= N": a run that exactly fits must be allowed through.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_edge");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.limits.files = Some(2); // == plan.files.len()
    let (summary, code) = run(ctx).await.expect("the boundary must pass");
    assert_eq!(code, RunExit::Success);
    assert!(
        summary.flags.limited,
        "the flag was passed, so it is reported"
    );
    assert_eq!(summary.files_skipped.load(Ordering::Relaxed), 2);

    mock.stop();
}

#[tokio::test]
async fn max_files_zero_is_a_ceiling_not_a_usage_error() {
    // `0` is a perfectly meaningful ceiling: it is not an error on an empty
    // plan and it is on a 3000-file one. Only an unparseable value is
    // exit `1`, and that is clap's job.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_zero");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.limits.files = Some(0);
    let (_, code) = run(ctx).await.expect("0 is a ceiling");
    assert_eq!(code, RunExit::Limit, "0 files allowed, 2 planned -> refuse");

    mock.stop();
}

#[tokio::test]
async fn max_files_is_checked_after_the_scope_filter() {
    // The pairing that gives the flag its purpose: `--include` selects,
    // `--max-files` verifies the selection.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_scoped");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    ctx.limits.files = Some(1); // the *filtered* plan is 1 file
    let (summary, code) = run(ctx).await.expect("scoped boundary passes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.files_total, 1);
    assert!(summary.flags.scoped && summary.flags.limited);

    // Same scope, ceiling of 0: now it refuses.
    let dir2 = temp_dir("max_files_scoped_refuse");
    let mut ctx2 = ctx_for(dir2.to_path_buf(), &mock, 1);
    ctx2.path_scope = scope(&["game/foo.dat"], &[]);
    ctx2.limits.files = Some(0);
    let (_, code2) = run(ctx2).await.expect("refusal is not an error");
    assert_eq!(code2, RunExit::Limit);

    mock.stop();
}

#[tokio::test]
async fn max_delete_bytes_refuses_and_deletes_nothing() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_delete");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    // Two orphans totalling 170 bytes, which no purge flag would touch yet.
    write_game_file(&dir, "stray/orphan.dat", &[0x5Au8; 100]);
    write_game_file(&dir, "other/stale.dat", &[0x5Bu8; 70]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_after = true;
    ctx.limits.delete_bytes = Some(100);
    let (summary, code) = run(ctx).await.expect("refusal is not an error");
    assert_eq!(code, RunExit::Limit, "170 > 100");
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "a refused purge deletes nothing"
    );
    assert!(read_game_file(&dir, "stray/orphan.dat").is_some(), "kept");
    assert!(read_game_file(&dir, "other/stale.dat").is_some(), "kept");

    // Exactly at the ceiling: the purge runs.
    let mut ctx2 = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx2.purge_after = true;
    ctx2.limits.delete_bytes = Some(170);
    let (summary2, code2) = run(ctx2).await.expect("the boundary passes");
    assert_eq!(code2, RunExit::Success);
    assert_eq!(summary2.deleted_extra_bytes.load(Ordering::Relaxed), 170);

    mock.stop();
}

#[tokio::test]
async fn a_dry_run_post_delete_ceiling_stops_with_the_limit_class() {
    // The up-front purge plan is built before the plan document is written, so
    // the up-front delete ceiling sees no candidates. Placing `--plan-out`
    // inside the install makes it a candidate that appears between that plan
    // and the post-repair re-plan: this is the deterministic way to reach the
    // re-check. A dry run must stop on it like a real run, name the flag in
    // `SUMMARY`, and list nothing.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("dry_post_delete_ceiling");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.purge_after = true;
    ctx.dry_run = true;
    ctx.limits.delete_bytes = Some(1);
    ctx.plan_out = Some(dir.join("plan.json"));
    let (result, logs) = logs_of(run(ctx)).await;
    let (summary, code) = result.expect("a stop is not an Err");

    assert_eq!(code, RunExit::Limit, "the post-repair re-check is a limit");
    assert_eq!(
        summary.flags.stopped,
        Some("--max-delete-bytes"),
        "the stop names the flag that bound"
    );
    assert_eq!(
        summary.deleted_extra_bytes.load(Ordering::Relaxed),
        0,
        "a dry run frees nothing"
    );
    assert!(
        !logs.contains("would purge extra"),
        "the pass stopped before listing its candidates: {logs}"
    );

    // The two stages are distinguishable without a test-only hook:
    //
    // - the plan document exists, and the up-front ceiling is asserted before
    //   that document is written. Its presence therefore says the up-front
    //   check passed rather than refused;
    // - the document's own `purge` array is the up-front plan, and it is empty,
    //   so the up-front plan had no delete candidate at all;
    // - the log's `purge_bytes` is exactly the document's size, the only path
    //   this dry run added. The up-front total was 0, so that figure can only
    //   have been measured by the post-repair re-plan.
    let plan_doc = read_game_file(&dir, "plan.json")
        .expect("the up-front check passed, so --plan-out was written");
    let doc: serde_json::Value =
        serde_json::from_slice(&plan_doc).expect("--plan-out writes valid JSON");
    assert_eq!(
        doc["purge"].as_array().map(Vec::len),
        Some(0),
        "the up-front plan had no delete candidates: {doc}"
    );
    assert!(
        logs.contains(&format!("purge_bytes={} limit=1", plan_doc.len())),
        "only the post-repair re-plan could measure the plan document: {logs}"
    );

    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("exit=5"), "{line}");
    assert!(line.contains("limited=1"), "{line}");
    assert!(line.contains("stopped=--max-delete-bytes"), "{line}");

    // Nothing was removed: not the candidate, not the files it was measured
    // against.
    assert!(
        read_game_file(&dir, "plan.json").is_some(),
        "a stopped dry run deletes nothing"
    );
    for f in &fx2.files {
        assert_eq!(read_game_file(&dir, &f.rel).unwrap(), f.data);
    }

    mock.stop();
}

#[tokio::test]
async fn an_out_of_scope_path_never_counts_toward_the_delete_ceiling() {
    // The scope and the ceiling are complementary, not redundant: the glob
    // narrows what is *counted*, the ceiling bounds what is *done*.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_delete_scoped");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    // A 1 GB orphan that is deliberately out of scope.
    write_game_file(&dir, "huge/orphan.dat", &[0x5Au8; 4096]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.path_scope = scope(&["game/foo.dat", "game/bar.dat"], &[]);
    ctx.purge_after = true;
    ctx.limits.delete_bytes = Some(1); // would be blown by the orphan
    let (_, code) = run(ctx).await.expect("scoped run");
    assert_eq!(
        code,
        RunExit::Success,
        "the out-of-scope orphan is not a candidate at all"
    );
    assert!(read_game_file(&dir, "huge/orphan.dat").is_some(), "kept");

    mock.stop();
}

#[tokio::test]
async fn a_tripped_ceiling_still_emits_a_summary_tail() {
    // A refusal is *returned*, not raised, so every exit path ends with
    // one uniform SUMMARY.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_files_summary");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.limits.files = Some(1);
    let (summary, code) = run(ctx).await.expect("refusal is not an error");
    let line = summary_record(&summary, code.code()).text();
    assert_eq!(
        line,
        "SUMMARY total=2 skipped=0 repaired=0 failed=0 download_bytes=0 deleted_extra_bytes=0 \
         exit=5 limited=1 plan_files=2"
    );
    assert!(!line.contains("scoped"), "no scope was passed: {line}");

    mock.stop();
}

#[tokio::test]
async fn max_download_bytes_refuses_on_the_estimate_and_passes_at_the_boundary() {
    // With no local build the reuse map is empty, so the estimate is every
    // chunk's compressed size — the same quantity the runner would download.
    let fx = build_fixture();
    let est = fx.chunk_store.values().map(|b| b.len() as u64).sum::<u64>();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_dl_refuse");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    // One byte under the estimate → refuse before fetching anything.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.limits.download_bytes = Some(est - 1);
    let (summary, code) = run(ctx).await.expect("a refusal is not an error");
    assert_eq!(code, RunExit::Limit);
    assert_eq!(
        summary.flags.plan_download_bytes,
        Some(est),
        "SUMMARY reports the estimate it refused on"
    );
    assert!(mock.chunk_hits().is_empty(), "nothing may be fetched");
    assert!(!dir.join("game").join("foo.dat_tmp").exists(), "no _tmp");

    // Exactly the estimate → the boundary passes and the whole thing runs.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 2);
    ctx.limits.download_bytes = Some(est);
    let (summary, code) = run(ctx).await.expect("the boundary passes");
    assert_eq!(code, RunExit::Success);
    assert_eq!(summary.download.settled(), est);
    let line = summary_record(&summary, code.code()).text();
    assert!(
        line.contains(&format!("plan_download_bytes={est}")),
        "{line}"
    );

    mock.stop();
}

#[tokio::test]
async fn a_partial_body_then_a_drop_is_charged_to_the_download_meter() {
    // Incompressible bytes: `compressed_size` is close to the payload's length,
    // so truncating at half of it is a meaningful partial body.
    let mut x = 0x1234_5678u32;
    let payload: Vec<u8> = (0..16 * 1024)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect();
    let spec = FixtureSpec::new(vec![FileSpec::new(
        "game/big.dat",
        vec![ChunkSpec::new("c0", payload, 0)],
    )]);
    let fx = build_fixture_spec(&spec);
    let compressed = fx.chunk_store["c0"].len() as u64;
    let cut = (compressed / 2) as usize;
    assert!(cut > 0, "the fixture's body is big enough to cut");
    let mock = MockServer::start(
        fx,
        MockOpts {
            truncate_first_chunks: 1,
            truncate_bytes: cut,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("partial_body_meter");

    // The estimate is every chunk's `compressed_size`, and the ceiling is
    // exactly that: the plan passes, and the retry's traffic is what crosses it.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.limits.download_bytes = Some(compressed);
    let (summary, code) = run(ctx).await.expect("a stop is not an error");
    assert_eq!(
        code,
        RunExit::Limit,
        "a settle that crosses the ceiling is the runtime hard stop"
    );
    assert_eq!(
        summary.flags.stopped,
        Some("--max-download-bytes"),
        "the stop names the flag that bound"
    );
    assert_eq!(
        summary.download.settled(),
        compressed,
        "the promoted file's bytes are the settled figure"
    );
    assert_eq!(
        summary.download.live(),
        compressed + cut as u64,
        "the partial body crossed the wire, so the live figure has it too"
    );
    assert!(
        summary.download.live() > compressed,
        "the ceiling was passed by the bytes that really arrived"
    );
    assert_eq!(
        mock.chunk_hits(),
        vec!["c0".to_string(), "c0".to_string()],
        "the dropped body was retried once"
    );
    assert_eq!(
        read_game_file(&dir, "game/big.dat").expect("the file was repaired"),
        spec.new[0].chunks[0].payload,
        "and the retry's body is what landed"
    );

    mock.stop();
}

#[tokio::test]
async fn a_broken_reuse_path_still_trips_the_runtime_stop() {
    // The case that justifies B2 having *two* mechanisms. `config.ini` already
    // at the latest tag → the local build loads → the reuse map covers both of
    // foo's chunks → the pre-flight estimate is 0 and the run is allowed
    // through. Then foo's bytes are corrupted in *both* halves, so every local
    // slice fails its MD5 and every chunk falls back to a download. The
    // estimate was wrong; the runtime reservation is what stops the run.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_dl_runtime");
    std::fs::write(
        dir.join("config.ini"),
        "[General]\ngame_version=9.9.9-test\n",
    )
    .unwrap();
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data); // bar intact
    let mut all_bad = fx2.files[0].data.clone();
    for b in all_bad.iter_mut() {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &all_bad);

    let limit = 1u64; // any positive value: the estimate is 0, so it passes
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.limits.download_bytes = Some(limit);
    let (summary, code) = run(ctx).await.expect("a stop is not an error");
    assert_eq!(
        code,
        RunExit::Limit,
        "the runtime hard stop is also a limit"
    );
    assert_eq!(
        summary.flags.stopped,
        Some("--max-download-bytes"),
        "the stop must be distinguishable from a refusal"
    );
    assert_eq!(
        summary.flags.plan_download_bytes,
        Some(0),
        "the estimate really was zero — that is the point"
    );
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "not a failure"
    );
    assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 0);
    assert_eq!(
        summary.files_skipped.load(Ordering::Relaxed),
        1,
        "bar was intact"
    );
    assert!(
        summary.download.settled() <= limit,
        "the ceiling must not be overshot"
    );
    // The stopped file keeps its `_tmp`, and the next run resumes it.
    assert_ne!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "a stopped file is not promoted"
    );

    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("stopped=--max-download-bytes"), "{line}");
    assert!(line.contains("limited=1"), "{line}");

    mock.stop();
}

#[tokio::test]
async fn the_summary_growth_keys_appear_only_when_their_flag_is_active() {
    // Every scope/ceiling key is emitted only when its flag is active, so
    // an unflagged run stays byte-identical to one that passes no flags.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("summary_absent");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, &f.data);
    }
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 2))
        .await
        .expect("plain run");
    assert_eq!(code, RunExit::Success);
    for format in [OutputFormat::Text, OutputFormat::Json] {
        let line = summary_record(&summary, code.code()).render(format);
        for k in [
            "limited",
            "scoped",
            "config_bumped",
            "plan_files",
            "plan_download_bytes",
            "download_bytes_spent",
            "write_bytes",
            "write_bytes_spent",
            "stopped",
            // A fresh install has no `config.ini`, so no local version is known
            // and the lookup is never attempted — the key must not appear.
            "local_manifest",
        ] {
            assert!(
                !line.contains(k),
                "{k} must be absent when unflagged: {line}"
            );
        }
    }
    mock.stop();
}

#[tokio::test]
async fn fail_fast_reaches_its_summary_in_seconds_against_a_500_origin() {
    // Without `--fail-fast` each failing file costs `MAX_ATTEMPTS` × (the
    // 5× retry with 1-5 s sleeps inside `stream_chunk_to` + 1-4 s of
    // file-level backoff) ≈ **125 s of pure sleeping**, measured — and that is
    // before `HypClient::new_with_bases` is even given a connect timeout. With
    // it, a file gets one attempt and the abort spares every file behind it.
    //
    // There is deliberately no "without the flag" control here: proving that
    // case means paying those 125 s, and it is already covered structurally —
    // `repairs_missing_and_corrupt_files_end_to_end` shows every planned file is
    // attempted when nothing is flagged.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            fail_chunks: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("fail_fast");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.fail_fast = true;
    let started = std::time::Instant::now();
    let (summary, code) = run(ctx).await.expect("a failed repair is not an Err");
    let elapsed = started.elapsed();
    assert_eq!(
        code,
        RunExit::Write,
        "a file failed, so the post-phase is skipped"
    );
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        1,
        "honest count"
    );
    assert_eq!(summary.files_repaired.load(Ordering::Relaxed), 0);
    assert!(!dir.join("config.ini").exists());
    // Every chunk request 500s, so nothing was ever delivered. A reservation
    // that the failed fetch never gave back would still be sitting in the
    // total — and with no ceiling set the sentinel is u64::MAX, so the leak is
    // not gated on `--max-download-bytes`.
    assert_eq!(
        summary.download.settled(),
        0,
        "a fetch that delivered nothing must count nothing"
    );
    // The second file was never even asked for. The plan is sorted, so with
    // `jobs=1` `bar.dat` runs first, exhausts its retries and trips the abort;
    // `foo.dat` then drops out at its task entry without opening a connection.
    let hits = mock.chunk_hits();
    assert!(
        hits.iter().all(|id| id == "chunk_bar0"),
        "foo.dat must not be attempted after the abort: {hits:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "fail-fast must reach its SUMMARY in seconds, took {elapsed:?}"
    );
    assert!(
        summary.files_skipped.load(Ordering::Relaxed)
            + summary.files_repaired.load(Ordering::Relaxed)
            + summary.files_failed.load(Ordering::Relaxed)
            < summary.files_total,
        "the run was truncated before the plan was finished"
    );

    mock.stop();
}

#[tokio::test]
async fn a_404_is_asked_once_where_a_500_is_asked_five_times() {
    // The two halves of the retry policy, from the origin's side of the wire.
    //
    // A 404 cannot be fixed by asking again: the chunk id came out of this
    // build's own `getBuild` body, and the live blob routes answer a missing
    // object with 404 + `application/xml`, never 200-with-a-code. So the
    // request is made once and the run moves on. A 500 is a server that may
    // come back, so it is retried on the linear backoff.
    //
    // `fail_fast` gives a file one attempt, so the two branches are directly
    // comparable: the 500 spends the chunk fetch's 5 retries on that one attempt,
    // and the 404 spends none of them.
    for (name, opts, want_hits, max_elapsed) in [
        (
            "404",
            MockOpts {
                not_found_chunks: true,
                ..Default::default()
            },
            1usize,
            std::time::Duration::from_secs(5),
        ),
        (
            "500",
            MockOpts {
                fail_chunks: true,
                ..Default::default()
            },
            ATTEMPTS,
            std::time::Duration::from_secs(30),
        ),
    ] {
        let fx = build_fixture();
        let mock = MockServer::start(fx, opts).await;
        let dir = temp_dir(&format!("retry_{name}"));
        let fx2 = build_fixture();
        write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
        // `--include` so exactly one file is in scope, and `fail_fast` so a
        // single file's retry budget is the whole number being measured rather
        // than being multiplied by the rest of the plan.
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        ctx.fail_fast = true;
        ctx.path_scope = scope(&[&fx2.files[0].rel], &[]);
        let started = std::time::Instant::now();
        let (summary, code) = run(ctx).await.expect("not an Err");
        let elapsed = started.elapsed();

        assert_eq!(
            code,
            RunExit::Write,
            "{name}: a file failed, so the post-phase is skipped"
        );
        assert_eq!(summary.files_failed.load(Ordering::Relaxed), 1, "{name}");
        let hits = mock.chunk_hits();
        assert_eq!(
            hits.len(),
            want_hits,
            "{name}: the origin was asked {want_hits} time(s): {hits:?}"
        );
        assert!(hits.iter().all(|id| id == "chunk_foo0"), "{name}: {hits:?}");
        assert!(
            elapsed < max_elapsed,
            "{name} must not spend the budget in sleep, took {elapsed:?}"
        );
        mock.stop();
    }
}

#[tokio::test]
async fn a_404_manifest_fails_the_run_without_waiting_for_it() {
    // The manifest half. Without a terminal status this costs ~15 s of backoff
    // to learn that the build's manifests are gone, and then exits `2` anyway —
    // metadata, not a repaired file, so the exit code is unchanged.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            not_found_manifests: true,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("manifest_404");
    let started = std::time::Instant::now();
    let err = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect_err("no manifests means the run cannot plan");
    let elapsed = started.elapsed();
    assert_eq!(
        err.exit_code,
        RunExit::Metadata,
        "metadata, not a write failure"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "a 404 must not be retried at all, took {elapsed:?}"
    );
    // Asked once. The manifest route has no per-chunk log, so this reads the
    // server's own request log.
    let manifests = mock
        .request_targets()
        .into_iter()
        .filter(|t| t.contains("/manifests/"))
        .count();
    assert_eq!(manifests, 1, "the manifest was asked once");

    mock.stop();
}

#[tokio::test]
async fn a_run_that_stops_before_acting_creates_no_game_dir() {
    // `create_dir_all` lives in the execution phase, not at the top of
    // the run, so a run that never reaches it (here: the non-chunk
    // download-mode refusal) leaves nothing on disk.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            download_mode: Some("DOWNLOAD_MODE_FILE".to_string()),
            ..Default::default()
        },
    )
    .await;
    let parent = temp_dir("nodir");
    let missing = parent.join("never-created");
    let err = run(ctx_for(missing.clone(), &mock, 1))
        .await
        .expect_err("non-chunk mode must fail");
    assert_eq!(err.exit_code, RunExit::Usage);
    assert!(
        !missing.exists(),
        "a run that refuses must not create game_dir"
    );

    mock.stop();
}
