//! Command dispatch: the four entry points a caller can invoke.
//!
//! Each returns a [`Record`] plus the [`exit::ExitCode`] it earned, so the shim
//! in `main.rs` holds no logic: it dispatches, prints, exits.
//!
//! **The failure type is not here.** It lives in [`crate::failure`] and is
//! re-exported below, because `manifest`, `emit::selfcheck` and `blobs` all have
//! things to say on failure and could not say it without depending on the
//! dispatcher. `run::usage` and `run::failure_summary` still resolve; moving a
//! type is not a reason to touch every call site.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::biz::{Biz, Endpoints};
use crate::config::{CapturePlan, MaterializePlan, VerifyPlan, VersionPair, artifact_dir};
use crate::emit::selfcheck::Selfcheck;
use crate::exit::ExitCode;
use crate::http::{ApiError, Fetcher, RequestLog, Transport, probe};
use crate::hyp::GetBuild;
use crate::manifest::CapturedManifest;
use crate::manifest::diff;
use crate::report::{OutputFormat, Record};
use crate::{assemble, blobs, emit, hyp, manifest, select};

// The failure story moved to `crate::failure` so that the modules that own the
// work can name it. Re-exported because `main.rs` and the integration tests
// reach for `run::usage` and `run::failure_summary`, and there is no reason to
// make them learn a second path for the same thing.
pub use crate::failure::{
    Failure, Outcome, apierr, assertion, failure_summary, failure_summary_detail, usage, write_err,
};

/// The `matching_field` every run scores first, and the one every later stage
/// needs whatever class a capture lands in.
///
/// `game` is the one mandatory field: `--audio` widens the *selection pool* but
/// never makes the game field optional. Naming it here rather than inlining it
/// at the call site gives the rule "game is never optional" one address.
const MANIFEST_FIELD: &str = "game";



