//! Command dispatch: the four entry points a caller can invoke.
//!
//! Each returns a [`Record`] plus the [`crate::exit::ExitCode`] it earned, so the shim
//! in `main.rs` holds no logic: it dispatches, prints, exits.
//!
//! **The failure type is not here.** It lives in [`crate::failure`] and is
//! re-exported below, because `manifest`, `emit::selfcheck` and `blobs` all have
//! things to say on failure and could not say it without depending on the
//! dispatcher. `run::usage` and `run::failure_summary` still resolve; moving a
//! type is not a reason to touch every call site.

use std::path::PathBuf;

use crate::biz::{Biz, Endpoints};
use crate::config::{CapturePlan, MaterializePlan, VerifyPlan, artifact_dir};
use crate::emit::selfcheck::Selfcheck;
use crate::exit::ExitCode;
use crate::http::{Fetcher, RequestLog, Transport, negative, probe};
use crate::manifest::CapturedManifest;
use crate::manifest::diff;
use crate::report::{self, OutputFormat, Record};
use crate::{blobs, emit, hyp, manifest, select};

// The failure story moved to `crate::failure` so that the modules that own the
// work can name it. Re-exported because `main.rs` and the integration tests
// reach for `run::usage` and `run::failure_summary`, and there is no reason to
// make them learn a second path for the same thing.
pub use crate::failure::{
    Failure, Outcome, apierr, assertion, failure_summary, failure_summary_detail, usage, write_err,
};

