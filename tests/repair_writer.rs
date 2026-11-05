//! Writer-path cases: in-place, temp and discard repairs, and the write
//! accounting that accompanies them.

// Tests assert with `unwrap`/`expect`; the lint is for code a user runs.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{
    LATEST_TAG, Lies, MockOpts, MockServer, build_fixture, build_fixture_with_lies,
    build_moved_fixture, ctx_for, field, load_counters, read_game_file, scope, temp_dir,
    write_game_file,
};
use girpr::test_support::{RepairMode, summary_record};
use girpr::{RunExit, run};
use std::sync::atomic::Ordering;

// ---------------------------------------------------------------------------
// --max-write-bytes, --fail-fast, --purge-only
// ---------------------------------------------------------------------------

#[tokio::test]
async fn verified_local_slice_is_reused_without_redownload() {
    // S2: same-file reuse. config.ini already at latest so the local build
    // loads; foo keeps chunk0 intact and corrupts chunk1 -> only chunk_foo1
    // may be fetched.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("reuse");
    let fx2 = build_fixture();
    std::fs::write(
        dir.join("config.ini"),
        "[General]\ngame_version=9.9.9-test\n",
    )
    .unwrap();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("reuse run");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );
    assert_eq!(mock.chunk_hits(), vec!["chunk_foo1".to_string()]);

    mock.stop();
}

#[tokio::test]
async fn partial_tmp_prefix_is_resumed() {
    // Resume-by-length: the **temp** path's resume mechanism, so
    // it is asked for explicitly: `--repair-mode temp` is the mode that
    // writes `{path}_tmp` at all. foo missing but foo_tmp holds the first
    // chunk; only the second chunk is downloaded, then tmp is promoted.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("resume");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir, "game/foo.dat_tmp", &fx2.files[0].data[..64]);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.repair_mode = RepairMode::Temp;
    ctx.repair_mode_explicit = true;
    let (summary, code) = run(ctx).await.expect("resume run");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    // The prefix was kept, not rewritten: the temp path resumes by length, so
    // the first chunk's 64 bytes count as bytes the repair did not have to
    // touch — the same definition `kept_bytes` uses for the in-place path.
    assert_eq!(
        summary.kept_bytes.load(Ordering::Relaxed),
        64,
        "the resumed `_tmp` prefix is kept, not rewritten"
    );
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );
    assert!(
        !dir.join("game").join("foo.dat_tmp").exists(),
        "tmp promoted"
    );
    assert_eq!(mock.chunk_hits(), vec!["chunk_foo1".to_string()]);

    mock.stop();
}

#[tokio::test]
async fn a_full_length_corrupt_tmp_is_deleted_and_rebuilt() {
    // Resume-by-length keeps a prefix it has not verified, so a `_tmp` that is
    // already the right length can be entirely wrong. The whole-file MD5 gate
    // is what refuses it: the promote deletes the `_tmp`, the file attempt is
    // retried, and the second attempt downloads the file from scratch instead
    // of promoting the corrupt bytes.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("corrupt_tmp_full");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let bad = vec![0xEEu8; fx2.files[0].data.len()];
    write_game_file(&dir, "game/foo.dat_tmp", &bad);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.repair_mode = RepairMode::Temp;
    ctx.repair_mode_explicit = true;
    let (summary, code) = run(ctx).await.expect("the retry repairs the file");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "the corrupt attempt was retried, not counted as a failure"
    );
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "the rebuilt file is the manifest's"
    );
    assert!(
        !dir.join("game").join("foo.dat_tmp").exists(),
        "the `_tmp` was deleted by the failed gate and rebuilt"
    );
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_foo0".to_string(), "chunk_foo1".to_string()],
        "the full-length corrupt `_tmp` was not reused"
    );

    mock.stop();
}

#[tokio::test]
async fn the_in_place_repair_writes_only_the_wrong_chunk_and_no_tmp() {
    // The disk claim, on the fixture: a right-sized file whose second chunk
    // is corrupt is repaired by writing that chunk and nothing else. The
    // write counter is the evidence — the temp path would have written the
    // whole 128 bytes.
    //
    // There is no `config.ini` here, so there is no local manifest and no
    // reuse map: the in-place path finds the intact chunk on its own, from
    // the scan. That is the network win that needs no manifest at all.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("inplace_patch");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("in-place run");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "the file is correct"
    );
    // The disk claim: one chunk written, not the whole file.
    assert_eq!(
        summary.write.settled(),
        64,
        "only the corrupt chunk was written"
    );
    // And no `_tmp` anywhere: the in-place path never creates one.
    assert!(
        !dir.join("game").join("foo.dat_tmp").exists(),
        "the in-place path must not create a temp file"
    );
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_foo1".to_string()],
        "the intact chunk is kept, not fetched"
    );

    mock.stop();
}