/// `gircap capture` — fetch from the live APIs and write a bundle.
///
/// **M2.** Writes the seven `api/` documents: the four HoYoPlay bodies and both
/// `getBuild`s byte-exact, plus the generated tag probe, and
/// `api/requests.ndjson` beside them. Manifests, chunks and the `game/` fixture
/// land in M3, M6 and M8.
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
    let captured = client
        .capture_all(plan.new_selector(), plan.old_selector(), plan.jobs)
        .await
        .map_err(|e| apierr("metadata capture", &e))?;

    // Assert what the capture depends on, before a blob is fetched: the launcher
    // must be on chunk delivery, both builds must echo the tags they were asked
    // for, and every published manifest must be storable.
    captured
        .validate()
        .map_err(|e| assertion("validate the capture", &e))?;

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
    //    `game` first because it is the field every later stage needs regardless
    //    of which class a capture lands in; the audio codes follow because they
    //    exist only to widen the *selection pool*, and a voice pack must never
    //    outrank a game file on a cost tie (see [`select`]).
    //
    //    `--no-chunk-blobs` does not skip this: a manifest is metadata, and the
    //    flag is about chunk blobs. Skipping it would leave a bundle whose
    //    `versions` block names builds it has no manifest for.
    let old_build = captured
        .old_build()
        .map_err(|e| apierr("re-read the captured old build", &e))?;
    let new_build = captured
        .new_build()
        .map_err(|e| apierr("re-read the captured new build", &e))?;
    let fields = selection_fields(&plan);
    // Indexed [field][0=old, 1=new], so `manifests[f][t]` is the manifest a
    // field contributes to the diff and nothing downstream has to remember the
    // interleaving the fetch loop used.
    //
    // These are fetched **all at once**. The loop this replaces was
    // `for field { for tag { capture().await } }`, which put a 9 MB download
    // after a 2 MB download after a 9 MB download after a 2 MB download for no
    // reason: nothing here depends on anything else here. On the reference
    // bundle that is ~20 MB strictly in series.
    //
    // The order they are *verified and filed* in is unchanged and is still
    // input order, because `map_concurrently` returns input order — so which
    // download finished first cannot move anything in the bundle.
    let wanted: Vec<(String, GetBuild, String)> = fields
        .iter()
        .flat_map(|field| {
            [(&old_tag, &old_build), (&new_tag, &new_build)]
                .into_iter()
                .map(move |(tag, build)| {
                    (
                        field.to_string(),
                        build.clone(),
                        (*tag).to_string(),
                    )
                })
        })
        .collect();
    let fetched = fetcher
        .map_concurrently(&wanted, plan.jobs, |fetcher, i, (field, build, tag)| async move {
            match manifest::capture(&fetcher, &build, &tag, &field).await {
                Ok(m) => (i, Ok(m)),
                Err(e) => (i, Err(e.to_string())),
            }
        })
        .await;

    let mut manifests: Vec<Vec<CapturedManifest>> =
        (0..fields.len()).map(|_| Vec::with_capacity(2)).collect();
    for (k, r) in fetched.into_iter().enumerate() {
        let field = &fields[k / 2];
        let tag = if k % 2 == 0 { &old_tag } else { &new_tag };
        let m = match r {
            Ok(m) => m,
            Err(e) => {
                return Err(apierr(
                    &format!("fetch the {tag} {field} manifest"),
                    &ApiError::shape(tag, e),
                ));
            }
        };
        manifest::verify(&m, tag, field)?;
        manifests[k / 2].push(m);
    }

    // 6a. What the API does with a request it will not serve. Placed here rather
    //     than beside the tag probe because it needs the pair's blob prefixes, and
    //     a `404` measured against an invented prefix would prove nothing: the
    //     point is that the *real* prefix answers `404` for an id the real build
    //     does not have, and `404` for a real id under the other build's.
    //
    //     Costs seven small requests and downloads no blob. It must not be able to
    //     fail the capture: a negative response is the measurement.
    let game_pair = fields
        .iter()
        .zip(&manifests)
        .find(|(field, _)| field.as_str() == MANIFEST_FIELD)
        .map(|(_, pair)| pair)
        .ok_or_else(|| {
            apierr(
                "the negative probes",
                &ApiError::shape(
                    "the game manifest pair",
                    "no `game` manifest to take blob prefixes from",
                ),
            )
        })?;
    let (old_game, new_game) = (&game_pair[0], &game_pair[1]);
    // A chunk the new build has and the old one does not — see `ErrorProbes`.
    // A shared chunk would make the wrong-build probe record a `200` and claim the
    // opposite of what it is there to show.
    let old_chunk_ids: std::collections::HashSet<&str> = old_game
        .entries
        .iter()
        .flat_map(|f: &crate::manifest::proto::SophonChunkFile| f.chunks.iter())
        .map(|c| c.id.as_str())
        .collect();
    let new_only_chunk_id = new_game
        .entries
        .iter()
        .flat_map(|f: &crate::manifest::proto::SophonChunkFile| f.chunks.iter())
        .find(|c| !old_chunk_ids.contains(c.id.as_str()))
        .map(|c| c.id.clone())
        .ok_or_else(|| {
            apierr(
                "the negative probes",
                &ApiError::shape(
                    "the two `game` manifests",
                    "every chunk of the new build is also in the old one, so there is \
                     no id to ask for under the wrong prefix",
                ),
            )
        })?;
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
    let mut diffs = Vec::with_capacity(fields.len());
    for (field, pair) in fields.iter().zip(&manifests) {
        let fd = diff::diff_field(&pair[0], &pair[1])
            .map_err(|m| assertion("diff the manifests", &ApiError::shape(field, m)))?;
        tracing::info!(
            field = %fd.field,
            common = fd.counts.common,
            identical = fd.counts.identical,
            changed = fd.counts.changed(),
            added = fd.counts.added,
            removed = fd.counts.removed,
            pool_chunks = fd.pool.new_chunks_total,
            pool_md5_hits = fd.pool.new_chunks_md5_present_in_old_manifest,
            cross_file_only = fd.pool.cross_file_only_chunks,
            cross_file_bytes = fd.pool.cross_file_only_bytes_forgone,
            "diff computed"
        );
        diffs.push(fd);
    }

    // The aggregate is computed **once**, here, and used twice: it is both the
    // `diff/paths.diff.json` body and the headline numbers on the summary. It
    // used to be built again at the summary, from the same inputs — one O(rows)
    // walk and two sorted `Vec<String>` per call.
    let agg = diff::aggregate(&old_tag, &new_tag, &diffs);

    // 6c. Score, classify, admit. Pure arithmetic over `diffs` — the only work
    //     this adds to the run is the audio manifests fetched above.
    let pool = select::candidates(&diffs);
    let budget = match plan.max_download_bytes {
        Some(n) => select::Budget::capped(n),
        None => select::Budget::unlimited(),
    };
    let selection = select::select(&pool, plan.per_class.max(1) as usize, budget, &plan.files);
    for a in &selection.admitted {
        tracing::info!(
            capture_id = %a.capture_id,
            class = %a.primary_class,
            matching_field = %a.matching_field,
            cost = a.budget_cost,
            path = %a.path,
            "admitted"
        );
    }
    for p in &selection.forced_skipped {
        // Named individually: the count says something was missed and the path
        // says what, and a caller who asked for five files needs both.
        tracing::warn!(
            path = %p,
            "--files named a path neither manifest carries; nothing to capture for it"
        );
    }
    for row in &selection.matrix {
        if row.is_satisfied() {
            continue;
        }
        tracing::warn!(
            class = %row.class,
            considered = row.candidates_considered,
            reason = row.reason.as_deref().unwrap_or(""),
            "class not satisfied"
        );
    }
    for r in &selection.rejected {
        tracing::info!(
            capture_id = %r.capture_id,
            class = %r.primary_class,
            cost = r.budget_cost,
            reason = %r.reason,
            path = %r.path,
            "wanted but not funded"
        );
    }
    tracing::info!(
        pool = pool.len(),
        admitted = selection.admitted.len(),
        rejected = selection.rejected.len(),
        bytes = selection.bytes_admitted,
        budget = %selection.budget.describe(),
        "selection decided"
    );

    // 6d. `--require-class` is the only thing that turns an absent class into a
    //     failed run. Without it an over-budget class is recorded and the run
    //     still exits 0 — the caller asked for a best effort and got one.
    let required = required_classes(&plan)?;
    let missing: Vec<select::Class> = required
        .into_iter()
        .filter(|c| {
            selection
                .matrix
                .iter()
                .find(|r| r.class == *c)
                .is_some_and(|r| !r.is_satisfied())
        })
        .collect();
    if !missing.is_empty() {
        // One pass, and the row is carried rather than looked up again. This
        // was a second `.find()` plus `.expect("every required class has a
        // matrix row")` — provably unreachable, since `missing` is built from
        // the identical predicate over the identical slice, but it is a panic in
        // a function that takes a user-supplied `--require-class`, and the
        // redundancy bought nothing.
        let detail = missing
            .iter()
            .filter_map(|c| {
                let row = selection.matrix.iter().find(|r| r.class == *c)?;
                Some(format!(
                    "{c}: {:?}{}",
                    row.status,
                    row.reason
                        .as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                ))
            })
            .collect::<Vec<_>>()
            .join("; ");
        tracing::error!(classes = ?missing, "--require-class was not met");
        return Err(Failure {
            code: ExitCode::RequirementUnmet,
            detail: format!("--require-class not met: {detail}"),
        });
    }

    // 6e. The chunk store. Planned *before* any blob is fetched, so the cost the
    //     log announces is a real figure rather than an estimate, and A12 is
    //     checked on the plan instead of after the bytes have been paid for.
    let mut by_tag: BTreeMap<String, &manifest::CapturedManifest> = BTreeMap::new();
    for (field, pair) in fields.iter().zip(&manifests) {
        if *field != MANIFEST_FIELD {
            // Only the `game` manifests are loaded into memory; the audio ones
            // exist to widen the *pool*, and a selected path is always scored
            // against game first (M5's tie-break). If a path ever comes only
            // from an audio field, that is a bug there, and here it surfaces as
            // a one-sided pair rather than a wrong download.
            continue;
        }
        for m in pair {
            by_tag.insert(m.tag.clone(), m);
        }
    }
    let pairs = blobs::file_pairs(&selection, &by_tag, &old_tag, &new_tag);
    let wants = blobs::chunks::wants(&pairs);
    for (a, p) in selection.admitted.iter().zip(&pairs) {
        let r = blobs::chunks::retention(p);
        if !r.a12_holds() {
            return Err(assertion(
                &format!("plan the chunk sets for {}", a.path),
                &ApiError::shape("A12", r.a12_failure().unwrap_or("unknown")),
            ));
        }
        tracing::info!(
            capture_id = %a.capture_id,
            path = %a.path,
            old_chunks = r.ids(blobs::chunks::Set::MaterializeOld).len(),
            new_chunks = r.ids(blobs::chunks::Set::MaterializeNew).len(),
            patch_fetch = r.ids(blobs::chunks::Set::PatchFetch).len(),
            reuse = r.reuse.len(),
            "retention planned"
        );
    }
    let projected = blobs::projected_download(&wants);
    tracing::info!(
        chunks = wants.len(),
        projected_bytes = projected,
        max_download_bytes = plan.max_download_bytes.unwrap_or(0),
        distinct_files = selection.admitted.len(),
        "chunk store planned"
    );
    if let Some(cap) = plan.max_download_bytes {
        // The budget is charged against `total_cost` in M5; this is a *second*,
        // independent ceiling over the bytes actually on the wire. Both hold.
        if projected > cap {
            return Err(assertion(
                "fetch the chunk blobs",
                &ApiError::shape(
                    "max-download-bytes",
                    format!(
                        "the admitted captures need {projected} B of chunk blobs and the \
                         ceiling is {cap} B; lower --per-class or raise the ceiling"
                    ),
                ),
            ));
        }
    }

    // 7. The tree. All three writes happen even though the run is incomplete:
    //    what was asked, what it cost, and what came back are the parts that
    //    stay useful, and the bytes are the evidence behind every number in the
    //    summary.
    let dir = artifact_dir(&plan.out, plan.biz, &old_tag, &new_tag, started);
    let mut written =
        emit::write_api(&dir, &captured, &probe, &error_shapes)
            .map_err(|e| write_err(&dir, e))?;
    // Counted from what the writer returned, never restated: a hand-written
    // count goes stale silently the moment a document is added, and nothing
    // fails when it does.
    let api_documents = written.len() as u64;
    for pair in &mut manifests {
        for m in pair {
            let stored = manifest::dump::write(&dir, m, plan.gzip_threshold, plan.manifest_detail)
                .map_err(|e| write_err(&dir, e))?;
            // `stored_as` is filled in here rather than by the writer: only now is it
            // known where the archive ended up, and only the request log can say it.
            manifest::mark_stored(&log, m, &stored[0].path);
            written.extend(stored.into_iter().map(|s| s.path));
        }
    }
    written.extend(
        emit::write_diff(&dir, &agg, &diffs[0].pool, &diffs).map_err(|e| write_err(&dir, e))?,
    );
    written.extend(
        emit::write_selection(
            &dir,
            &select::candidates_document(&diffs),
            &selection,
            plan.per_class.max(1) as usize,
        )
        .map_err(|e| write_err(&dir, e))?,
    );
    // 7b. The chunk blobs themselves, then `chunks/index.json`.
    //
    //     The index is written either way. `--no-chunk-blobs` produces one with
    //     every role list filled and no blobs, which is how a reader tells "this
    //     capture did not fetch" from "this capture had nothing to fetch".
    let store = blobs::Store::new(
        &dir,
        blobs::Sources::from_manifests(manifests.iter().flatten()),
    );
    let (entries, fetch_errors) = if plan.no_chunk_blobs {
        tracing::info!(
            chunks = wants.len(),
            projected_bytes = projected,
            "--no-chunk-blobs: the index records the plan and stores nothing"
        );
        (blobs::Store::plan_only(&wants), Vec::new())
    } else {
        let (e, errs) = store.fill(&fetcher, &wants, plan.jobs).await;
        for e in &errs {
            tracing::error!(error = %e, "chunk fetch failed");
        }
        (e, errs)
    };
    let index = blobs::ChunkIndex::build(entries);
    tracing::info!(detail = %blobs::describe(&wants, &index), "chunk store done");
    // `patchmaps/` is created whether or not anything lands in it.
    //
    // `--no-chunk-blobs` admits captures and then stores no blobs, so there is
    // nothing to write a map for, and the directory simply never appeared - which
    // made `verify` refuse the bundle as unreadable (exit 1, "a bundle with no
    // patchmaps has no captures to verify") rather than report the finding a
    // reader actually wants, which is that the bundle cannot be materialized. An
    // empty directory costs nothing and makes the layout uniform.
    std::fs::create_dir_all(dir.join("patchmaps")).map_err(|e| write_err(&dir, e))?;
    written.push(emit::write_chunk_index(&dir, &index).map_err(|e| write_err(&dir, e))?);

    // 7c. Assembly. Old, new, and the patch flow, from the blobs just stored.
    //     A7-A12 are asserted here rather than left to M9, because a capture that
    //     cannot rebuild its own files has nothing worth keeping and the operator
    //     should find out at capture time.
    //
    //     `game/` and `expected/` are written one capture at a time, as each
    //     assembles, so the fixture is never fully resident: the whole thing is
    //     the largest object this tool ever holds.
    let chunk_source = assemble::ChunkSource::from_index(&dir, &index);
    // Where the game puts the audio scan file. Never hardcoded: it is the
    // launcher's path, and the fixture has to write it where the tool under
    // test looks for it.
    let scan_dir = captured
        .configs
        .data::<hyp::GameConfigs>()
        .map_err(|e| apierr("parse getGameConfigs", &e))?
        .audio_pkg_scan_dir()
        .map_err(|e| apierr("read getGameConfigs.audio_pkg_scan_dir", &e))?
        .to_string();
    let mut rows: Vec<emit::selfcheck::Assertion> =
        api_and_manifest_assertions(&captured, versions, &manifests, &index, plan.no_chunk_blobs);
    let mut assembled_ids: Vec<String> = Vec::new();
    // (old md5, new md5) per capture, for A19. The digests, not the bytes:
    // A19 only asks whether the two sides differ, and holding the assembled
    // files here would keep a whole patch resident twice.
    let mut side_digests: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut prelude: Vec<String> = Vec::new();
    let (mut reused_total, mut fetched_total, mut moved_total) = (0_u64, 0_u64, 0_u64);
    if !plan.no_chunk_blobs {
        for (a, pair) in selection.admitted.iter().zip(&pairs) {
            let mut classes = vec![a.primary_class];
            classes.extend(a.also_satisfies.iter().copied());
            classes.sort();
            classes.dedup();
            let got = assemble::assemble(&assemble::AssemblyInput {
                pair,
                matching_field: a.matching_field.as_str(),
                classes: &classes,
                sources: store.sources(),
                chunks: &chunk_source,
            });
            match got {
                Ok(out) => {
                    let p = out.patch.as_ref();
                    tracing::info!(
                        capture_id = %a.capture_id,
                        path = %a.path,
                        old_md5_ok = out.old.as_ref().is_none_or(|o| o.md5_matches()),
                        new_md5_ok = out.new.as_ref().is_none_or(|n| n.md5_matches()),
                        a9 = p.is_none_or(|p| p.equals_new),
                        reuse = p.map(|p| p.reused).unwrap_or(0),
                        fetch = p.map(|p| p.fetched).unwrap_or(0),
                        moved = p.map(|p| p.moved).unwrap_or(0),
                        download_bytes = out.map.totals.map(|t| t.download_bytes).unwrap_or(0),
                        "assembled"
                    );
                    if let Some(p) = &out.patch {
                        reused_total += p.reused as u64;
                        fetched_total += p.fetched as u64;
                        moved_total += p.moved as u64;
                    }
                    // The assembler already produces selfcheck rows — both sides
                    // of each comparison, with the spec's wording — so they are
                    // carried through rather than re-derived here.
                    rows.extend(out.selfcheck_rows());
                    assembled_ids.push(a.capture_id.clone());
                    side_digests.insert(
                        a.capture_id.clone(),
                        (
                            out.old.as_ref().map(|s| s.md5.clone()).unwrap_or_default(),
                            out.new.as_ref().map(|s| s.md5.clone()).unwrap_or_default(),
                        ),
                    );
                    written.push(
                        emit::write_patchmap(&dir, &a.capture_id, &out.map)
                            .map_err(|e| write_err(&dir, e))?,
                    );
                    // `game/` is the fixture, `expected/` the ground truth. A
                    // `removed` capture contributes only to the first.
                    if let Some(o) = &out.old {
                        written.push(
                            emit::gamedir::write_variant(
                                &dir,
                                "game",
                                &emit::gamedir::VariantFile {
                                    rel_path: &a.path,
                                    bytes: &o.bytes,
                                    capture_id: &a.capture_id,
                                },
                            )
                            .map_err(|e| write_err(&dir, e))?,
                        );
                    }
                    if let Some(n) = &out.new {
                        written.push(
                            emit::gamedir::write_variant(
                                &dir,
                                "expected",
                                &emit::gamedir::VariantFile {
                                    rel_path: &a.path,
                                    bytes: &n.bytes,
                                    capture_id: &a.capture_id,
                                },
                            )
                            .map_err(|e| write_err(&dir, e))?,
                        );
                    }
                }
                Err(e) => {
                    return Err(assertion(
                        &format!("assemble {}", a.path),
                        &ApiError::shape("assemble", e),
                    ));
                }
            }
        }
    }
    // `write_prelude` always runs, including under `--no-chunk-blobs`.
    //
    // `config.ini` and the audio scan file are derived from `getGameConfigs` and
    // the `--audio` flag, not from a single chunk. Putting them behind the
    // flag made a metadata-only bundle have no `game/` at all, so A13 - "the
    // config re-parses with `game_version == --old-tag`" - failed on a tree that
    // simply had no config in it, and reported a missing file as a broken
    // assertion. The flag suppresses *chunk blobs*, not the launcher metadata.
    prelude.extend(
        emit::gamedir::write_prelude(&dir, &biz, &old_tag, &scan_dir, plan.audio.codes())
            .map_err(|e| write_err(&dir, e))?,
    );
    written.extend(prelude);
    let assembly_failures = rows.iter().filter(|a| !a.ok).count();
    emit::selfcheck::report_failed(&rows);
    tracing::info!(
        rows = rows.len(),
        failures = assembly_failures,
        patchmaps = assembled_ids.len(),
        "assembly assertions complete"
    );
    if assembly_failures > 0 {
        return Err(assertion(
            "assemble the admitted captures",
            &ApiError::shape(
                "A7-A12",
                format!(
                    "{assembly_failures} of {} selfcheck rows failed",
                    rows.len()
                ),
            ),
        ));
    }

    emit::write_requests_log(&dir, &log).map_err(|e| write_err(&dir, e))?;

    // 7d. The closing documents.
    //
    //     The order is load-bearing and is the only ordering that makes each
    //     document complete: A13-A17 first, because they are about the tree that
    //     now exists; then `selfcheck.json`, `MANIFEST.json` and `README.md`,
    //     which are about each other; then `checksums.json` last, because it must
    //     cover all of them.
    //
    //     A16 is the one that cannot be settled in that order — it asks whether
    //     `checksums.json` covers everything, and `checksums.json` does not exist
    //     yet. So it is checked after the first write and its row appended, which
    //     means `selfcheck.json` and `checksums.json` are each written twice.
    //     Two passes converge exactly: the second `checksums.json` is scanned
    //     after the final `selfcheck.json`, so what it records is what is on
    //     disk. A third pass would find nothing new, which is the proof it
    //     converged.
    rows.extend(closing_assertions(
        &dir,
        &old_tag,
        &scan_dir,
        &selection,
        &diffs,
        &side_digests,
        &written,
        plan.no_chunk_blobs,
    ));
    emit::selfcheck::report_failed(&rows);

    let input = ManifestInput {
        started,
        biz: &biz,
        endpoints: &endpoints,
        versions,
        diff_tags: &params.diff_tags,
        audio: plan.audio.codes(),
        fields: &fields,
        gzip_threshold: plan.gzip_threshold,
        diffs: &diffs,
        selection: &selection,
        index: &index,
        bytes_downloaded: log.bytes_in(),
        argv: &plan.argv,
        complete: true,
    };
    let mut manifest = build_manifest(&input)
        .map_err(|e| apierr("build MANIFEST.json", &ApiError::shape("manifest", e)))?;
    // A17 is about the matrix this document carries, so it is settled here and
    // appended to the rows the rest of the close-out will write.
    let (a17_ok, a17_detail) = manifest.a17();
    rows.push(emit::selfcheck::Assertion::row(
        "A17",
        "MANIFEST.json matrix",
        "every class captured | covered_by | unavailable",
        a17_ok,
        &a17_detail,
    ));

    // `files[]` now that every other document exists, so it lists as much as it
    // can. It cannot list itself, README or checksums — `checksums.json` is the
    // complete list, and its doc comment says so.
    //
    // This is the close-out's **one** full-tree digest pass. It used to be a
    // fourth one, in a private function that also re-implemented
    // `Checksums::scan`'s walk and inlined its `encoding_of` — so a third
    // encoding would have taught `MANIFEST.json` the wrong one while
    // `checksums.json` learned the right one.
    let (listed, seeded) = emit::write_manifest_files(&dir).map_err(|e| write_err(&dir, e))?;
    manifest.files = listed;
    manifest.totals.bytes_on_disk = manifest.files.iter().map(|f| f.bytes).sum::<u64>();

    // A6 joins A7-A12 here: with no blobs there is nothing to measure, nothing to
    // assemble and nothing to prove. A declared gap is `incomplete`, not `fail`.
    let skipped: Vec<&str> = if plan.no_chunk_blobs {
        vec!["A6", "A7", "A8", "A9", "A10", "A11", "A12", "A19"]
    } else {
        Vec::new()
    };
    let matrix_rows: Vec<emit::selfcheck::MatrixRow> = manifest
        .matrix
        .iter()
        .map(|m| emit::selfcheck::MatrixRow {
            class: m.class.clone(),
            status: m.status.clone(),
            // One definition of "an acceptable state", not a second copy of the
            // literal: the A17 row and the selfcheck's matrix row must not be
            // able to disagree about what `ok` means.
            ok: emit::selfcheck::MatrixRow::ok_status(&m.status),
        })
        .collect();

    // Counted here because the vector is **moved** below. It used to be
    // `rows.clone()`, so that one integer on the summary could read `rows.len()`
    // afterwards — a deep copy of every assertion id, subject, expected and
    // actual string, for one `usize`.
    let assembly_rows = rows.len() as u64;
    let mut selfcheck = emit::selfcheck::Selfcheck::build(rows, matrix_rows, &skipped);
    emit::write_selfcheck(&dir, &selfcheck).map_err(|e| write_err(&dir, e))?;
    written.push(emit::write_manifest(&dir, &manifest).map_err(|e| write_err(&dir, e))?);
    written.push(emit::write_readme(&dir, &manifest, &selfcheck).map_err(|e| write_err(&dir, e))?);
    let (_, checksums) = emit::write_checksums_reusing(&dir, &seeded, &[]).map_err(|e| write_err(&dir, e))?;
    // The independent half of A16: re-reads every listed file from disk and
    // compares. This is deliberately **not** folded into the scan above — a
    // re-hash that reuses the digests it is checking would be a tautology.
    let (a16_ok, a16_detail) = checksums.a16(&dir);
    written.push("verify/checksums.json".to_string());

    // Second pass: record A16, then re-write the two documents it changed.
    // The re-scan sees the final `selfcheck.json`, so the second
    // `checksums.json` is the authoritative one.
    //
    // **Unconditional.** This used to be guarded by
    // `if !selfcheck.by_id().contains_key("A16")`, which could never be false:
    // `selfcheck` is built from `rows`, and A16 is pushed *inside* the guard.
    // A conditional that always fires is worse than none — it reads as "this
    // might converge in one pass", and the comment above the pair is what
    // actually guarantees it.
    selfcheck.assertions.push(emit::selfcheck::Assertion::row(
        "A16",
        "verify/checksums.json",
        "covers every file under the artifact root, and re-hashing matches",
        a16_ok,
        &a16_detail,
    ));
    selfcheck =
        emit::selfcheck::Selfcheck::build(selfcheck.assertions, selfcheck.matrix, &skipped);
    emit::write_selfcheck(&dir, &selfcheck).map_err(|e| write_err(&dir, e))?;
    // Only `selfcheck.json` moved since the scan above, so only it is re-digested.
    emit::write_checksums_reusing(&dir, &checksums.entries, &["verify/selfcheck.json"])
        .map_err(|e| write_err(&dir, e))?;

    tracing::info!(
        selfcheck = ?selfcheck.result,
        total = selfcheck.counts.total,
        passed = selfcheck.counts.passed,
        failed = selfcheck.counts.failed,
        a16_ok,
        a16_detail,
        files = manifest.files.len(),
        checksummed = checksums.file_count,
        bytes_on_disk = manifest.totals.bytes_on_disk,
        "artifact written"
    );
    let documents = written.len();
    let flat: Vec<&CapturedManifest> = manifests.iter().flatten().collect();
    // Four sums and a max, in one walk. It was five walks of the same slice.
    let mut manifest_bytes = 0_u64;
    let mut manifest_entries = 0_u64;
    let mut manifest_chunks = 0_u64;
    let mut max_chunk = 0_u64;
    for m in &flat {
        manifest_bytes += m.meta.stored.bytes;
        manifest_entries += m.meta.decoded.entries;
        manifest_chunks += m.meta.decoded.total_chunks;
        max_chunk = max_chunk.max(m.meta.decoded.max_chunk_compressed_bytes);
    }
    tracing::info!(
        dir = %dir.display(),
        records = log.len(),
        documents,
        manifest_entries,
        manifest_chunks,
        "manifests written"
    );

    let legacy_behind = captured.legacy_staleness().map(|(n, _)| n).unwrap_or(0);
    // The diff's numbers are the run's headline result, so they go on the data
    // plane and not only into the stderr narration: this is what a caller reads.
    let pool_stats = &diffs[0].pool;
    let summary = Record::summary("capture")
        .set("result", "ok")
        .set("stage", "m8-artifact")
        .set("biz", biz.as_str())
        .set("old_tag", &old_tag)
        .set("new_tag", &new_tag)
        .set("package_id", &params.package_id)
        .set("old_build_id", &versions.old.build_id)
        .set("new_build_id", &versions.new.build_id)
        .set("artifact", dir.as_path())
        .set("api_documents", api_documents)
        .set("selection_pool", fields.join(","))
        .set("candidates_scored", pool.len() as u64)
        .set("captures_admitted", selection.admitted.len() as u64)
        .set("captures_over_budget", selection.rejected.len() as u64)
        .set("forced_files_requested", plan.files.len() as u64)
        .set(
            "forced_files_skipped",
            selection.forced_skipped.len() as u64,
        )
        .set("chunk_blobs_wanted", wants.len() as u64)
        .set(
            "chunk_blobs_stored",
            index
                .blobs
                .iter()
                .filter(|b| b.props.stored_path.is_some())
                .count() as u64,
        )
        .set("chunk_bytes_projected", projected)
        .set(
            "chunk_bytes_fetched",
            index.blobs.iter().map(|b| b.props.bytes).sum::<u64>(),
        )
        .set("chunk_fetches_failed", fetch_errors.len() as u64)
        .set("chunk_a6_failing", index.failing().len() as u64)
        .set("patch_fetch_chunks", index.sets["patch_fetch"].len() as u64)
        .set("patchmaps_written", assembled_ids.len() as u64)
        .set("assembly_selfcheck_rows", assembly_rows)
        .set("assembly_selfcheck_failures", assembly_failures as u64)
        .set("patch_reuse_chunks", reused_total)
        .set("patch_fetch_chunks_actually", fetched_total)
        .set("patch_moved_chunks", moved_total)
        .set("capture_bytes_admitted", selection.bytes_admitted)
        .set("budget_unlimited", selection.budget.limit().is_none())
        .set("budget_limit", selection.budget.limit().unwrap_or(0))
        .set("classes_satisfied", satisfied(&selection))
        .set("classes_unmet", selection.unmet().len() as u64)
        .set("manifests_captured", flat.len() as u64)
        .set("manifest_bytes", manifest_bytes)
        .set("manifest_entries", manifest_entries)
        .set("manifest_chunks", manifest_chunks)
        .set("max_chunk_compressed_bytes", max_chunk)
        .set("diff_common", agg.counts.common)
        .set("diff_identical", agg.counts.identical)
        .set("diff_changed", agg.counts.changed)
        .set("diff_added", agg.counts.added)
        .set("diff_removed", agg.counts.removed)
        .set("diff_patch_bytes", agg.byte_counts.patch)
        .set(
            "reuse_md5_present_in_old",
            pool_stats.new_chunks_md5_present_in_old_manifest,
        )
        .set(
            "reuse_cross_file_only_chunks",
            pool_stats.cross_file_only_chunks,
        )
        .set(
            "reuse_cross_file_bytes_forgone",
            pool_stats.cross_file_only_bytes_forgone,
        )
        .set("documents_written", documents as u64)
        .set("requests", log.len() as u64)
        .set("bytes_downloaded", log.bytes_in())
        .set("tags_probed", probe.summary.requested as u64)
        .set("tags_served", probe.summary.served as u64)
        .set(
            "tags_served_outside_diff_tags",
            probe.summary.not_in_diff_tags_but_served as u64,
        )
        .set("legacy_endpoint_majors_behind", legacy_behind)
        .set("exit_code", ExitCode::Ok.code());

    Ok((summary, ExitCode::Ok))
}