/// `gircap capture` — fetch from the live APIs and write a bundle.
///
/// Writes the `api/` documents first: the four HoYoPlay bodies and both
/// `getBuild`s byte-exact, plus the generated tag probe, and
/// `api/requests.ndjson` beside them. Manifests, chunks and the `game/` fixture
/// follow.
///
/// The order is [`hyp::HypClient::capture_all`]'s, not this function's:
/// `getGameBranches` carries the `package_id` every `getBuild` URL is built from,
/// so nothing after it can be requested before it.
///
/// The bundle's directory is `<biz>/<old>__<new>/<id>/`, which is why the pair
/// is resolved before anything is written rather than after.
pub async fn capture(plan: CapturePlan) -> Result<Outcome, Failure> {
    let started = chrono::Utc::now();
    let biz = Biz::resolve(plan.biz);
    let log = RequestLog::new();
    let fetcher = Fetcher::new(
        Transport {
            retries: plan.retries,
            timeout: plan.timeout,
            max_response_bytes: plan.max_response_bytes,
        },
        log.clone(),
    )
    .map_err(|e| apierr("build the http client", &e))?;

    // A testing seam, and the only thing that redirects a capture: the ids still
    // come from the `--biz` mapping, so a mock run sends exactly what a live one
    // sends.
    let endpoints = if plan.hyp_base.is_none() && plan.sophon_base.is_none() {
        biz.bases()
    } else {
        Endpoints::owned(
            plan.hyp_base
                .clone()
                .unwrap_or_else(|| biz.hyp_base().to_string()),
            plan.sophon_base
                .clone()
                .unwrap_or_else(|| biz.sophon_base().to_string()),
        )
    };
    tracing::info!(
        hyp_base = endpoints.hyp,
        sophon_base = endpoints.sophon,
        override_in_play = plan.hyp_base.is_some() || plan.sophon_base.is_some(),
        "endpoints resolved"
    );

    // 1-4. All six metadata bodies, byte-exact, and the version pair.
    //
    // `capture_all` owns the ordering because it has to: `getGameBranches`
    // carries the `package_id` every `getBuild` URL is built from, so a caller
    // that fetched them out of order would request a URL with no package in it.
    let client = hyp::HypClient::new(endpoints.clone(), biz, &fetcher);
    let mut captured = client
        .capture_all(plan.new_selector(), plan.old_selector(), plan.jobs)
        .await
        .map_err(|e| apierr("metadata capture", &e))?;

    // Assert what the capture depends on, before a blob is fetched: the launcher
    // must be on chunk delivery, both builds must echo the tags they were asked
    // for, and every published manifest must be storable.
    captured
        .validate()
        .map_err(|e| assertion("validate the capture", &e))?;

    // Each body now knows which request produced it, so that `emit::write_api`
    // can mark that request's row in `requests.ndjson` with the path it wrote.
    // Until this ran, `seq` was `0` on all six and `set_stored_as(0, ..)` found
    // no row — so every `api` row recorded a null `stored_as` for a body that
    // was on disk. Cheap here because the log is already in hand; it cannot be
    // done later, because the log is written to `api/requests.ndjson` before the
    // close-out and nothing rewrites it.
    hyp::attach_seq(&log, &mut captured.bodies_mut());

    let versions = &captured.versions;
    let params = captured
        .branch_params()
        .map_err(|e| apierr("re-read the captured branch", &e))?;
    tracing::info!(
        biz = biz.as_str(),
        package_id = %params.package_id,
        old_tag = %versions.old.tag,
        old_source = %versions.old.source,
        old_build_id = %versions.old.build_id,
        new_tag = %versions.new.tag,
        new_source = %versions.new.source,
        new_build_id = %versions.new.build_id,
        diff_tags = ?versions.diff_tags,
        "version pair resolved"
    );
    let (new_tag, old_tag) = (versions.new.tag.clone(), versions.old.tag.clone());

    // 5. Which tags the API will serve at all. Cheap, and the fact that decides
    //    whether an old file is reconstructible today.
    let tags = probe::compose_tags(
        &[old_tag.clone(), new_tag.clone()],
        &params.diff_tags,
        &plan.probe_tags,
    );
    // The pair's `getBuild` was already fetched, so the probe does not pay for it
    // twice. This is the largest avoidable cost in a metadata-only run.
    let known = vec![
        (old_tag.clone(), versions.old.build_id.clone()),
        (new_tag.clone(), versions.new.build_id.clone()),
    ];
    let probe = probe::probe_tags(
        &fetcher,
        endpoints.sophon,
        &params,
        &tags,
        &params.diff_tags,
        &known,
        plan.jobs,
    )
    .await
    .map_err(|e| apierr("tag probe", &e))?;
    tracing::info!(probe = %probe.describe(), served = ?probe.served(), "tag probe complete");

    // 6. The manifests, both builds, for every selected `matching_field`.
    //
    //     Fetched all at once, filed by the slot each request carries rather than
    //     by the position it finished in. See `manifest::fetch` for why the order a
    //     download completes in cannot be allowed to reach the bundle.
    let fields = selection_fields(&plan);
    let old_build = captured
        .old_build()
        .map_err(|e| apierr("re-read the captured old build", &e))?;
    let new_build = captured
        .new_build()
        .map_err(|e| apierr("re-read the captured new build", &e))?;
    let manifest_wants =
        manifest::fetch::wants(&fields, &old_tag, &new_tag, &old_build, &new_build);
    // Every value a shape rule refuses during the run lands here. The store is
    // created before the manifests are fetched because that is the first place
    // an untrusted value reaches a write decision; the document is written with
    // the rest of the tree below.
    let rejections = std::sync::Arc::new(crate::rejections::Rejections::new());
    // The logical→on-disk names for device components. Every writer records
    // into it, and the document is written once the fixture has been assembled
    // (a game/expected path is only known then).
    let path_map = crate::pathmap::PathMap::new();
    let mut manifests = match manifest::fetch::fetch_all(
        &fetcher,
        &manifest_wants,
        fields.len(),
        plan.jobs,
        plan.max_decompressed_bytes,
        rejections.clone(),
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            // **The refusal is also evidence.** A hostile manifest path stops
            // the run before a bundle exists, so there is no `MANIFEST.json` to
            // carry the rejection document. Write what was refused into the
            // directory the bundle would have occupied anyway: the run's verdict
            // is unchanged, and the values are not left only in a scrollback.
            if !rejections.is_empty() {
                let dir = artifact_dir(&plan.out, plan.biz, &old_tag, &new_tag, started);
                match emit::write_rejections(&dir, &rejections) {
                    Ok(paths) => tracing::warn!(
                        dir = %dir.display(),
                        documents = ?paths,
                        "the run refused manifest input; the rejected values are recorded"
                    ),
                    Err(io) => tracing::warn!(
                        dir = %dir.display(),
                        error = %io,
                        "could not record the rejected values"
                    ),
                }
            }
            return Err(e);
        }
    };

    // 6a. What the API does with a request it will not serve.
    //
    //     Seven small requests and no blob downloaded. It must not be able to fail
    //     the capture: a negative response *is* the measurement. `negative` owns
    //     the two decisions that decide what gets asked for - which pair, and
    //     which chunk id - because both are pure and both were previously only
    //     reachable by running a capture against the live API.
    let game_pair = negative::game_pair(&fields, &manifests)?;
    let (old_game, new_game) = manifest::fetch::pair_sides(game_pair, "the game manifest pair")?;
    let new_only_chunk_id =
        negative::new_only_chunk_id(negative::chunk_ids(old_game), negative::chunk_ids(new_game))?;
    let error_shapes = probe::probe_error_shapes(
        &fetcher,
        &endpoints,
        &biz,
        &params,
        &probe::ErrorProbes {
            manifest_prefix: &new_game.meta.declared.manifest_download.url_prefix,
            chunk_prefix: new_game.chunk_url_prefix(),
            other_manifest_prefix: &old_game.meta.declared.manifest_download.url_prefix,
            other_chunk_prefix: old_game.chunk_url_prefix(),
            new_only_chunk_id: &new_only_chunk_id,
        },
        plan.jobs,
    )
    .await
    .map_err(|e| apierr("negative probes", &e))?;
    tracing::info!(shapes = %error_shapes.describe(), "error shapes recorded");

    // 6b. A diff per field, then the pool built across all of them.
    let diffs = diff::diff_all(&fields, &manifests)?;

    // The aggregate is computed **once**, here, and used twice: it is both the
    // `diff/paths.diff.json` body and the headline numbers on the summary, and
    // each of those would otherwise be an O(rows) walk plus two sorted
    // `Vec<String>` over the same inputs.
    let agg = diff::aggregate(&old_tag, &new_tag, &diffs);

    // 6c. Score, classify, admit. Pure arithmetic over `diffs` - the only work
    //     this adds to the run is the audio manifests fetched above. The decision
    //     and the account of it moved together into `select::decide`.
    let decided =
        select::decide::decide(&diffs, plan.per_class, plan.max_download_bytes, &plan.files);
    let (selection, candidates_scored) = (decided.selection, decided.pool_len);

    // 6d. `--require-class` is the only thing that turns an absent class into a
    //     failed run. Without it an over-budget class is recorded and the run
    //     still exits 0 - the caller asked for a best effort and got one.
    select::require::check(&plan.require_class, &selection)?;

    // 6e. The chunk store. Planned *before* any blob is fetched, so the cost the
    //     log announces is a real figure rather than an estimate, and A12 is
    //     checked on the plan instead of after the bytes have been paid for.
    let chunk_plan = blobs::plan::plan(
        &fields,
        &manifests,
        &selection,
        &old_tag,
        &new_tag,
        plan.max_download_bytes,
        plan.no_chunk_blobs,
    )?;
    let pairs = chunk_plan.pairs;
    let wants: std::sync::Arc<[blobs::chunks::Want]> = chunk_plan.wants.into();
    let projected = chunk_plan.projected;

    // 7. The tree. All four writes happen even though the run is incomplete:
    //    what was asked, what it cost, and what came back are the parts that
    //    stay useful, and the bytes are the evidence behind every number in the
    //    summary. `manifests` is borrowed mutably because marking where each
    //    archive landed is what makes the chunk store's URLs resolve.
    let dir = artifact_dir(&plan.out, plan.biz, &old_tag, &new_tag, started);
    let tree = emit::tree::write(emit::tree::TreeInput {
        dir: &dir,
        captured: &captured,
        probe: &probe,
        error_shapes: &error_shapes,
        manifests: &mut manifests,
        log: &log,
        gzip_threshold: plan.gzip_threshold,
        manifest_detail: plan.manifest_detail,
        agg: &agg,
        diffs: &diffs,
        selection: &selection,
        per_class: plan.per_class as usize,
        path_map: &path_map,
    })?;
    let mut written = tree.written;
    let mut api_documents = tree.api_documents;

    // 7a-ii. What the run refused, beside what it kept.
    //
    //     Written after the tree so it joins `files[]` and the checksum list,
    //     and before the chunk fetches because every shape refusal happens at
    //     the manifest boundary or earlier. An empty document is still written:
    //     "no hostile value was seen" is a fact, and leaving the file out would
    //     make it indistinguishable from "this run did not look".
    written.extend(emit::write_rejections(&dir, &rejections).map_err(|e| write_err(&dir, e))?);

    // 7b. The chunk blobs themselves, then `chunks/index.json`.
    //
    //     The index is written either way. `--no-chunk-blobs` produces one with
    //     every role list filled and no blobs, which is how a reader tells "this
    //     capture did not fetch" from "this capture had nothing to fetch".
    //     `patchmaps/` is created by `write_chunk_index`, not here: a caller must
    //     not be able to get a bundle with one and not the other.
    let store = blobs::Store::new(
        &dir,
        blobs::Sources::from_manifests(manifests.iter().flatten()),
        plan.max_decompressed_bytes,
    );
    let filled = blobs::fill::fill_or_plan(
        &store,
        &fetcher,
        &wants,
        plan.jobs,
        plan.no_chunk_blobs,
        projected,
    )
    .await;
    let fetch_errors = filled.errors;
    let index = blobs::ChunkIndex::build(filled.entries);
    tracing::info!(detail = %blobs::describe(&wants, &index), "chunk store done");
    written.push(emit::write_chunk_index(&dir, &index).map_err(|e| write_err(&dir, e))?);

    // 7b-ii. The bodies of the non-2xx responses the run paid for.
    //
    //     Written here, not with the other `api/` documents, because chunk
    //     fetches happen after those: a `404` on one blob and a `429` on another
    //     are only known now. `write_requests_log` below is later still, so every
    //     `stored_path` these rows name is already on disk and in `written`.
    {
        let bodies = fetcher.error_bodies();
        let paths = emit::write_error_responses(&dir, &bodies.snapshot(), bodies.budget())
            .map_err(|e| write_err(&dir, e))?;
        api_documents += paths.len() as u64;
        written.extend(paths);
    }

    // 7c. The fixture: assemble every admitted capture and write both sides,
    //     plus the launcher metadata that is not a chunk at all.
    //
    //     A7-A12 are asserted here rather than left to `verify`, because a capture
    //     that cannot rebuild its own files has nothing worth keeping and the
    //     operator should find out at capture time.
    let fixture = emit::fixture::build(emit::fixture::FixtureInput {
        dir: &dir,
        captured: &captured,
        versions,
        manifests: &manifests,
        index: &index,
        admitted: &selection.admitted,
        pairs: &pairs,
        store: &store,
        biz: &biz,
        old_tag: &old_tag,
        audio: plan.audio.codes(),
        no_chunk_blobs: plan.no_chunk_blobs,
        max_decompressed_bytes: plan.max_decompressed_bytes,
        path_map: &path_map,
    })?;
    let scan_dir = fixture.scan_dir;
    let rows = fixture.rows;
    let assembled_ids = fixture.assembled_ids;
    let side_digests = fixture.side_digests;
    let shapes = fixture.shapes;
    let (reused_total, fetched_total, moved_total) =
        (fixture.reused, fixture.fetched, fixture.moved);
    let assembly_failures = fixture.failures;
    written.extend(fixture.written);

    // 7c-ii. The logical→on-disk names, now that the fixture has written both
    //        sides. Written after `game/`/`expected/` because their mappings are
    //        the ones a materializing reader needs, and before the close-out so
    //        it joins `files[]` and the checksum list.
    written.extend(emit::write_path_map(&dir, &path_map).map_err(|e| write_err(&dir, e))?);

    // On the blocking pool. Serialising every record and writing them is both
    // CPU-bound and synchronous `std::fs`, so doing it inline inside
    // `async fn capture` holds a worker unavailable to *fetch* anything for the
    // duration. `RequestLog` is `Arc`-backed and cheap to clone, so the log moves
    // to the pool rather than being borrowed.
    {
        let for_the_pool = log.clone();
        let dir_for_the_pool = dir.clone();
        write_requests_log_on_the_pool(move || {
            emit::write_requests_log(&dir_for_the_pool, &for_the_pool)
        })
        .await
        .map_err(|e| write_err(&dir, std::io::Error::other(e)))?
        .map_err(|e| write_err(&dir, e))?;
    }

    // 7d. The closing documents, and A16.
    //
    //     Four documents and a fixpoint. `close` owns the order because no single
    //     one of the writes can say when it is allowed: `checksums.json` must
    //     cover `selfcheck.json`, and A16 - which asks whether it does - cannot be
    //     answered until `checksums.json` exists. So the two are written twice and
    //     the second pass is what lands. See `emit::closeout`.
    let closed = emit::closeout::close(emit::closeout::CloseoutInput {
        dir: &dir,
        rows,
        manifest: emit::index::ManifestInput {
            started,
            biz: &biz,
            endpoints: &endpoints,
            versions,
            diff_tags: &params.diff_tags,
            audio: plan.audio.codes(),
            fields: &fields,
            gzip_threshold: plan.gzip_threshold,
            max_decompressed_bytes: plan.max_decompressed_bytes,
            diffs: &diffs,
            selection: &selection,
            index: &index,
            bytes_downloaded: log.bytes_in(),
            argv: &plan.argv,
            complete: true,
        },
        old_tag: &old_tag,
        scan_dir: &scan_dir,
        selection: &selection,
        diffs: &diffs,
        side_digests: &side_digests,
        shapes: &shapes,
        written,
        no_chunk_blobs: plan.no_chunk_blobs,
    })?;
    let assembly_rows = closed.assembly_rows;

    let documents = closed.written.len();
    let manifests_summary = manifest_stats(&manifests);
    tracing::info!(
        dir = %dir.display(),
        records = log.len(),
        documents,
        manifest_entries = manifests_summary.entries,
        manifest_chunks = manifests_summary.chunks,
        "manifests written"
    );

    let legacy_behind = captured.legacy_staleness().map(|(n, _)| n).unwrap_or(0);
    // The diff's numbers are the run's headline result, so they go on the data
    // plane and not only into the stderr narration: this is what a caller reads.
    let pool_stats = &diffs[0].pool;
    let exit_code = ExitCode::Ok.code();
    let summary = report::capture_summary(&report::CaptureSummary {
        result: "ok",
        biz: biz.as_str(),
        old_tag: &old_tag,
        new_tag: &new_tag,
        package_id: &params.package_id,
        old_build_id: &versions.old.build_id,
        new_build_id: &versions.new.build_id,
        artifact: dir.as_path(),
        api_documents,
        selection_pool: &fields.join(","),
        selection: report::SelectionStats {
            candidates_scored,
            captures_admitted: selection.admitted.len() as u64,
            captures_over_budget: selection.rejected.len() as u64,
            forced_files_requested: plan.files.len() as u64,
            forced_files_skipped: selection.forced_skipped.len() as u64,
            forced_files_unscored: selection.forced_unscored.len() as u64,
            capture_bytes_admitted: selection.bytes_admitted,
            budget_unlimited: selection.budget.limit().is_none(),
            budget_limit: selection.budget.limit().unwrap_or(0),
            classes_satisfied: satisfied(&selection),
            classes_unmet: selection.unmet().len() as u64,
        },
        chunks: report::ChunkStats {
            wanted: wants.len() as u64,
            stored: index
                .blobs
                .iter()
                .filter(|b| b.props.stored_path.is_some())
                .count() as u64,
            bytes_projected: projected,
            bytes_fetched: index.blobs.iter().map(|b| b.props.bytes).sum::<u64>(),
            fetches_failed: fetch_errors.len() as u64,
            a6_failing: index.failing().len() as u64,
            patch_fetch_chunks: index.sets["patch_fetch"].len() as u64,
        },
        assembly: report::AssemblyStats {
            patchmaps_written: assembled_ids.len() as u64,
            selfcheck_rows: assembly_rows,
            selfcheck_failures: assembly_failures as u64,
            patch_reuse_chunks: reused_total,
            patch_fetch_chunks: fetched_total,
            patch_moved_chunks: moved_total,
        },
        manifests: manifests_summary,
        diff: report::DiffStats {
            common: agg.counts.common,
            identical: agg.counts.identical,
            changed: agg.counts.changed,
            added: agg.counts.added,
            removed: agg.counts.removed,
            patch_bytes: agg.byte_counts.patch,
        },
        reuse: report::ReuseStats {
            md5_present_in_old: pool_stats.new_chunks_md5_present_in_old_manifest,
            cross_file_only_chunks: pool_stats.cross_file_only_chunks,
            cross_file_bytes_forgone: pool_stats.cross_file_only_bytes_forgone,
        },
        http: report::HttpStats {
            documents_written: documents as u64,
            requests: log.len() as u64,
            bytes_downloaded: log.bytes_in(),
        },
        probe: report::ProbeStats {
            probed: probe.summary.requested as u64,
            served: probe.summary.served as u64,
            served_outside_diff_tags: probe.summary.not_in_diff_tags_but_served as u64,
        },
        legacy_endpoint_majors_behind: legacy_behind,
        exit_code,
    });

    // The bundle is written, the summary is built, and only now is the verdict
    // taken. A capture that fails its own assertions still gets to report how
    // much it did and what it spent — and that report is carried on the failure
    // rather than printed here, because `--output` is resolved in the shim above
    // every command and this function has never known it.
    match closed.fatal {
        Some(f) => Err(f.preceded_by(summary)),
        None => Ok((summary, ExitCode::Ok)),
    }
}