#[tokio::test]
async fn a_file_that_grows_writes_only_the_growth() {
    // foo.dat is 128 bytes in the manifest; on disk it is 64. The first
    // chunk is already right, the second is missing: the repair writes 64
    // bytes — the growth — and not the 128 the temp path would have.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("inplace_grow");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    write_game_file(&dir, &fx2.files[0].rel, &fx2.files[0].data[..64]);

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("in-place run");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "the file grew to its manifest size"
    );
    assert_eq!(
        summary.write.settled(),
        64,
        "only the missing chunk was written"
    );
    assert!(!dir.join("game").join("foo.dat_tmp").exists());

    mock.stop();
}

#[tokio::test]
async fn the_discard_mode_deletes_the_target_and_writes_it_from_scratch() {
    // `discard` trades the network for the disk: the target is deleted and
    // written again, so nothing reusable survives — not even the chunk that
    // was already correct where it belonged.
    //
    // The local manifest is loaded here (config.ini at the latest tag), so
    // the reuse map exists and the *default* run would keep the intact
    // chunk. That is the difference the mode makes visible.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("discard");
    std::fs::write(
        dir.join("config.ini"),
        format!("[General]\ngame_version={}\n", LATEST_TAG),
    )
    .unwrap();
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.repair_mode = RepairMode::Discard;
    let (summary, code) = run(ctx).await.expect("discard run");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_foo0".to_string(), "chunk_foo1".to_string()],
        "after the delete there is nothing left to reuse, so both chunks are fetched"
    );
    assert!(!dir.join("game").join("foo.dat_tmp").exists());

    mock.stop();
}

#[tokio::test]
async fn a_read_only_target_is_made_writable_and_repaired_in_place() {
    // A game file can carry the read-only attribute, and then it cannot be
    // opened for writing. The repair clears the attribute and carries on —
    // the temp path would not have helped, because the promote renames over
    // the target and that fails the same way.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("readonly");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);
    let target = dir.join("game").join("foo.dat");
    let mut perms = std::fs::metadata(&target).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&target, perms).unwrap();

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("read-only run");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );
    assert!(
        !dir.join("game").join("foo.dat_tmp").exists(),
        "the in-place path repaired it without a temp file"
    );

    mock.stop();
}

#[tokio::test]
async fn the_ram_tier_keeps_every_source_when_the_content_moved() {
    // The fixture's old file is `P || Q || R` and the new one is `R || P ||
    // T`: the content moved, so the repair has to move bytes *inside* the
    // file. The sources are in both directions (R's is later, P's is
    // earlier), which is the case the two tiers differ on.
    //
    // The RAM tier stages the moving bytes, so **every** source survives:
    // only the brand-new chunk is downloaded.
    let fx = build_moved_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("moved_staged");
    let (old_rel, old_bytes) = build_moved_fixture().old_data.expect("the old file");
    std::fs::write(
        dir.join("config.ini"),
        "[General]\ngame_version=9.9.8-test\n",
    )
    .unwrap();
    write_game_file(&dir, &old_rel, &old_bytes);

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("moved run");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_t".to_string()],
        "both moved chunks are salvaged; only the new one is downloaded"
    );
    // The file is the new one.
    let want = build_moved_fixture().files[0].data.clone();
    assert_eq!(read_game_file(&dir, &old_rel).unwrap(), want);
    // And nothing was staged for nothing: the payload was the two moved
    // chunks, and the peak says so.
    assert!(
        summary.ram_peak_bytes() > 0,
        "the staged payload is counted"
    );
    assert!(!dir.join("game").join("moved.dat_tmp").exists());

    mock.stop();
}