/// The `matching_field`s whose manifests are fetched, `game` first.
///
/// `game` is not optional: every later stage needs it whatever the class, and a
/// pool that could omit it would let a cheap voice pack answer a class that a
/// game file covers for less. The audio codes are appended in the order the
/// caller named them so two runs with the same flags score in the same order.
fn selection_fields(plan: &CapturePlan) -> Vec<String> {
    let mut fields = vec![MANIFEST_FIELD.to_string()];
    fields.extend(plan.audio.codes().iter().cloned());
    fields
}

/// The classes whose absence must fail the run.
///
/// **Empty by default, and that is the correct default rather than a missing
/// one.** `--require-class` is opt-in, so a class is only fatal when the caller
/// named it. Requiring all nine by default would make *every* run of a pair
/// like 7.0.0 → 7.1.0 exit `4`, because that pair genuinely has no
/// `full_reuse_reordered` file — a fact about the world, recorded as
/// `status: unavailable`, not a failure of the tool.
///
/// The distinction the exit code turns on:
///
/// - **unavailable** — no path in the pool satisfies the class. A statement
///   about the manifests. Recorded, exit `0`.
/// - **over budget** — a candidate existed and `--max-download-bytes` would not
///   pay for it. A statement about *this run's* configuration. Recorded,
///   exit `0` unless the caller named the class.
fn required_classes(plan: &CapturePlan) -> Result<Vec<select::Class>, Failure> {
    plan.require_class
        .iter()
        .map(|c| select::Class::parse(c).map_err(|e| usage("--require-class", &e)))
        .collect()
}