/// Five sums, a max and a count over every manifest the run wrote.
///
/// Moved out of `capture()` with the rest of the summary. It was a bare loop
/// between two `tracing::info!` calls, and it is the only place these five
/// numbers are computed - so a reader tracing `manifest_bytes` back to its
/// source had to find it in the middle of the orchestrator rather than beside
/// the key that reports it.
///
/// One walk, as before. It was five walks of the same slice when each number had
/// its own `iter().flatten()` chain, which is what the old comment on the loop
/// was correcting.
fn manifest_stats(manifests: &[Vec<CapturedManifest>]) -> report::ManifestStats {
    let mut stats = report::ManifestStats {
        captured: 0,
        bytes: 0,
        entries: 0,
        chunks: 0,
        max_chunk_compressed_bytes: 0,
    };
    for m in manifests.iter().flatten() {
        stats.captured += 1;
        stats.bytes += m.meta.stored.bytes;
        stats.entries += m.meta.decoded.entries;
        stats.chunks += m.meta.decoded.total_chunks;
        stats.max_chunk_compressed_bytes = stats
            .max_chunk_compressed_bytes
            .max(m.meta.decoded.max_chunk_compressed_bytes);
    }
    stats
}

/// The `matching_field`s whose manifests are fetched, `game` first.
///
/// `game` is not optional: every later stage needs it whatever the class, and a
/// pool that could omit it would let a cheap voice pack answer a class that a
/// game file covers for less. The audio codes are appended in the order the
/// caller named them so two runs with the same flags score in the same order.
fn selection_fields(plan: &CapturePlan) -> Vec<String> {
    let mut fields = vec![manifest::MANIFEST_FIELD.to_string()];
    fields.extend(plan.audio.codes().iter().cloned());
    fields
}