#[tokio::test]
async fn the_ordered_tier_keeps_one_direction_and_produces_the_same_file() {
    // The same file, repaired by the ordered tier: no staging, so the
    // sources are read from the file at write time and only the ones that
    // survive the write order can be used. One direction's worth is kept —
    // here the tie goes to ascending, which keeps `R` and loses `P`.
    //
    // The file that comes out is **the same file**: the tier is a choice of
    // how to get there, not of what to produce.
    let fx = build_moved_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("moved_ordered");
    let (old_rel, old_bytes) = build_moved_fixture().old_data.expect("the old file");
    std::fs::write(
        dir.join("config.ini"),
        "[General]\ngame_version=9.9.8-test\n",
    )
    .unwrap();
    write_game_file(&dir, &old_rel, &old_bytes);

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    // "stage nothing at all" is what selects the ordered tier.
    ctx.limits.ram_bytes = Some(0);
    let (summary, code) = run(ctx).await.expect("moved run");
    assert_eq!(code, RunExit::Success);
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_p".to_string(), "chunk_t".to_string()],
        "the source that the other direction needed was lost, so it downloaded"
    );
    // The same file the RAM tier produces.
    let want = build_moved_fixture().files[0].data.clone();
    assert_eq!(read_game_file(&dir, &old_rel).unwrap(), want);
    assert!(!dir.join("game").join("moved.dat_tmp").exists());

    mock.stop();
}

#[tokio::test]
async fn an_interrupted_in_place_run_resumes_by_verification() {
    // The state an interrupted in-place run leaves behind: some chunks
    // correct where they belong, some not. There is no artefact to resume
    // from — the next run finds the correct chunks in its scan and rewrites
    // the rest, and a `_tmp` is nowhere in sight.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("inplace_resume");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    // Half of the second chunk is wrong: the file is neither the old one
    // nor the new one, which is exactly what an interrupted write leaves.
    let mut interrupted = fx2.files[0].data.clone();
    for b in interrupted.iter_mut().skip(96) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &interrupted);

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("in-place run");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data
    );
    assert_eq!(
        summary.write.settled(),
        64,
        "the chunk that was already right was not rewritten"
    );
    assert!(!dir.join("game").join("foo.dat_tmp").exists());
    assert!(
        !dir.join("game").join("foo.dat_tmp").exists(),
        "no artefact: resume is by verification"
    );

    mock.stop();
}

#[tokio::test]
async fn max_write_bytes_stops_mid_flight_and_leaves_the_work_for_the_next_run() {
    // The only ceiling that cannot be a pre-flight refusal: reuse success is
    // decided per chunk at runtime, against bytes that may not be there. This
    // run downloads both files, so almost every byte written into the install
    // is a write the plan's estimate never counted.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("max_write");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let limit = 1u64;
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.limits.write_bytes = Some(limit);
    let (summary, code) = run(ctx).await.expect("a stop is not an error");
    assert_eq!(code, RunExit::Limit);
    assert_eq!(
        summary.flags.stopped,
        Some("--max-write-bytes"),
        "which ceiling bound"
    );
    assert!(summary.flags.limited, "the ceiling was requested");
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "not a failure"
    );
    assert!(
        summary.write.live() <= limit,
        "the ceiling must not be overshot: the live counter is the one the \
         ceiling bounds"
    );
    assert!(
        summary.write.settled() <= limit,
        "and the reported figure is no larger than it"
    );

    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("stopped=--max-write-bytes"), "{line}");
    assert!(line.contains("write_bytes="), "{line}");

    // The file is left half-written: the next run re-verifies it and finishes
    // it, so an aborted run is a smaller run, not a corrupt one.
    assert!(!dir.join("config.ini").exists(), "post-phase is skipped");
    assert_ne!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "the file was not finished"
    );

    mock.stop();
}

