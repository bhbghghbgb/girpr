# Version-upgrade stale-file sweep — pre-planning findings

**STATUS: investigation report, not an implementation spec.** Every finding below
was checked against the local sources and, where marked *live*, against the
production endpoints on **2026-11-25**. Server behaviour, response contents and
launcher sources change; **revalidate every point in this report before
committing to an implementation plan or writing code.** No product code was
changed by this investigation.

## Goal under consideration

Add an always-on deletion of stale files to `girpr`'s prerequisite phase, so a
repair run frees space before patching even when no purge flag is given. The
list is the union of the delete lists the server's `getPatchBuild` patch
manifests currently provide, filtered by the same rules both launchers use
(notably: a path that is in the live new-version manifest is never a target).
This is distinct from the existing extra-file purge, which is opt-in and
computes its candidates from a full install walk.

## Findings, with evidence

| Finding | Verdict | Evidence |
|---|---|---|
| `deprecated_files` = 18 legacy DLL/SYS/EXE names, no version field | Confirmed | `app/fetch/src/hyp.rs:268-272`; Starward `GameDeprecatedFile.cs:21-25` — only `name`. *Live:* global API returned exactly 18 entries (`UnityPlayer.dll`, `mhyprot2.Sys`, `blueReporter.exe`, …). |
| Collapse repair deletes legacy lists/residue; version-pair deletions are update-only | Confirmed | `RepairManagement/Genshin/Check.cs:233-314` (`*deletefiles*`, `.diff`/`_tmp`/`.hdiff` → `RepairAssetType.Unused`); `Hi3Helper.Sophon/SophonPatch.cs:430-453` (`UnusedAssets` → `SophonPatchMethod.Remove`), reachable only from `InstallManagerBase.Sophon.cs:674` → `InstallManagerBase.SophonPatch.cs:42`, with version-from from `GetGameExistingVersion()` (`:61-66`). Collapse also passes the patch assets as `compareWithList` to skip paths still present in the new-version manifest (`SophonPatch.cs:420,436-439`) and disables old-file removal when a past-version patch manifest is missing (`SophonPatch.cs:369-373`). |
| Starward `DeleteTags` come from a binary protobuf patch manifest; applied only in Update + Patch mode | Confirmed | `Starward.RPC/GameInstall/Sophon.proto:33-40,76-92` (`delete_tags{tag, delete_collection.delete_files{file,size,md5}}`). `GamePackageService.cs:322-341` populates them only for `Predownload/Update` with `GameSophonPatchBuild` (Patch mode) and selects `DeleteTags.FirstOrDefault(x => x.Tag == localVersion)`; `:361-376` removes deletes whose file is in `patches`. `GameInstallService.cs:508-519` deletes them inside `ExecuteUpdateTaskAsnyc`; `ExecuteRepairTaskAsync` (`:640+`) never does. Repair only runs `ClearDeprecatedFiles` (`:263`). |
| Both launchers use effectively the same server-produced patch-build protobuf | Confirmed | Starward `Sophon.proto` and Collapse `Hi3Helper.Sophon/Protos/SophonPatchProto.proto` describe the same wire layout. Live patch blobs decode with those tags. |
| `getPatchBuild` ignores `tag`; update-to is fixed to latest | Confirmed live | Starward `HoYoPlayClient.cs:595-612` and Collapse `PresetConfig.cs:199-207` send only `branch`/`package_id`/`password`. Live: no tag, `&tag=7.0.0`, `&tag=7.1.0` all returned `tag=7.2.0` and the same `build_id`; `getGameBranches.main.tag=7.2.0`, `diff_tags=[7.1.0,7.0.0]`. |
| "API provides only one further version back" | **Incorrect / outdated** | *Live:* the `game` patch manifest had **two** delete tags — `7.1.0` (29 files) and `7.0.0` (12 files). Audio manifests had none. Union = 41 entries / 40 distinct paths. |
| Manifest exemption has no current effect, but is load-bearing | Confirmed live | The 40 patch-delete paths had **zero** overlap with the 2,795 paths of the live `game` chunk manifest. Tests therefore need a synthetic overlap case. |
| `girpr` has no version-upgrade sweep; purge is opt-in; no keep-extra option | Confirmed | `src/repair/mod.rs:505,703-740,987-1021`; `src/cli.rs:60-129,183-230`; `src/config.rs:122-181`. `getGameDeprecatedFileConfigs` deletion is already independent of purge (`src/repair/mod.rs:771-781`, `phases/post.rs:452-531`). |