/// How many classes the matrix answers, captured or covered.
fn satisfied(selection: &select::Selection) -> u64 {
    selection.matrix.iter().filter(|r| r.is_satisfied()).count() as u64
}

/// What `verify` and `materialize` both decide about a re-derived bundle.
///
/// The two commands describe different work on their summaries, but they reach
/// the same verdict over the same [`crate::verify::Report`] and translate it into
/// the same exit code. Keeping that decision here means the rule "a headline is
/// exit `3`" is stated once, and cannot drift between the commands.
struct Verdict {
    total: u64,
    passed: u64,
    failed: u64,
    agrees: bool,
    headline: Option<String>,
}

impl Verdict {
    fn of(report: &crate::verify::Report) -> Self {
        let (total, passed, failed) = report.counts();
        Self {
            total,
            passed,
            failed,
            agrees: report.agrees,
            headline: report.headline(),
        }
    }

    /// `pass` when every re-derived row held and the recording agreed.
    fn ok(&self) -> bool {
        self.failed == 0 && self.agrees
    }

    /// The code the summary records: the verdict itself, before the headline.
    fn recorded_exit_code(&self) -> ExitCode {
        if self.ok() {
            ExitCode::Ok
        } else {
            ExitCode::AssertionFailed
        }
    }

    /// The code the command exits with: a headline means `3` even when the
    /// verdict is `ok`, because the line it carries is the reason to look.
    fn exit_code(&self) -> ExitCode {
        if self.headline.is_some() {
            ExitCode::AssertionFailed
        } else {
            self.recorded_exit_code()
        }
    }
}

