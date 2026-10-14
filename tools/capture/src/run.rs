//! Command dispatch: the four entry points a caller can invoke, and the one
//! failure summary they share.
//!
//! Each returns a [`report::Summary`] plus the [`exit::ExitCode`] it earned, so
//! the shim in `main.rs` holds no logic: it dispatches, prints, exits. Commands
//! land here as their milestone arrives — `capture` in M8, the offline trio in
//! M9 — and until then each reports the honest fact that it has nothing to do
//! yet rather than pretending to have done it.
//!
//! `capture` is the exception that arrived early: M1 needs it end to end to
//! prove the transport, and the version pair has to be resolved to name the
//! bundle's directory at all.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::biz::{Biz, Endpoints};
use crate::config::{CapturePlan, MaterializePlan, VerifyPlan, artifact_dir};
use crate::exit::ExitCode;
use crate::http::{ApiError, Fetcher, RequestLog, Transport, probe};
use crate::manifest::CapturedManifest;
use crate::report::{OutputFormat, Summary};
use crate::{assemble, blobs, emit, exit, hyp, manifest, select};
use crate::manifest::diff;

/// The `matching_field` every run scores first, and the one every later stage
/// needs whatever class a capture lands in.
///
/// M5 widened the *selection pool* past this to `--audio`'s codes; it did not
/// change which field is mandatory. Keeping it named here rather than inlined
/// at the call site means the rule "game is never optional" has one address.
const MANIFEST_FIELD: &str = "game";

/// What a command hands back: its summary line, and the code to exit with.
///
/// The code travels with the summary rather than being returned separately so a
/// command cannot print a summary that disagrees with the status it exits with.
pub type Outcome = (Summary, ExitCode);

/// Why a command failed, in a form the summary can carry.
///
/// A code alone is not an answer. Exit `1` and exit `2` are both "something was
/// wrong" and the operator needs to know what — so the cause rides out on stdout
/// as a `detail=` field instead of living only in the stderr narration that a
/// `--output json` pipeline is not reading.
#[derive(Debug, Clone)]
pub struct Failure {
    /// What to exit with.
    pub code: ExitCode,
    /// The one-line cause.
    pub detail: String,
}

impl Failure {
    /// A failure discovered by validating the request. Still exit `1`: the
    /// request was wrong, and nothing about it was the network's fault.
    pub fn usage(message: impl std::fmt::Display) -> Self {
        Failure {
            code: ExitCode::Usage,
            detail: message.to_string(),
        }
    }

    /// A failure of the network or the API. Always exit `2`.
    pub fn api(what: &str, e: &ApiError) -> Self {
        Failure {
            code: ExitCode::Network,
            detail: format!("{what}: {e}"),
        }
    }