#[tokio::test]
async fn write_bytes_is_counted_only_when_the_flag_is_active() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("write_bytes_absent");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    // Same run, no flag: the repair completes and the key is absent.
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("run");
    assert_eq!(code, RunExit::Success);
    assert!(summary.write.settled() > 0, "it was counted");
    let line = summary_record(&summary, code.code()).text();
    assert!(!line.contains("write_bytes"), "{line}");
    // `download_bytes` is the network; `write_bytes` is the disk. They are not the
    // same quantity and never were — that is the whole reason `--max-write-bytes`
    // exists. Here every byte written came from a download, so write_bytes is
    // simply the *uncompressed* size, which is strictly larger.
    assert!(
        summary.write.settled() > summary.download.settled(),
        "write_bytes counts uncompressed payload, download_bytes the compressed blob"
    );

    // A generous ceiling is reported but not tripped.
    let dir2 = temp_dir("write_bytes_generous");
    let fx3 = build_fixture();
    write_game_file(&dir2, &fx3.files[0].rel, b"not the game data");
    write_game_file(&dir2, &fx3.files[1].rel, b"not the game data");
    let mut ctx = ctx_for(dir2.to_path_buf(), &mock, 1);
    ctx.limits.write_bytes = Some(1 << 20);
    let (summary, code) = run(ctx).await.expect("run");
    assert_eq!(code, RunExit::Success);
    assert!(summary.flags.stopped.is_none());
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("write_bytes="), "{line}");
    assert!(!line.contains("stopped="), "{line}");

    mock.stop();
}

#[tokio::test]
async fn the_spent_figures_appear_only_when_their_ceiling_is_active() {
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;

    // No ceiling: neither figure is on the line.
    let dir = temp_dir("spent_absent");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, b"not the game data");
    }
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("run");
    assert_eq!(code, RunExit::Success);
    let line = summary_record(&summary, code.code()).text();
    assert!(!line.contains("_spent"), "{line}");

    // With the ceilings on, both figures are: what the repair achieved and
    // what it spent. In a clean run they are the same number — nothing was
    // wasted — and the spent one is the larger whenever the transport made
    // the run work for it.
    let dir2 = temp_dir("spent_present");
    let fx3 = build_fixture();
    for f in &fx3.files {
        write_game_file(&dir2, &f.rel, b"not the game data");
    }
    let mut ctx = ctx_for(dir2.to_path_buf(), &mock, 1);
    ctx.limits.download_bytes = Some(1 << 20);
    ctx.limits.write_bytes = Some(1 << 20);
    let (summary, code) = run(ctx).await.expect("run");
    assert_eq!(code, RunExit::Success);
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("download_bytes_spent="), "{line}");
    assert!(line.contains("write_bytes_spent="), "{line}");
    // The settled download figure is the base key, and in a clean run it is
    // the same number as the spent one: nothing was wasted.
    let spent = field(&line, "download_bytes_spent");
    let settled = field(&line, "download_bytes");
    assert_eq!(spent, settled, "a clean run wastes nothing: {line}");
    assert!(spent > 0, "the run really downloaded something: {line}");
    // The disk is a different quantity from the network, and the two families
    // are not the same number: the payload is uncompressed.
    assert!(
        field(&line, "write_bytes_spent") > field(&line, "download_bytes_spent"),
        "the disk cost is the uncompressed payload: {line}"
    );

    mock.stop();
}

#[tokio::test]
async fn the_verified_write_ceiling_stops_the_run_on_the_settled_figure() {
    // The fixture: `bar.dat` is one 48-byte chunk, `foo.dat` is two 64-byte
    // ones, and the plan is sorted — so `bar.dat` is repaired first and
    // settles 48 bytes, which is under the line. `foo.dat`'s first chunk
    // settles 64 more and crosses it.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("verified_write");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, b"not the game data");
    }

    let limit = 64u64;
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.limits.verified_write_bytes = Some(limit);
    let (summary, code) = run(ctx).await.expect("a stop is not an error");
    assert_eq!(
        code,
        RunExit::Limit,
        "the verified ceiling is a hard stop, like every other"
    );
    assert_eq!(
        summary.flags.stopped,
        Some("--max-verified-write-bytes"),
        "the stop must name the flag that bound"
    );
    assert!(summary.flags.limited, "the ceiling was requested");
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "not a failure: the ceiling stopped the run, the file did not fail"
    );
    assert_eq!(
        summary.files_repaired.load(Ordering::Relaxed),
        1,
        "the file that crossed the line was repaired; the next one was not started"
    );
    // The settled figure is counted even when it crosses the line: the write
    // happened, and a figure that stopped counting would be a lie.
    assert!(
        summary.write.settled() > limit,
        "the write that crossed the line is counted: {} > {limit}",
        summary.write.settled()
    );
    // And the live figure — the one `--max-write-bytes` bounds — is the same
    // number here: nothing was wasted, the run simply ran out of budget.
    assert_eq!(summary.write.live(), summary.write.settled());

    let line = summary_record(&summary, code.code()).text();
    assert!(
        line.contains("stopped=--max-verified-write-bytes"),
        "{line}"
    );
    assert!(line.contains("write_bytes="), "{line}");
    assert!(line.contains("limited=1"), "{line}");
    // The **spent** figure is the live one, and the live ceiling was not
    // asked for: the key is not on the line. The settled one is, because this
    // ceiling is about it.
    assert!(
        !line.contains("write_bytes_spent="),
        "the spent figure belongs to the live ceiling: {line}"
    );

    mock.stop();
}