/// `gircap verify` — re-check a bundle from its stored bytes, no network.
///
/// Exits `0` when every re-derived row holds and `3` when one does not; there is
/// no network failure to distinguish, because there is no network.
///
/// The summary names `checks_derived` and `checks_covered_by_recording` because
/// "verified" is a claim about how much was actually checked, and a reader who
/// cannot tell those two apart will read seventeen when six were looked at.
pub async fn verify(plan: VerifyPlan) -> Result<Outcome, Failure> {
    let bundle = crate::verify::Bundle::open(&plan.artifact).map_err(|e| {
        Failure::usage(format!(
            "verify: {} is not a readable gircap bundle: {e}",
            plan.artifact.display()
        ))
    })?;
    tracing::info!(artifact = %plan.artifact.display(), bundle = %bundle.describe(), "opened");
    let report = crate::verify::reverify(bundle);
    for r in report.failures() {
        tracing::error!(
            assertion = %r.id,
            subject = %r.subject,
            expected = %r.expected,
            actual = %r.actual,
            "re-derived assertion does not hold"
        );
    }
    for f in &report.findings {
        tracing::error!(detail = %f, "finding");
    }
    if !report.agrees {
        tracing::error!(
            "the bundle's recorded selfcheck claims rows hold that re-derivation does not; \
             the bundle has changed since it was written"
        );
    }
    let verdict = Verdict::of(&report);
    let summary = Record::summary("verify")
        .set("result", if verdict.ok() { "pass" } else { "fail" })
        .set("artifact", plan.artifact.as_path())
        .set("bundle", report.bundle.manifest.capture.id.clone())
        .set("old_tag", report.bundle.manifest.versions.old.tag.clone())
        .set("new_tag", report.bundle.manifest.versions.new.tag.clone())
        .set(
            "files_checked",
            report.bundle.checksums.entries.len() as u64,
        )
        .set("blobs_checked", report.bundle.index.blobs.len() as u64)
        .set("captures_rebuilt", report.rebuilt.len() as u64)
        .set("bytes_rebuilt", report.rebuilt_bytes())
        .set("checks_derived", crate::verify::derived_ids().join(","))
        .set(
            "checks_covered_by_recording",
            covered_ids(&report.bundle.recorded),
        )
        .set("checks_total", verdict.total)
        .set("checks_passed", verdict.passed)
        .set("checks_failed", verdict.failed)
        .set("agrees_with_recording", verdict.agrees)
        .set("findings", report.findings.len() as u64)
        .set("recorded_result", selfcheck_slug(&report.bundle.recorded))
        .set("exit_code", verdict.recorded_exit_code().code());
    if let Some(h) = &verdict.headline {
        return Ok((summary.set("detail", h.clone()), verdict.exit_code()));
    }
    Ok((summary, verdict.exit_code()))
}