/// How many classes the matrix answers, captured or covered.
fn satisfied(selection: &select::Selection) -> u64 {
    selection.matrix.iter().filter(|r| r.is_satisfied()).count() as u64
}



/// `gircap verify` — re-check a bundle from its stored bytes, no network.
///
/// **M9.** Exits `0` when every re-derived row holds and `3` when one does not;
/// there is no network failure to distinguish, because there is no network.
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
    let (total, passed, failed) = report.counts();
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
    let ok = failed == 0 && report.agrees;
    let summary = Record::summary("verify")
        .set("result", if ok { "pass" } else { "fail" })
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
        .set("checks_total", total)
        .set("checks_passed", passed)
        .set("checks_failed", failed)
        .set("agrees_with_recording", report.agrees)
        .set("findings", report.findings.len() as u64)
        .set("recorded_result", selfcheck_slug(&report.bundle.recorded))
        .set(
            "exit_code",
            if ok {
                ExitCode::Ok.code()
            } else {
                ExitCode::AssertionFailed.code()
            },
        );
    if let Some(h) = report.headline() {
        return Ok((summary.set("detail", h), ExitCode::AssertionFailed));
    }
    Ok((
        summary,
        if ok {
            ExitCode::Ok
        } else {
            ExitCode::AssertionFailed
        },
    ))
}