#[tokio::test]
async fn the_live_and_the_verified_ceiling_stop_on_whichever_is_reached_first() {
    // The fixture: `bar.dat` is one 48-byte chunk and `foo.dat` is two
    // 64-byte ones, and the plan is sorted — so `bar.dat` is repaired first
    // (48 bytes) and `foo.dat`'s first chunk settles 64 more.
    async fn run(live: u64, verified: u64, tag: &str) -> &'static str {
        let fx = build_fixture();
        let mock = MockServer::start(fx, MockOpts::default()).await;
        let dir = temp_dir(tag);
        let fx2 = build_fixture();
        for f in &fx2.files {
            write_game_file(&dir, &f.rel, b"not the game data");
        }
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        ctx.limits.write_bytes = Some(live);
        ctx.limits.verified_write_bytes = Some(verified);
        let (summary, code) = girpr::run(ctx).await.expect("a stop is not an error");
        assert_eq!(code, RunExit::Limit, "a ceiling bound");
        mock.stop();
        summary
            .flags
            .stopped
            .expect("the ceiling that bound is named")
    }

    // The live ceiling is the tighter one: it is reached first, and it is the
    // one that is named.
    assert_eq!(
        run(1, 1 << 20, "both_live_first").await,
        "--max-write-bytes"
    );
    // The verified one is tighter: the settled figure crosses its line while
    // the live figure is still under its own.
    assert_eq!(
        run(1 << 20, 64, "both_verified_first").await,
        "--max-verified-write-bytes"
    );
}

#[tokio::test]
async fn the_verified_download_ceiling_stops_the_run_on_the_settled_figure() {
    let fx = build_fixture();
    // The settled download figure is counted in **compressed** bytes, and it
    // is settled per file — a file is the atom it is counted in.
    let bar_bytes = fx.chunk_store.get("chunk_bar0").unwrap().len() as u64;
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("verified_download");
    let fx2 = build_fixture();
    for f in &fx2.files {
        write_game_file(&dir, &f.rel, b"not the game data");
    }

    // A ceiling of zero: the first file that is repaired crosses it, and the
    // second one is never started.
    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.limits.verified_download_bytes = Some(0);
    let (summary, code) = run(ctx).await.expect("a stop is not an error");
    assert_eq!(
        code,
        RunExit::Limit,
        "the verified ceiling is a hard stop, like every other"
    );
    assert_eq!(
        summary.flags.stopped,
        Some("--max-verified-download-bytes"),
        "the stop must name the flag that bound"
    );
    assert!(summary.flags.limited, "the ceiling was requested");
    assert_eq!(
        summary.files_failed.load(Ordering::Relaxed),
        0,
        "not a failure: the ceiling stopped the run, the file did not fail"
    );
    assert_eq!(
        summary.files_repaired.load(Ordering::Relaxed),
        1,
        "the first file was repaired; the next one was not started"
    );
    assert!(
        summary.download.settled() >= bar_bytes,
        "the file that crossed the line is counted: {} >= {bar_bytes}",
        summary.download.settled()
    );
    // The live figure is the traffic: the same number here, because nothing
    // was fetched twice.
    assert_eq!(summary.download.live(), summary.download.settled());

    let line = summary_record(&summary, code.code()).text();
    assert!(
        line.contains("stopped=--max-verified-download-bytes"),
        "{line}"
    );
    assert!(line.contains("limited=1"), "{line}");

    mock.stop();
}