/// `gircap materialize` — rebuild both files from `chunks/`, no network.
///
/// The same rebuild `verify` performs, written out rather than only compared.
/// It is a separate command because the two answer different questions:
/// `verify` asks *is the bundle closed*, `materialize` hands a consumer the bytes
/// and lets *it* do the comparing — which is the only way to answer "does a
/// downstream tool accept these bytes" without `gircap` guessing what it does.
pub async fn materialize(plan: MaterializePlan) -> Result<Outcome, Failure> {
    let bundle = crate::verify::Bundle::open(&plan.artifact).map_err(|e| {
        Failure::usage(format!(
            "materialize: {} is not a readable gircap bundle: {e}",
            plan.artifact.display()
        ))
    })?;
    // Beside the bundle, never inside it. Writing the rebuild into the tree it is
    // proving would add a file that `checksums.json` does not list, so the very
    // next `verify` would fail A16 on a bundle this command had just "repaired".
    // A sibling named `<capture_id>-materialized` keeps the two comparable and the
    // bundle untouched.
    let out = match &plan.out {
        Some(d) => d.clone(),
        None => {
            // Named after the bundle's own directory, not its capture id: the
            // two are the same string today, but the directory name is what the
            // operator is looking at, and the output lands *beside* the bundle
            // either way.
            let stem = plan
                .artifact
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| bundle.manifest.capture.id.clone());
            match plan.artifact.parent() {
                Some(p) if !p.as_os_str().is_empty() => p.join(format!("{stem}-materialized")),
                // The artifact is at a filesystem root, so there is no beside.
                _ => PathBuf::from(format!("{stem}-materialized")),
            }
        }
    };
    if out.starts_with(&plan.artifact) {
        tracing::warn!(
            out = %out.display(),
            "the rebuild is written inside the bundle it is proving; A16 will fail on the \
             next verify until checksums.json is regenerated"
        );
    }
    let report = crate::verify::reverify(bundle);
    let mut written: Vec<String> = Vec::new();
    let mut bytes: u64 = 0;
    for pair in &report.rebuilt {
        for (side, tree) in [(&pair.old, "game"), (&pair.new, "expected")] {
            let Some(s) = side else { continue };
            let rel = format!("{tree}/{}", s.path);
            // `Bundle::open` refuses a patchmap path that leaves the bundle, so
            // this is defence in depth for a caller that builds a report itself.
            // The check belongs at the join: `out.join("game/../../x")` is the
            // write, and `out.starts_with(&plan.artifact)` above tests the chosen
            // directory, not this destination.
            //
            // A device component is legal in the *logical* path and is escaped
            // for the write, exactly as the capture wrote it: the rebuild must
            // reproduce the bundle's own tree, not the game's unescaped one.
            if let Some(problem) = crate::paths::logical_path_problem(&rel) {
                return Err(Failure::usage(format!(
                    "materialize: the rebuilt path {problem}"
                )));
            }
            let on_disk = crate::paths::escape_path(&rel);
            let dest = out.join(&on_disk);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| write_err(&plan.artifact, e))?;
            }
            std::fs::write(&dest, &s.bytes).map_err(|e| write_err(&dest, e))?;
            written.push(on_disk);
            bytes += s.bytes.len() as u64;
        }
    }
    written.sort();
    tracing::info!(
        artifact = %plan.artifact.display(),
        out = %out.display(),
        files = written.len(),
        bytes,
        "rebuilt both files from chunks/"
    );
    let verdict = Verdict::of(&report);
    let summary = Record::summary("materialize")
        .set("result", if verdict.ok() { "pass" } else { "fail" })
        .set("artifact", plan.artifact.as_path())
        .set("out", out.as_path())
        .set("captures_rebuilt", report.rebuilt.len() as u64)
        .set("files_written", written.len() as u64)
        .set("bytes_written", bytes)
        .set("first_file", written.first().cloned().unwrap_or_default())
        .set("last_file", written.last().cloned().unwrap_or_default())
        .set("checks_total", verdict.total)
        .set("checks_passed", verdict.passed)
        .set("checks_failed", verdict.failed)
        .set("agrees_with_recording", verdict.agrees)
        .set("exit_code", verdict.recorded_exit_code().code());
    if let Some(h) = &verdict.headline {
        return Ok((summary.set("detail", h.clone()), verdict.exit_code()));
    }
    Ok((summary, verdict.exit_code()))
}