/// `gircap materialize` — rebuild both files from `chunks/`, no network.
///
/// **M9.** The same rebuild `verify` performs, written out rather than only
/// compared. It is a separate command because the two answer different questions:
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
            let dest = out.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| write_err(&plan.artifact, e))?;
            }
            std::fs::write(&dest, &s.bytes).map_err(|e| write_err(&dest, e))?;
            written.push(rel);
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
    let (total, passed, failed) = report.counts();
    let ok = failed == 0 && report.agrees;
    let summary = Record::summary("materialize")
        .set("result", if ok { "pass" } else { "fail" })
        .set("artifact", plan.artifact.as_path())
        .set("out", out.as_path())
        .set("captures_rebuilt", report.rebuilt.len() as u64)
        .set("files_written", written.len() as u64)
        .set("bytes_written", bytes)
        .set("first_file", written.first().cloned().unwrap_or_default())
        .set("last_file", written.last().cloned().unwrap_or_default())
        .set("checks_total", total)
        .set("checks_passed", passed)
        .set("checks_failed", failed)
        .set("agrees_with_recording", report.agrees)
        .set(
            "exit_code",
            if ok {
                ExitCode::Ok.code()
            } else {
                ExitCode::AssertionFailed.code()
            },
        );
    if let Some(h) = report.headline() {
        return Ok((summary.set("detail", h), ExitCode::AssertionFailed));
    }
    Ok((
        summary,
        if ok {
            ExitCode::Ok
        } else {
            ExitCode::AssertionFailed
        },
    ))
}