    /// The summary a caller sees for this failure.
    pub fn summary(&self, event: &'static str) -> Summary {
        failure_summary_detail(event, self.code, Some(&self.detail))
    }
}

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
            jobs: plan.jobs,
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
            plan.hyp_base.clone().unwrap_or_else(|| biz.hyp_base().to_string()),
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
        .capture_all(plan.new_selector(), plan.old_selector())
        .await
        .map_err(|e| apierr("metadata capture", &e))?;

    // Assert what the capture depends on, before a blob is fetched: the launcher
    // must be on chunk delivery, both builds must echo the tags they were asked
    // for, and every published manifest must be storable.
    captured.validate().map_err(|e| assertion("validate the capture", &e))?;

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
    let mut manifests: Vec<Vec<CapturedManifest>> = Vec::with_capacity(fields.len());
    for field in &fields {
        let mut pair = Vec::with_capacity(2);
        for (tag, build) in [(&old_tag, &old_build), (&new_tag, &new_build)] {
            let m = manifest::capture(&fetcher, build, tag, field)
                .await
                .map_err(|e| apierr(&format!("fetch the {tag} {field} manifest"), &e))?;
            verify_manifest(&m, tag, field)?;
            pair.push(m);
        }
        manifests.push(pair);
    }

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

    // 6c. Score, classify, admit. Pure arithmetic over `diffs` — the only work
    //     this adds to the run is the audio manifests fetched above.
    let pool = select::candidates(&diffs);
    let budget = match plan.max_download_bytes {
        Some(n) => select::Budget::capped(n),
        None => select::Budget::unlimited(),
    };
    let selection = select::select(
        &pool,
        plan.per_class.max(1) as usize,
        budget,
        &plan.files,
    );
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
        let detail = missing
            .iter()
            .map(|c| {
                let row = selection
                    .matrix
                    .iter()
                    .find(|r| r.class == *c)
                    .expect("every required class has a matrix row");
                format!(
                    "{c}: {:?}{}",
                    row.status,
                    row.reason
                        .as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                )
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
    let mut written = emit::write_api(&dir, &captured, &probe).map_err(|e| {
        apierr(
            &format!("write {}", dir.display()),
            &ApiError::shape("artifact writer", e),
        )
    })?;
    for pair in &mut manifests {
        for m in pair {
            let stored = manifest::dump::write(&dir, m, plan.gzip_threshold, plan.manifest_detail)
                .map_err(|e| {
                    apierr(
                        &format!("write {}", dir.display()),
                        &ApiError::shape("artifact writer", e),
                    )
                })?;
            // `stored_as` is filled in here rather than by the writer: only now is it
            // known where the archive ended up, and only the request log can say it.
            manifest::mark_stored(&log, m, &stored[0].path);
            written.extend(stored.into_iter().map(|s| s.path));
        }
    }
    written.extend(
        emit::write_diff(
            &dir,
            &diff::aggregate(&old_tag, &new_tag, &diffs),
            &diffs[0].pool,
            &diffs,
        )
        .map_err(|e| {
            apierr(
                &format!("write {}", dir.display()),
                &ApiError::shape("artifact writer", e),
            )
        })?,
    );
    written.extend(
        emit::write_selection(
            &dir,
            &select::candidates_document(&diffs, &pool),
            &selection,
            plan.per_class.max(1) as usize,
        )
        .map_err(|e| {
            apierr(
                &format!("write {}", dir.display()),
                &ApiError::shape("artifact writer", e),
            )
        })?,
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
    written.push(
        emit::write_chunk_index(&dir, &index).map_err(|e| {
            apierr(
                &format!("write {}", dir.display()),
                &ApiError::shape("artifact writer", e),
            )
        })?,
    );

    // 7c. Assembly. Old, new, and the patch flow, from the blobs just stored.
    //     A7-A12 are asserted here rather than left to M9, because a capture that
    //     cannot rebuild its own files has nothing worth keeping and the operator
    //     should find out at capture time.
    let chunk_source = assemble::ChunkSource::from_index(&dir, &index);
    let mut rows: Vec<(&'static str, bool, String)> = Vec::new();
    let mut assembled_ids: Vec<String> = Vec::new();
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
                    rows.extend(out.selfcheck_rows());
                    assembled_ids.push(a.capture_id.clone());
                    written.push(
                        emit::write_patchmap(&dir, &a.capture_id, &out.map).map_err(|e| {
                            apierr(
                                &format!("write {}", dir.display()),
                                &ApiError::shape("artifact writer", e),
                            )
                        })?,
                    );
                }
                Err(e) => {
                    return Err(assertion(
                        &format!("assemble {}", a.path),
                        &ApiError::shape("assemble", e),
                    ));
                }
            }
        }
    } else {
        tracing::warn!("--no-chunk-blobs: nothing to assemble, so A7-A12 are not asserted");
    }
    let assembly_failures = rows.iter().filter(|(_, ok, _)| !*ok).count();
    for (assertion, _, detail) in rows.iter().filter(|(_, ok, _)| !*ok) {
        tracing::error!(assertion, detail, "selfcheck row failed");
    }
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
                format!("{assembly_failures} of {} selfcheck rows failed", rows.len()),
            ),
        ));
    }

    emit::write_requests_log(&dir, &log).map_err(|e| {
        apierr(
            &format!("write {}", dir.display()),
            &ApiError::shape("artifact writer", e),
        )
    })?;
    let documents = written.len();
    let flat: Vec<&CapturedManifest> = manifests.iter().flatten().collect();
    let manifest_bytes: u64 = flat.iter().map(|m| m.meta.stored.bytes).sum();
    let manifest_entries: u64 = flat.iter().map(|m| m.meta.decoded.entries).sum();
    let manifest_chunks: u64 = flat.iter().map(|m| m.meta.decoded.total_chunks).sum();
    tracing::info!(
        dir = %dir.display(),
        records = log.len(),
        documents,
        manifest_entries,
        manifest_chunks,
        "manifests written"
    );

    let legacy_behind = captured
        .legacy_staleness()
        .map(|(n, _)| n)
        .unwrap_or(0);
    let max_chunk = flat
        .iter()
        .map(|m| m.meta.decoded.max_chunk_compressed_bytes)
        .max()
        .unwrap_or(0);
    // The diff's numbers are the run's headline result, so they go on the data
    // plane and not only into the stderr narration: this is what a caller reads.
    let agg = diff::aggregate(&old_tag, &new_tag, &diffs);
    let pool_stats = &diffs[0].pool;
    let summary = Summary::new("capture")
        .set("result", "ok")
        .set("stage", "m5-selection")
        .set("biz", biz.as_str())
        .set("old_tag", &old_tag)
        .set("new_tag", &new_tag)
        .set("package_id", &params.package_id)
        .set("old_build_id", &versions.old.build_id)
        .set("new_build_id", &versions.new.build_id)
        .set("artifact", dir.as_path())
        .set("api_documents", 7_u64)
        .set("selection_pool", fields.join(","))
        .set("candidates_scored", pool.len() as u64)
        .set("captures_admitted", selection.admitted.len() as u64)
        .set("captures_over_budget", selection.rejected.len() as u64)
        .set("chunk_blobs_wanted", wants.len() as u64)
        .set("chunk_blobs_stored", index.blobs.iter().filter(|b| b.props.stored_path.is_some()).count() as u64)
        .set("chunk_bytes_projected", projected)
        .set("chunk_bytes_fetched", index.blobs.iter().map(|b| b.props.bytes).sum::<u64>())
        .set("chunk_fetches_failed", fetch_errors.len() as u64)
        .set("chunk_a6_failing", index.failing().len() as u64)
        .set("patch_fetch_chunks", index.sets["patch_fetch"].len() as u64)
        .set("patchmaps_written", assembled_ids.len() as u64)
        .set("assembly_selfcheck_rows", rows.len() as u64)
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
        .set("reuse_cross_file_only_chunks", pool_stats.cross_file_only_chunks)
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
    selection
        .matrix
        .iter()
        .filter(|r| r.is_satisfied())
        .count() as u64
}