*Live detail worth keeping:* `getGameBranches` and `getPatchBuild` both work for
`hk4e_cn` and `hk4e_bilibili` on the mihoyo hosts (both returned `7.2.0`, five
manifests), even though Starward throws for bilibili. The patch blob is zstd
(199,587 compressed → 901,266 decoded bytes); the descriptor's checksum is the
MD5 of the decoded bytes, as in girpr's existing manifest fetch.

## Decisions already settled by the requester

1. **Union-all delete lists** is acceptable; the live-manifest cross-check is
   the safety gate. No keying off `config.ini`'s version.
4. Fetch/decode failures are **best-effort** (same class as `deprecated_files`),
   not fatal to the run.
5. Mode gating: normal repair and `--purge-only` act; `--dry-run` lists;
   `--check-only` and `--print-plan` skip entirely.
7. Counter/ceiling behaviour **follows the existing precedent**:
   `deprecated_files` deletes count into `deleted_extra_bytes` and are exempt
   from `--max-delete-bytes` (`docs/04:362`); the new sweep does the same.
9. The 18-name legacy `deprecated_files` delete **moves to the prerequisite
   phase** alongside the patch-delete sweep, rather than staying in post-phase.

## Still ambiguous — settle before planning

- **Which manifests feed the union.** Recommended: the same filter repair uses
  (game + selected audio, minus `res_category_dir` ignores). Today audio
  manifests carry no delete tags, so this is future-proofing.
- **Manifest-exemption source.** Recommended: the live chunk-manifest paths
  girpr already has (`per_manifest` / repair plan), not the patch manifest's own
  `patches` list. Both express "new-version file set"; girpr's own set is what
  it will actually install.
- **Ordering inside the prerequisite phase.** Recommended: after
  `prepare_install`/lock/explicit-audio and **before** `--purge-before`, so the
  space is freed as early as possible.
- **Bilibili support.** Recommended: support all three girpr biz values, since
  the live endpoint serves bili; do not copy Starward's `ArgumentOutOfRangeException`.
- **Fresh/mismatched install.** Expected: union-all makes `config.ini` accuracy
  irrelevant and a fresh install a no-op. Confirm no version key is wanted
  anywhere in this feature.

## Feasibility sketch (not a plan)

- Add `getPatchBuild` POST + DTOs to `app/fetch/src/hyp.rs`; `meta.branch`
  already has `package_id`/`branch`/`password`.
- Add the patch-manifest wire types to `crates/proto` (additive; shared with
  `gircap`) and decode through the existing zstd + MD5-verified manifest path;
  adapt for `diff_download` vs `chunk_download` in the descriptor.
- Adapt `sophon::filter::select_manifests` to the patch build, union the tags,
  subtract live-manifest paths, then route through a generalized version of
  `post::delete_deprecated` (scope → `contained_join` → files-only → dry-run
  listing → `deleted_extra_bytes`).
- Fetch in the plan phase alongside the deprecated list, carry on `PlanPhase`,
  execute in the prerequisite phase before `--purge-before`.
- Update `app/test-fixtures` mock (and `tools/capture` if bridge captures should
  cover it), add proto/filter/scope/integration tests, and document the
  one-time `deleted_extra_bytes` change (cf. the C12 note in README).

## Revalidation checklist before implementation

1. Re-run the live `getGameBranches` / `getPatchBuild` / `manifest_download`
   probes; confirm latest tag, `diff_tags`, number of delete tags and that
   `tag` is still ignored.
2. Re-confirm the protobuf field numbers against current launcher sources
   (`Starward.RPC/GameInstall/Sophon.proto`, Collapse `SophonPatchProto.proto`).
3. Re-check the remaining ambiguous items above with the requester.
4. Confirm which mock/fixture/capture surfaces need a new route and that no
   existing byte-stable contract (`SUMMARY`, `PLAN`, `PLAN` JSON) is broken
   unintentionally; document intentional changes.
5. Re-read the current `src/repair/mod.rs`, `phases/post.rs` and `src/cli.rs`
   line references — they were accurate at investigation time but will drift.
6. Establish a clean baseline (`cargo nextest run --workspace --all-features`)
   before changing code.