#[tokio::test]
async fn kept_bytes_is_the_complement_of_the_files_settled_writes() {
    // The fixture: `foo.dat` is two 64-byte chunks, and the mock serves the
    // second one corrupt on its first request. The first attempt writes the
    // first chunk — which passes its gate — and then fails on the second; the
    // second attempt finds the first chunk correct, writes the second, and
    // the file verifies.
    async fn run(opts: MockOpts, tag: &str) -> u64 {
        let fx = build_fixture();
        let mock = MockServer::start(fx, opts).await;
        let dir = temp_dir(tag);
        let fx2 = build_fixture();
        write_game_file(&dir, &fx2.files[0].rel, b"not the game data");
        let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
        ctx.repair_mode = RepairMode::Inplace;
        // The key is emitted only when the flag was passed.
        ctx.repair_mode_explicit = true;
        ctx.path_scope = scope(&["game/foo.dat"], &[]);
        let (summary, code) = girpr::run(ctx).await.expect("run");
        assert_eq!(
            code,
            RunExit::Success,
            "the file was repaired on the second attempt"
        );
        assert_eq!(
            read_game_file(&dir, &fx2.files[0].rel).unwrap(),
            fx2.files[0].data,
            "the file is correct"
        );
        mock.stop();
        summary.kept_bytes.load(Ordering::Relaxed)
    }

    let clean = run(MockOpts::default(), "kept_clean").await;
    let retried = run(
        MockOpts {
            corrupt_chunks: vec!["chunk_foo1".to_string()],
            ..Default::default()
        },
        "kept_retry",
    )
    .await;

    assert_eq!(clean, 0, "nothing was kept: the whole file was written");
    assert_eq!(
        retried, clean,
        "a retry must not look like a file that was left alone: the first \
         attempt's chunk is in the install, so it was not kept"
    );
}