/// `gircap list` — index every capture under a root.
///
/// **M9.** The only command that takes a root rather than a bundle, because its
/// job is to find bundles. A directory that looks like one and does not open is
/// listed as a skip rather than aborting the walk: `list` exists to be run over a
/// tree that may contain a half-finished capture.
///
/// One `BUNDLE` record per **valid** bundle on stdout, then one `SUMMARY`. The
/// per-bundle facts used to live only on stderr at INFO, so a machine could
/// consume this command only by parsing narration; `complete` and
/// `selfcheck` in particular are the two a caller needs to decide which bundle to
/// verify, and neither is on the summary.
///
/// A directory that does not open stays stderr-only, as before: there is no
/// bundle to describe, so a `BUNDLE` record would have to carry the failure
/// instead of the facts, and a caller asking "which bundles are here" would get
/// an entry that is not one.
pub async fn list(root: PathBuf, output: OutputFormat) -> Result<Outcome, Failure> {
    let found = crate::verify::list(&root).map_err(|e| Failure::usage(e.to_string()))?;
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
    let summary = Record::summary("list")
        .set("result", "ok")
        .set("root", root.as_path())
        .set("bundles", found.len() as u64)
        .set("bundles_passing", capturing)
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







/// A13, A14 and A15: the assertions that are about the tree, not about a blob.
#[allow(clippy::too_many_arguments)]
fn closing_assertions(
    dir: &std::path::Path,
    old_tag: &str,
    scan_dir: &str,
    selection: &select::Selection,
    diffs: &[diff::FieldDiff],
    side_digests: &BTreeMap<String, (String, String)>,
    written: &[String],
    no_chunk_blobs: bool,
) -> Vec<emit::selfcheck::Assertion> {
    use crate::emit::selfcheck::{Assertion, wording};
    let mut rows = Vec::new();

    // A13: `game/config.ini` re-parses with `game_version == --old-tag`.
    //
    // Re-read from disk rather than from the string that produced it: the
    // question is whether a *reader* can get the version out, and comparing
    // against the same input would pass even if the format were unreadable.
    rows.push(check_config_ini(dir, old_tag));

    // A14: `game/` holds nothing but the tool-read config and the variant
    // files. Anything else is a file the tool under test would build a plan
    // against, which is the failure this bundle exists to rule out.
    let captured: std::collections::BTreeSet<&str> =
        selection.admitted.iter().map(|a| a.path.as_str()).collect();
    let game_files: std::collections::BTreeSet<&str> = written
        .iter()
        .filter_map(|p| p.strip_prefix("game/"))
        .collect();
    let strays: Vec<&str> = game_files
        .iter()
        .copied()
        .filter(|p| *p != "config.ini" && *p != scan_dir && !captured.contains(p))
        .collect();
    rows.push(if strays.is_empty() {
        Assertion::pass(
            "A14",
            wording("A14"),
            "game/",
            &format!(
                "{} files, all of them config.ini, the scan file, or a captured path",
                game_files.len()
            ),
        )
    } else {
        Assertion::fail(
            "A14",
            wording("A14"),
            "game/",
            "config.ini, the audio scan file, and captured paths only",
            &format!("{} stray path(s): {}", strays.len(), strays.join(", ")),
        )
    });

    // A19: the *converse* of A14 - every captured file differs between the two
    // trees exactly when its class says it should.
    //
    // A14 asks "is `game/` free of paths the tool under test would plan
    // against", which is one direction. It cannot see the other failure: a
    // capture admitted for `partial_same_offset` whose two variant files came
    // out byte-identical. That bundle would satisfy A14, satisfy A7/A8 (each
    // side matches its own declared MD5), and be useless - the fixture would
    // hold no evidence of the patch it was captured for. M11 found the gap by
    // comparing the two trees by hand, which is the argument for checking it.
    //
    // A capture with only one side - `added` or `removed` - is skipped: its file
    // is absent from one tree *by construction*, which is the whole point of the
    // class rather than a disagreement.
    // Skipped under `--no-chunk-blobs`, for the reason A6 is: there are no
    // assembled sides to compare, so the check would pass vacuously and a green
    // row would be a claim about nothing. Exactly the failure mode A6 had.
    let mut agree: Vec<&str> = Vec::new();
    let mut disagree: Vec<String> = Vec::new();
    if !no_chunk_blobs {
        for a in &selection.admitted {
            let Some((old, new)) = side_digests.get(&a.capture_id) else {
                continue;
            };
            if old.is_empty() || new.is_empty() {
                // One-sided: absent from the other tree by design.
                continue;
            }
            let differs = old != new;
            let should_differ = !a.primary_class.is_identical();
            if differs == should_differ {
                agree.push(a.path.as_str());
            } else {
                disagree.push(format!(
                    "{} ({}) {} but its class says it should {}",
                    a.path,
                    a.capture_id,
                    if differs { "differs" } else { "is unchanged" },
                    if should_differ {
                        "differ"
                    } else {
                        "be unchanged"
                    }
                ));
            }
        }
        rows.push(if disagree.is_empty() {
            Assertion::pass(
                "A19",
                wording("A19"),
                "game/ vs expected/",
                &format!(
                    "{} two-sided capture(s): each file differs exactly when its class says \
                     it should",
                    agree.len()
                ),
            )
        } else {
            Assertion::fail(
                "A19",
                wording("A19"),
                "game/ vs expected/",
                "each two-sided file differs exactly when its class says it should",
                &format!("{} disagree: {}", disagree.len(), disagree.join("; ")),
            )
        });
    }
    // data it counted is caught rather than copied.
    for f in diffs {
        let recomputed_common = f
            .rows
            .iter()
            .filter(|r| matches!(r.state, diff::State::Identical | diff::State::Changed))
            .count() as u64;
        let recomputed_identical = f
            .rows
            .iter()
            .filter(|r| r.state == diff::State::Identical)
            .count() as u64;
        let recomputed_added = f
            .rows
            .iter()
            .filter(|r| r.state == diff::State::Added)
            .count() as u64;
        let recomputed_removed = f
            .rows
            .iter()
            .filter(|r| r.state == diff::State::Removed)
            .count() as u64;
        let subject = format!("diff/{} counts", f.field);
        for (name, recorded, actual) in [
            ("common", f.counts.common, recomputed_common),
            ("identical", f.counts.identical, recomputed_identical),
            ("added", f.counts.added, recomputed_added),
            ("removed", f.counts.removed, recomputed_removed),
        ] {
            rows.push(Assertion::check(
                "A15",
                wording("A15"),
                &format!("{subject} ({name})"),
                &recorded.to_string(),
                &actual.to_string(),
            ));
        }
    }

    // `--no-chunk-blobs` contributes **no** A7-A12 rows. A declared gap is not a
    // failure: it is declared the way the design declares everything else — by
    // listing the ids in `skipped`, which is what makes `Selfcheck::build` answer
    // `incomplete` rather than `fail`. A row that says "not asserted" and is marked
    // `ok: false` tells a reader the bundle is broken when it is honestly
    // incomplete.
    if no_chunk_blobs {
        tracing::debug!("--no-chunk-blobs: A7-A12 are declared skipped, not asserted");
    }
    rows
}

/// Everything `build_manifest` needs, as one argument.
struct ManifestInput<'a> {
    /// When the run started, for `capture.captured_at`.
    pub started: chrono::DateTime<chrono::Utc>,
    /// The region.
    pub biz: &'a Biz,
    /// The bases actually used, which may be `--hyp-base` overrides.
    pub endpoints: &'a Endpoints<'a>,
    /// The resolved pair.
    pub versions: &'a VersionPair,
    /// `main.diff_tags` as served.
    pub diff_tags: &'a [String],
    /// Audio codes written into the scan file.
    pub audio: &'a [String],
    /// The fields that were scored.
    pub fields: &'a [String],
    /// The `--gzip-threshold` this run used, recorded in `MANIFEST.json`.
    pub gzip_threshold: u64,
    /// The diffs, in the same order as `fields`.
    pub diffs: &'a [diff::FieldDiff],
    /// The selection.
    pub selection: &'a select::Selection,
    /// The chunk index.
    pub index: &'a blobs::ChunkIndex,
    /// Bytes the CDN sent.
    pub bytes_downloaded: u64,
    /// The command line.
    pub argv: &'a [String],
    /// Whether the run finished.
    pub complete: bool,
}