/// `gircap list` — index every capture under a root.
///
/// The only command that takes a root rather than a bundle, because its
/// job is to find bundles. It is meant to be run over a tree that may contain
/// a half-finished capture, so a directory that is not a bundle is not an
/// error. There are three arms, and they differ:
///
/// 1. A directory that is not a bundle at all — no `MANIFEST.json`, or a
///    manifest that declares no schema version this build can read — is
///    stepped over silently. A half-written capture is a normal transient.
/// 2. A directory that *is* a bundle this build refuses — an older schema, a
///    permanent condition with a known fix — is named on stderr with the reason
///    and counted in the summary's `bundles_unreadable`. It is still not a
///    `BUNDLE` record: there is no bundle to describe, so a record would have
///    to carry the failure instead of the facts, and a caller asking "which
///    bundles are here" would get an entry that is not one.
/// 3. A directory, entry or `MANIFEST.json` the walk **cannot read** stops the
///    command with a usage error naming what could not be read — exit `1`,
///    `result=error`. An inventory that quietly omits a subtree it never saw is
///    wrong, and a bundle behind a share lock or an ACL denial is exactly that.
///
/// One `BUNDLE` record per **valid** bundle on stdout, then one `SUMMARY`. The
/// per-bundle facts are on the data plane so a machine can consume this command
/// without parsing narration; `complete` and `selfcheck` in particular are the
/// two a caller needs to decide which bundle to verify, and neither is on the
/// summary.
///
/// **The summary still counts the ones that are bundles.** A bundle this build
/// refuses — an older schema — is a permanent condition with a known fix, and
/// an operator whose inventory quietly omits one has an inventory that is
/// wrong. So the count is on the summary and the path and the reason are on
/// stderr.
pub async fn list(root: PathBuf, output: OutputFormat) -> Result<Outcome, Failure> {
    let index = crate::verify::list(&root).map_err(|e| Failure::usage(e.to_string()))?;
    let found = index.bundles;
    let capturing = found.iter().filter(|f| f.selfcheck == "pass").count() as u64;
    let blobs: u64 = found.iter().map(|f| f.blobs as u64).sum();
    for f in &found {
        tracing::info!(
            path = %f.path,
            id = %f.capture_id,
            tags = %f.tags,
            patchmaps = f.patchmaps,
            blobs = f.blobs,
            selfcheck = %f.selfcheck,
            complete = f.complete,
            "capture"
        );
        Record::labeled("BUNDLE")
            .set("path", f.path.as_str())
            .set("id", f.capture_id.as_str())
            .set("tags", f.tags.as_str())
            .set("patchmaps", f.patchmaps as u64)
            .set("blobs", f.blobs as u64)
            .set("selfcheck", f.selfcheck.as_str())
            .set("complete", f.complete)
            .emit(output);
    }
    for u in &index.unreadable {
        tracing::error!(
            path = %u.path,
            reason = %u.reason,
            "a bundle under --root that this build will not read; not in the index"
        );
    }
    let summary = Record::summary("list")
        .set("result", "ok")
        .set("root", root.as_path())
        .set("bundles", found.len() as u64)
        .set("bundles_passing", capturing)
        .set("bundles_unreadable", index.unreadable.len() as u64)
        .set(
            "bundles_incomplete",
            found.iter().filter(|f| !f.complete).count() as u64,
        )
        .set(
            "patchmaps",
            found.iter().map(|f| f.patchmaps as u64).sum::<u64>(),
        )
        .set("blobs", blobs)
        .set(
            "first",
            found.first().map(|f| f.path.clone()).unwrap_or_default(),
        )
        .set(
            "last",
            found.last().map(|f| f.path.clone()).unwrap_or_default(),
        )
        .set("exit_code", ExitCode::Ok.code());
    Ok((summary, ExitCode::Ok))
}