#[tokio::test]
async fn download_bytes_counts_only_promoted_files_even_after_a_retry() {
    // `download_bytes` is the bytes of files that were actually
    // promoted. The counter is live per-chunk so the runtime hard stop can
    // read it mid-flight, which puts two ways of inflating it in reach of an
    // unflagged run — and both are exactly what an *abandoned* attempt looks
    // like:
    //
    //  - a reservation the `?` on a failed fetch never gave back, and
    //  - the bytes that attempt had already downloaded.
    //
    // The mock serves one corrupt-but-well-formed chunk, so the first attempt
    // really does transfer bytes and then fail its MD5.
    let fx = build_fixture();
    let est: u64 = fx.chunk_store.values().map(|b| b.len() as u64).sum();
    let mock = MockServer::start(
        fx,
        MockOpts {
            corrupt_first_chunks: 1,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("download_retry");
    let fx2 = build_fixture();
    write_game_file(&dir, &fx2.files[1].rel, b"not the game data");

    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("the retry must recover");
    assert_eq!(code, RunExit::Success);
    assert_eq!(
        read_game_file(&dir, &fx2.files[1].rel).unwrap(),
        fx2.files[1].data
    );
    // bar.dat sorts first and its single chunk is the corrupt one, so attempt 1
    // really transfers a byte and then fails the chunk MD5. The total is still
    // every chunk in the plan and nothing more: the abandoned attempt settles
    // nothing, because its bytes are not part of the install.
    assert_eq!(
        summary.download.settled(),
        est,
        "a failed attempt's bytes must not survive into the total"
    );
    // The **live** figure is the other one, and it is larger: the abandoned
    // attempt's bytes crossed the wire and were spent, whatever became of them.
    // That is the difference between the two numbers — the waste.
    assert!(
        summary.download.live() > summary.download.settled(),
        "the spent figure carries the abandoned attempt: {} > {}",
        summary.download.live(),
        summary.download.settled()
    );
    assert_eq!(
        summary.files_repaired.load(Ordering::Relaxed),
        2,
        "both files were repaired, the first one on its second attempt"
    );

    mock.stop();
}

#[tokio::test]
async fn write_bytes_counts_the_bytes_that_passed_their_gate() {
    // `SUMMARY write_bytes` counts the bytes written into the install **from
    // sources that passed their gate**, so it is a function of the *input
    // state*: the same install against a clean origin and against one that
    // serves a corrupt-but-well-formed chunk reports the same number. The
    // corrupt chunk contributes nothing — its bytes landed, but they were
    // never a source that passed a gate.
    //
    // The live counter the `--max-write-bytes` ceiling reads is a *different*
    // number, and it is larger in the corrupt run: the chunk was written and
    // then written again. That is the disk the run actually took, and it is
    // the number the ceiling bounds.
    //
    // A streamed chunk is what makes this a question: its bytes land before
    // its digest is known, so "what was written" and "what was allowed to be
    // written" are two different figures, and only the second one is a
    // function of the install.
    //
    // The manifest claims a `compressed_size` a few bytes larger than the good
    // body. That is not what the test is about: the streamed body is capped at
    // what the caller reserved, and the mock's corrupt body recompresses one
    // byte longer than the good one, so without the slack the cap would refuse
    // it before a single decoded byte landed — and the write meter would have
    // nothing to count. The corrupt body still has to pass its decode to fail
    // its digest, which is the case this test names.
    fn lying_fixture() -> common::GameFixture {
        build_fixture_with_lies(Lies {
            wrong_chunk_size: vec![
                "chunk_foo0".to_string(),
                "chunk_foo1".to_string(),
                "chunk_bar0".to_string(),
            ],
            ..Default::default()
        })
    }

    async fn run(opts: MockOpts, tag: &str) -> (u64, u64) {
        let fx = lying_fixture();
        let mock = MockServer::start(fx, opts).await;
        let dir = temp_dir(tag);
        let fx2 = lying_fixture();
        for f in &fx2.files {
            write_game_file(&dir, &f.rel, b"not the game data");
        }
        let (summary, code) = girpr::run(ctx_for(dir.to_path_buf(), &mock, 1))
            .await
            .expect("run");
        assert_eq!(code, RunExit::Success, "the repair recovered");
        for f in &fx2.files {
            assert_eq!(
                read_game_file(&dir, &f.rel).unwrap(),
                f.data,
                "{} is correct",
                f.rel
            );
        }
        mock.stop();
        (summary.write.settled(), summary.write.live())
    }

    let (clean_reported, clean_spent) = run(MockOpts::default(), "write_bytes_clean").await;
    let (corrupt_reported, corrupt_spent) = run(
        MockOpts {
            corrupt_first_chunks: 1,
            ..Default::default()
        },
        "write_bytes_corrupt",
    )
    .await;

    // The two files are 128 + 48 bytes of chunks, and every one of them is
    // written from a source that passed its gate.
    assert_eq!(clean_reported, 176, "the clean run's verified writes");
    assert_eq!(
        corrupt_reported, clean_reported,
        "a corrupt chunk contributes nothing: the figure is a function of the \
         install, not of the transport"
    );
    // The ceiling's number is the other one: the corrupt chunk landed and was
    // charged, and then it landed again.
    assert_eq!(
        clean_spent, clean_reported,
        "a clean run spends what it writes"
    );
    assert!(
        corrupt_spent > corrupt_reported,
        "the corrupt chunk's bytes are spent even though they never validated: \
         {corrupt_spent} > {corrupt_reported}"
    );
}

#[tokio::test]
async fn a_reuse_run_and_a_no_reuse_run_are_distinguishable_on_the_data_plane() {
    // A run that reused everything and a run that could not reuse anything are
    // otherwise identical on the data plane: both exit `0`, and the second's
    // `download_bytes` is simply larger. That matters most to
    // `--max-download-bytes`, whose pre-flight estimate is computed against the
    // same empty map — so a degraded run silently moves the ceiling and the
    // refusal a caller expected never fires.
    //
    // The mock answers `getBuild?tag=<local>` with the manifest only when the
    // tag is the latest one, which is exactly the "the local manifest loaded" /
    // "the server no longer serves that tag" pair.
    let est: u64 = build_fixture()
        .chunk_store
        .values()
        .map(|b| b.len() as u64)
        .sum();

    // (a) local version at the latest tag → the manifest loads → reuse works.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("local_manifest_ok");
    std::fs::write(
        dir.join("config.ini"),
        format!("[General]\ngame_version={}\n", LATEST_TAG),
    )
    .unwrap();
    let fx2 = build_fixture();
    // bar intact (skipped) and foo half-intact, so its first chunk is copied
    // from a verified local slice and only the second is fetched. That is a real
    // reuse, not an absent one.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("run");
    assert_eq!(code, RunExit::Success);
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("local_manifest=ok"), "{line}");
    assert_eq!(mock.chunk_hits(), vec!["chunk_foo1".to_string()]);
    assert!(
        summary.download.settled() < est,
        "a run with a loaded local manifest reuses something: {} vs {est}",
        summary.download.settled()
    );
    mock.stop();

    // (b) a local version the server does not serve → lookup attempted and
    //     failed → the fallback is full-fetch, and the run says so.
    let fx = build_fixture();
    let mock = MockServer::start(fx, MockOpts::default()).await;
    let dir = temp_dir("local_manifest_gone");
    std::fs::write(
        dir.join("config.ini"),
        "[General]\ngame_version=1.0.0-long-gone\n",
    )
    .unwrap();
    let fx2 = build_fixture();
    // The same tree as (a), so the only difference between the two runs is
    // whether the local manifest loaded.
    write_game_file(&dir, &fx2.files[1].rel, &fx2.files[1].data);
    let mut half_bad = fx2.files[0].data.clone();
    for b in half_bad.iter_mut().skip(64) {
        *b ^= 0xFF;
    }
    write_game_file(&dir, &fx2.files[0].rel, &half_bad);
    let (summary, code) = run(ctx_for(dir.to_path_buf(), &mock, 1))
        .await
        .expect("a degraded run still succeeds");
    assert_eq!(code, RunExit::Success, "the fallback is not an error");
    let line = summary_record(&summary, code.code()).text();
    assert!(line.contains("local_manifest=unavailable"), "{line}");
    assert_eq!(
        mock.chunk_hits(),
        vec!["chunk_foo1".to_string()],
        "with no local manifest there is no reuse *map*, but the in-place \
         repair keeps the chunk that is already correct where it belongs — \
         the scan finds it without any manifest"
    );
    // The estimate is what still moves: with no local manifest the plan's
    // estimate is every chunk, while the run only fetched the corrupt one.
    // That is the "the estimate is an upper bound" property, and it is why the
    // `local_manifest` key has to be there.
    assert_eq!(
        summary.flags.plan_download_bytes, None,
        "the estimate is only reported when the ceiling asked for it"
    );
    assert!(
        summary.download.settled() < est,
        "the degraded run still fetched less than the whole plan: {} vs {est}",
        summary.download.settled()
    );
    mock.stop();
}

#[tokio::test]
async fn a_corrupt_chunk_is_not_resumed_as_already_complete() {
    // The temp mode resumes by **length**: a chunk whose end is inside the
    // `_tmp` is taken as done, with no hash check of its own. A streamed
    // chunk's bytes are written before its digest is known, so a chunk that
    // fails its digest leaves a `_tmp` that has grown past it — and the next
    // attempt would keep the corrupt bytes and call the file complete.
    //
    // The rollback truncates the `_tmp` back to the chunk's offset, so the
    // next attempt downloads *that* chunk again and keeps the good prefix
    // before it. The mock serves one corrupt-but-well-formed chunk, so the
    // first attempt really does write bad bytes and then fail its digest.
    let fx = build_fixture();
    let mock = MockServer::start(
        fx,
        MockOpts {
            corrupt_first_chunks: 1,
            ..Default::default()
        },
    )
    .await;
    let dir = temp_dir("tmp_rollback");
    let fx2 = build_fixture();

    let mut ctx = ctx_for(dir.to_path_buf(), &mock, 1);
    ctx.repair_mode = RepairMode::Temp;
    // Only foo.dat: its first chunk is the corrupt one, and its second chunk
    // is the one that must **not** be downloaded twice.
    ctx.path_scope = scope(&["game/foo.dat"], &[]);
    let (summary, code) = run(ctx).await.expect("the retry must recover");
    assert_eq!(
        code,
        RunExit::Success,
        "the file was repaired on the second attempt"
    );
    let (_, repaired, failed, _, _) = load_counters(&summary);
    assert_eq!((repaired, failed), (1, 0));
    assert_eq!(
        read_game_file(&dir, &fx2.files[0].rel).unwrap(),
        fx2.files[0].data,
        "the file is correct"
    );
    // The corrupt chunk was asked for twice — once for the attempt that
    // failed its digest, once for the attempt that repaired it — and the good
    // chunk only once. Without the rollback the whole `_tmp` would be thrown
    // away when the final MD5 failed, and the good chunk would be downloaded
    // twice as well.
    assert_eq!(
        mock.chunk_hits(),
        vec![
            "chunk_foo0".to_string(),
            "chunk_foo0".to_string(),
            "chunk_foo1".to_string()
        ],
        "the corrupt chunk was re-downloaded, the good one was not"
    );

    mock.stop();
}