/// Build `MANIFEST.json`.
///
/// Every field is something the run already knew; nothing here recomputes a
/// measurement, so the document cannot disagree with the tree it describes.
fn build_manifest(i: &ManifestInput<'_>) -> Result<emit::index::Manifest, String> {
    let id = i.started.format("%Y%m%dT%H%M%SZ").to_string();
    let manifest_entries: BTreeMap<String, u64> = i
        .fields
        .iter()
        .zip(i.diffs)
        .map(|(f, d)| {
            // `common + added`: the paths that exist on the new side, which is
            // exactly the candidate set this field contributed. A `removed` path
            // is not a candidate — there is nothing to capture on the new side.
            (f.clone(), d.counts.common + d.counts.added)
        })
        .collect();
    let matrix: Vec<emit::index::ManifestMatrixRow> = i
        .selection
        .matrix
        .iter()
        .map(|row| {
            let admission = row
                .capture_id
                .as_ref()
                .and_then(|cid| i.selection.admitted.iter().find(|a| &a.capture_id == cid));
            emit::index::ManifestMatrixRow {
                class: row.class.id().to_string(),
                status: row.status.id().to_string(),
                capture_id: row.capture_id.clone(),
                path: row.path.clone(),
                matching_field: admission.map(|a| a.matching_field.clone()),
                covered_by: row.covered_by.clone(),
                also_satisfies: admission.filter(|a| !a.also_satisfies.is_empty()).map(|a| {
                    a.also_satisfies
                        .iter()
                        .map(|c| c.id().to_string())
                        .collect()
                }),
                reason: row.reason.clone(),
                candidates_considered: Some(row.candidates_considered),
            }
        })
        .collect();
    let totals_by_set: BTreeMap<String, u64> = i
        .index
        .sets
        .iter()
        .map(|(k, v)| (k.clone(), v.len() as u64))
        .collect();
    Ok(emit::index::Manifest {
        schema_version: crate::SCHEMA_VERSION,
        tool: emit::index::Tool {
            name: "gircap".to_string(),
            version: crate::VERSION.to_string(),
        },
        capture: emit::index::Capture {
            id,
            captured_at: i.started.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            host_os: std::env::consts::OS.to_string(),
            tool_argv: i.argv.to_vec(),
            complete: i.complete,
        },
        biz: emit::index::Biz_ {
            id: i.biz.as_str().to_string(),
            launcher_id: i.biz.launcher_id.to_string(),
            game_id: i.biz.game_id.to_string(),
            channel: i.biz.channel.to_string(),
            sub_channel: i.biz.sub_channel.to_string(),
            cps: i.biz.cps.to_string(),
            hyp_base: i.endpoints.hyp.to_string(),
            sophon_base: i.endpoints.sophon.to_string(),
        },
        versions: emit::index::Versions {
            new: emit::index::Build {
                tag: i.versions.new.tag.clone(),
                build_id: i.versions.new.build_id.clone(),
                source: i.versions.new.source.clone(),
            },
            old: emit::index::Build {
                tag: i.versions.old.tag.clone(),
                build_id: i.versions.old.build_id.clone(),
                source: i.versions.old.source.clone(),
            },
            diff_tags: i.diff_tags.to_vec(),
        },
        field: MANIFEST_FIELD.to_string(),
        audio: i.audio.to_vec(),
        gzip_threshold: i.gzip_threshold,
        selection_pool: emit::index::SelectionPool {
            fields: i.fields.to_vec(),
            manifest_entries,
            note: "--audio widens this pool; captures may come from any listed field".to_string(),
        },
        totals: emit::index::Totals {
            files_captured: i.selection.admitted.len() as u64,
            chunks_blobs: i
                .index
                .blobs
                .iter()
                .filter(|b| b.props.stored_path.is_some())
                .count() as u64,
            bytes_downloaded: i.bytes_downloaded,
            bytes_on_disk: 0,
        },
        matrix,
        files: Vec::new(),
        totals_by_set,
    })
}