/// The two things a decoded manifest must satisfy before any byte of it is
/// trusted: its decompressed MD5 matches the declared checksum, and the decoded
/// counts reconcile against the declared stats.
///
/// Both are exit `3`. The bytes arrived intact or they did not, and either way
/// nothing here is the network's fault — a checksum that fails is a fact about
/// what was published, and the capture cannot be built on top of it.
fn verify_manifest(m: &CapturedManifest, tag: &str, field: &str) -> Result<(), Failure> {
    if !m.meta.verified.all_green() {
        return Err(assertion(
            &format!("verify the {tag} {field} manifest"),
            &ApiError::shape(
                tag,
                format!(
                    "decompressed md5 matches checksum: {}, checksum matches id suffix: {}",
                    m.meta.verified.decompressed_md5_matches_checksum,
                    m.meta.verified.checksum_matches_id_suffix
                ),
            ),
        ));
    }
    let v = manifest::reconcile(&m.meta.decoded, m.meta.declared.stats.as_ref());
    if !v.is_empty() {
        return Err(assertion(
            &format!("reconcile the {tag} {field} manifest"),
            &ApiError::shape(
                tag,
                v.iter()
                    .map(|(n, d, a)| format!("{n}: declared {d:?}, decoded {a}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        ));
    }
    manifest::dump::note_reconciliation(m);
    if !m.meta.decoded.structure.clean() {
        // Recorded, not fatal: spec §3.6 says a violation must still produce
        // a capture with the selfcheck naming what broke. M8 makes it a row.
        tracing::warn!(
            tag,
            field,
            violations = ?m.meta.decoded.structure.violations(),
            "manifest violates a structural invariant; recorded, not fatal"
        );
    }
    Ok(())
}

/// `gircap verify` — re-check a bundle from its stored bytes, no network.
pub async fn verify(_plan: VerifyPlan) -> Result<Outcome, Failure> {
    tracing::warn!("verify is not implemented yet");
    Err(Failure { code: ExitCode::AssertionFailed, detail: "verify is not implemented yet".into() })
}

/// `gircap materialize` — rebuild both files from `chunks/`, no network.
pub async fn materialize(_plan: MaterializePlan) -> Result<Outcome, Failure> {
    tracing::warn!("materialize is not implemented yet");
    Err(Failure { code: ExitCode::AssertionFailed, detail: "materialize is not implemented yet".into() })
}

/// `gircap list` — index every capture under a root.
pub async fn list(_root: PathBuf, _output: OutputFormat) -> Result<Outcome, Failure> {
    tracing::warn!("list is not implemented yet");
    Err(Failure { code: ExitCode::AssertionFailed, detail: "list is not implemented yet".into() })
}

/// Turn an [`ApiError`] into exit `2`, having named it on stderr.
fn apierr(what: &str, e: &ApiError) -> Failure {
    tracing::error!(subject = what, error = %e, url = e.url().unwrap_or(""), "api failure");
    Failure::api(what, e)
}

/// See [`Failure::usage`]; this adds the narration, so the dispatch in `main.rs`
/// has one place to report a rejected request rather than three.
pub fn usage(what: &str, message: &str) -> Failure {
    tracing::error!(subject = what, error = %message, "usage error");
    Failure::usage(message)
}

/// An assertion on captured API state: the answer arrived and it says something
/// this capture cannot be built from.
///
/// Exit `3`, not `2`. The request worked — a `2` would blame the network for a
/// fact about the world — and not `1`, because the request was not wrong either.
/// What is wrong is the artifact, and `3` is the code that says so.
fn assertion(what: &str, e: &ApiError) -> Failure {
    tracing::error!(subject = what, error = %e, "assertion failed");
    Failure {
        code: ExitCode::AssertionFailed,
        detail: format!("{what}: {e}"),
    }
}

/// The summary a command prints when it failed before it could build one.
///
/// Every command prints a summary on every path, so a harness that only reads
/// stdout never has to distinguish "the tool said nothing" from "the tool
/// succeeded quietly". The fields are the ones that are true of *any* failure:
/// which command, which code, what it meant, and — for the ones that leave a
/// tree behind — whether that tree can be believed.
pub fn failure_summary(event: &'static str, code: ExitCode) -> Summary {
    failure_summary_detail(event, code, None)
}

/// [`failure_summary`] with the one-line cause, when there is one.
///
/// The cause is a field rather than prose, so the whole line stays
/// whitespace-free and splittable.
pub fn failure_summary_detail(
    event: &'static str,
    code: ExitCode,
    detail: Option<&str>,
) -> Summary {
    let s = Summary::new(event)
        .set("result", "error")
        .set("exit_code", code.code())
        .set("exit_code_slug", code.slug())
        .set("exit_code_meaning", code.describe())
        .set("artifact_trustworthy", !code.artifact_is_untrustworthy());
    match detail {
        Some(d) => s.set("detail", d),
        None => s,
    }
}

/// Re-exported so `main.rs` can name the process exit without importing two
/// modules for it.
pub use exit::exit_with;

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
        assert_eq!(s.get("exit_code_slug"), Some(&crate::report::Val::Str("network".into())));
        assert_eq!(s.get("result"), Some(&crate::report::Val::Str("error".into())));
        assert_eq!(s.get("detail"), None);

        let d = failure_summary_detail("capture", ExitCode::AssertionFailed, Some("boom"));
        assert_eq!(d.get("detail"), Some(&crate::report::Val::Str("boom".into())));
        assert_eq!(
            d.get("artifact_trustworthy"),
            Some(&crate::report::Val::Bool(false)),
            "an assertion failure leaves a tree that must be read, not deleted"
        );
    }

    #[test]
    fn the_detail_survives_the_text_rendering_as_one_token() {
        let line = failure_summary_detail("capture", ExitCode::Network, Some("getBuild: retcode -202 (not found)"))
            .to_text_line();
        assert_eq!(
            line.split_whitespace().filter(|t| t.contains('=')).count(),
            line.split_whitespace().count() - 1,
            "a spaced detail would break tokenisation: {line}"
        );
        assert!(line.contains("retcode%20-202"), "{line}");
    }
}