/// The ids a recorded selfcheck covers that `verify` did not re-derive.
fn covered_ids(s: &Selfcheck) -> String {
    let mut ids: Vec<&str> = s
        .assertions
        .iter()
        .map(|a| a.id.as_str())
        .filter(|id| !crate::verify::REDERIVED.contains(id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.join(",")
}

/// The recorded `result`, as its lowercase wire spelling.
fn selfcheck_slug(s: &Selfcheck) -> String {
    serde_json::to_string(&s.result)
        .unwrap_or_else(|_| "?".into())
        .trim_matches('"')
        .to_string()
}

/// Write the request log on the blocking pool, keeping the cause if it fails.
///
/// The closure is a parameter rather than the write itself so a test can drive
/// the panic arm; the rule is [`crate::tasks::join_blocking`], shared with the
/// blob measurement. A cancellation is named as a cancellation, and a panic
/// keeps the message the writer gave — the failure the operator needs instead
/// of `task <id> panicked`.
async fn write_requests_log_on_the_pool<F>(work: F) -> Result<std::io::Result<usize>, String>
where
    F: FnOnce() -> std::io::Result<usize> + Send + 'static,
{
    crate::tasks::join_blocking("request log write", work).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::ApiError;
    use crate::report::Val;

    /// An assertion on captured API state is exit 3, and says why: neither the
    /// network nor the request is at fault.
    #[test]
    fn an_assertion_failure_is_three_and_is_not_mistaken_for_a_network_error() {
        let f = assertion("validate the capture", &ApiError::shape("x", "y"));
        assert_eq!(f.code, ExitCode::AssertionFailed);
        assert!(f.detail.contains("validate the capture"), "{}", f.detail);
        assert!(f.detail.contains("y"), "{}", f.detail);
        assert_eq!(
            f.summary("capture").get("exit_code"),
            Some(&Val::Int(3)),
            "not 2: the request worked and said something unusable"
        );
    }

    #[test]
    fn a_usage_failure_is_one_and_carries_the_message() {
        let f = usage("--jobs", "--jobs must be at least 1");
        assert_eq!(f.code, ExitCode::Usage);
        assert_eq!(
            f.summary("capture").get("detail"),
            Some(&Val::Str("--jobs must be at least 1".into()))
        );
    }

    #[test]
    fn a_failure_summary_names_the_code_and_says_whether_the_tree_can_be_believed() {
        let s = failure_summary("capture", ExitCode::Network);
        assert_eq!(s.get("exit_code"), Some(&crate::report::Val::Int(2)));
        assert_eq!(
            s.get("exit_code_slug"),
            Some(&crate::report::Val::Str("network".into()))
        );
        assert_eq!(
            s.get("result"),
            Some(&crate::report::Val::Str("error".into()))
        );
        assert_eq!(s.get("detail"), None);

        let d = failure_summary_detail("capture", ExitCode::AssertionFailed, Some("boom"));
        assert_eq!(
            d.get("detail"),
            Some(&crate::report::Val::Str("boom".into()))
        );
        assert_eq!(
            d.get("artifact_trustworthy"),
            Some(&crate::report::Val::Bool(false)),
            "an assertion failure leaves a tree that must be read, not deleted"
        );
    }

    #[test]
    fn the_detail_survives_the_text_rendering_as_one_token() {
        let line = failure_summary_detail(
            "capture",
            ExitCode::Network,
            Some("getBuild: retcode -202 (not found)"),
        )
        .to_text_line();
        assert_eq!(
            line.split_whitespace().filter(|t| t.contains('=')).count(),
            line.split_whitespace().count() - 1,
            "a spaced detail would break tokenisation: {line}"
        );
        assert!(line.contains("retcode%20-202"), "{line}");
    }

    /// **N1.** A panic inside the request-log write keeps its payload and is
    /// routed through the shared `JoinError` rule, not formatted with `Display`.
    #[tokio::test]
    async fn a_panicking_request_log_task_reports_its_payload() {
        let why = write_requests_log_on_the_pool(|| panic!("request log exploded"))
            .await
            .expect_err("the blocking task panicked");
        assert!(
            why.contains("the request log write task panicked"),
            "the site keeps its label: {why}"
        );
        assert!(
            why.contains("request log exploded"),
            "the payload is the part worth reporting: {why}"
        );
    }

    #[tokio::test]
    async fn the_request_log_task_still_returns_the_write_count() {
        assert_eq!(
            write_requests_log_on_the_pool(|| Ok(7))
                .await
                .expect("the task returned")
                .expect("the write succeeded"),
            7
        );
    }
}