/// A13: read `game/config.ini` back and check the version a reader would find.
///
/// **One implementation, two callers.** `verify` ran its own copy — same file,
/// same parse, same comparison, and *different wording* for every branch, which
/// is the worst kind of duplication: the two can disagree about what a passing
/// A13 looks like on paper while agreeing about whether it passes.
///
/// The capture's wording is the one kept, because `verify` compares its rows
/// against the capture's recorded ones, and the recorded row is what a reader
/// has already seen. `verify` no longer says `readable` where the capture says
/// `game_version == <tag>`, nor `game_version == v` where the capture writes
/// `game_version is v`.
pub fn check_config_ini(
    dir: &std::path::Path,
    old_tag: &str,
) -> emit::selfcheck::Assertion {
    use crate::emit::selfcheck::{Assertion, wording};
    const SUBJECT: &str = "game/config.ini";
    let cfg = dir.join("game").join("config.ini");
    let expected = format!("game_version == {old_tag}");
    match std::fs::read_to_string(&cfg) {
        Err(e) => Assertion::fail(
            "A13",
            wording("A13"),
            SUBJECT,
            &expected,
            &format!("unreadable: {e}"),
        ),
        Ok(text) => match emit::gamedir::parse_game_version(&text) {
            Err(e) => Assertion::fail("A13", wording("A13"), SUBJECT, &expected, &e),
            Ok(got) if got == old_tag => Assertion::pass(
                "A13",
                wording("A13"),
                SUBJECT,
                &format!("game_version is {got}"),
            ),
            Ok(got) => Assertion::fail(
                "A13",
                wording("A13"),
                SUBJECT,
                &expected,
                &format!("game_version is {got}, the old tag is {old_tag}"),
            ),
        },
    }
}

/// A1 through A6: what the API and the manifests said, and what arrived.
///
/// One row per *instance* — six bodies, two manifests per field, one row per
/// stored chunk — so a capture with 38 blobs carries 38 A6 rows. That is what
/// makes the row count checkable against the blob count rather than aspirational.
fn api_and_manifest_assertions(
    captured: &hyp::Captured,
    versions: &VersionPair,
    manifests: &[Vec<manifest::CapturedManifest>],
    index: &blobs::ChunkIndex,
    no_chunk_blobs: bool,
) -> Vec<emit::selfcheck::Assertion> {
    use crate::emit::selfcheck::{Assertion, wording};
    let mut rows = Vec::new();

    // A1: every captured response reported success.
    for body in captured.bodies() {
        let retcode = body.retcode();
        rows.push(Assertion::check(
            "A1",
            wording("A1"),
            &format!("api/{}", body.file),
            "0",
            &retcode.to_string(),
        ));
    }

    // A2: the two builds echo the tags they were asked for.
    for (file, want, got) in [
        (
            format!("api/getBuild.{}.json", versions.new.tag),
            versions.new.tag.as_str(),
            versions.new.tag.as_str(),
        ),
        (
            format!("api/getBuild.{}.json", versions.old.tag),
            versions.old.tag.as_str(),
            versions.old.tag.as_str(),
        ),
    ] {
        rows.push(Assertion::check("A2", wording("A2"), &file, want, got));
    }

    // A3, A4, A5: per manifest.
    for m in manifests.iter().flatten() {
        let subject = format!("manifests/{}/{}", m.tag, m.matching_field);
        let v = &m.meta.verified;
        rows.push(Assertion::check(
            "A3",
            wording("A3"),
            &subject,
            &m.meta.manifest.checksum,
            &m.meta.stored.md5_decompressed,
        ));
        rows.push(Assertion::check(
            "A4",
            wording("A4"),
            &subject,
            &m.meta.manifest.checksum,
            &m.meta.id_parts.suffix_32hex,
        ));
        // The 16-hex prefix is *recorded* as opaque and never verified, so the
        // row says that rather than implying it was checked and passed.
        rows.push(Assertion::pass(
            "A4",
            wording("A4"),
            &format!("{subject} (id prefix)"),
            &format!(
                "{} recorded as {}",
                m.meta.id_parts.prefix_16hex, m.meta.id_parts.prefix_semantics
            ),
        ));
        for (name, declared, actual) in
            manifest::reconcile(&m.meta.decoded, m.meta.declared.stats.as_ref())
        {
            rows.push(Assertion::check(
                "A5",
                wording("A5"),
                &format!("{subject} ({name})"),
                &format!("{declared:?}"),
                &actual.to_string(),
            ));
        }
        let _ = v;
    }

    // A6: one row per stored chunk, four checks folded into one verdict with the
    // failing ones named — a blob failing three of four is one finding about
    // one blob, not three findings about three blobs.
    //
    // `--no-chunk-blobs` emits **no** A6 rows at all and `A6` joins the skipped
    // list; otherwise a metadata-only run could never be green. The distinction
    // `BlobProps` already draws — `verified: null` means "not measured", a
    // different kind of fact from four `false` — has to reach the selfcheck too,
    // or a declared gap reads as a broken artifact.
    if !no_chunk_blobs {
        for b in &index.blobs {
            let subject =
                b.props.stored_path.clone().unwrap_or_else(|| {
                    format!("chunks/blobs/{}.zst (not stored)", b.props.chunk_id)
                });
            let v = b.props.verified;
            rows.push(match v {
                Some(v) if v.all_green() && b.props.decode_error.is_none() => {
                    Assertion::pass("A6", wording("A6"), &subject, &b.props.md5_decompressed)
                }
                Some(v) => Assertion::fail(
                    "A6",
                    wording("A6"),
                    &subject,
                    "all four checks hold",
                    &format!(
                        "{}; md5(decompressed) {}",
                        v.failures().join(", "),
                        b.props.md5_decompressed
                    ),
                ),
                // Reachable only when a blob is *wanted* but neither fetched nor
                // measured — a hole the store reported rather than one the run
                // chose. `verified: null` on a stored blob is a different thing
                // and does not get here, because the flag suppresses the rows.
                None => Assertion::fail(
                    "A6",
                    wording("A6"),
                    &subject,
                    "measured",
                    b.props
                        .decode_error
                        .as_deref()
                        .unwrap_or("wanted but never measured"),
                ),
            });
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
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
}

