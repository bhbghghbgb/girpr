# Repair findings & verdict - Low-disk GI repair/patch (Starward vs Collapse)

Scope: GI only (`hk4e_cn` / `hk4e_global` / `hk4e_bilibili`), Rust tool, automation-friendly
(no UI), minimize disk usage before / during / after patching. Repair-capable: local files may be
corrupt or any older version; after the run the game must be at the current live version.

## Reference projects (`reference-projects/`)

The checkouts under `reference-projects/` are read-only references for this design; no product code
imports them. Each one covers a different part of the GI update/repair mechanism:

- `reference-projects/Starward` — reference for HYP metadata discovery, Sophon chunk manifests and
  the version-agnostic repair core this tool ports (§1, §4).
- `reference-projects/Collapse` — reference for the extra-file purge and the dispatcher/persistent
  manifest handling that Starward lacks (§1, §3; see also §6).
- `reference-projects/GenshinSlimmer` — reference for the persistent files the game client
  writes (§6).
- `reference-projects/GI-Live-Hotfix-Downloader` — reference for querying the dispatcher for
  hotfix versions and downloading persistent hotfix files (§6).

## 1. How each launcher discovers the file list

### Starward (`reference-projects/Starward`)

- Local version: `src/Starward.RPC/GameInstall/GamePackageService.cs:GetLocalGameVersionAsync`
  reads `config.ini` (`game_version=` regex, last match). Missing file = install.
- Metadata API: `src/Starward.Core/HoYoPlay/HoYoPlayClient.cs` — `BuildUrl` builds the HYP
  URLs; the Sophon `getBuild`/`getPatchBuild` URLs are built in the same file by
  `GetGameChunkBuildAsync` / `GetGameSophonChunkBuildAsync` / `GetGameSophonPatchBuildAsync`.
  - CN host `https://hyp-api.mihoyo.com/hyp/hyp-connect/api/{api}?launcher_id=...&language=...`,
    global `https://sg-hyp-api.hoyoverse.com/...`. Launcher IDs
    (`src/Starward.Core/HoYoPlay/LauncherId.cs`): CN `jGHBHlcOq1`, global `VYTpXlbWo8`,
    bili-GI `umfgRO5gh5`. Game IDs (`GameId.cs`): `hk4e_cn=1Z8W5NHUQb`,
    `hk4e_global=gopR6Cufr3`, `hk4e_bilibili=T2S0Gz4Dr2`.
  - Used endpoints: `getGameConfigs` (exe name, audio scan/cache/res dirs, `default_download_mode`,
    `res_category_dir`, `blacklist_dir`, flags), `getGameBranches` (branch/package_id/password/tag/
    diff_tags), `getGameDeprecatedFileConfigs` (delete list), plus `getGamePackages` (legacy zip fallback),
    `getGameChannelSDKs`, `getWPFPackages`, `getGameScanInfo` (detection only).
- Sophon chunk API: `GET getBuild?branch=&package_id=&password=[&tag=]` →
  `GameSophonChunkBuild{build_id,tag,manifests[]}` (`GameSophonChunkBuild.cs`).
  Each manifest: `category_id/category_name/matching_field` (`game`, `zh-cn|en-us|ja-jp|ko-kr|mini-*`),
  `manifest{id,checksum,compressed_size,uncompressed_size}`, `chunk_download{url_prefix,...}`,
  `manifest_download{...}`. Patch variant: `POST getPatchBuild` (diff manifests, `DeleteTag`s).
- Manifest fetch (`GamePackageService.cs:EnsureSophonManifestFileAsync`): cache under app cache
  `game/{manifest.id}`, zstd-decompress (`ZstdSharp`), verify decompressed **MD5 == checksum**,
  else re-download `GET {UrlPrefix}/{id}`. Parse protobuf per `src/Starward.RPC/GameInstall/Sophon.proto`:
  `SophonChunkManifest{chunks: SophonChunkFile{file,chunks,is_folder,size,md5}}`,
  `SophonChunk{id,uncompressed_md5,offset,compressed_size,uncompressed_size,unknown_field6:int64=6,compressed_md5=7}`
  (field 6 is opaque and its meaning unknown; keep tag 7 for `compressed_md5` or protobuf misparses).
  Chunk bytes URL = `ChunkDownload.UrlPrefix/{chunk.id}`.
- Manifest filtering: skip `matching_field` listed in `res_category_dir` file
  (`{"category":"...","is_delete":true}` lines), skip all `*-??` audio fields then re-add only selected
  `AudioLanguage`s. Same for patch manifests (`GetAvailableGameSophonChunkManifests`,
  `GetAvaliableGameSophonPatchManifests`).
- Legacy fallback (`res_list_url` + `pkg_version` / `Audio_*_pkg_version` NDJSON
  `{remoteName,md5,fileSize}`) only if download mode is `FILE` or `getBuild` returns -202.

### Collapse (`reference-projects/Collapse`)

Core lib `Hi3Helper.Sophon/` + glue `CollapseLauncher/Classes/{RepairManagement/Genshin,InstallManagement/Genshin}`.
- Same Sophon primitives, different shape: `Structs/SophonChunksBranch.cs:CreateSophonChunkManifestInfoPair`
  (`GET {BranchUrl}[&tag=]` → build JSON → pick `matchingField`), `SophonManifest.cs:EnumerateAsync`
  (`GET {ManifestBaseUrl}/{ManifestId}`, zstd + `Google.Protobuf` parse of `Protos/SophonManifestProto.proto`,
  cached in `%LocalLow%/CollapseLauncher/_sophonMetadataCache/manifest_{xxh64}`),
  `Helper/Extension.cs:GetChunkAndIfAltAsync` (`GET {ChunksBaseUrl}/{ChunkName}` + alt fallback).
  Repair/download chunk identity is MD5-only (`ChunkDecompressedHashMd5`); `xxh64` parsed from the
  chunk name is only a preload/diff-staging resume probe, not the repair hash.
- GI glue (`RepairManagement/Genshin/Fetch.cs:BuildPrimaryManifest`) fakes the legacy `pkg_version`
  from the Sophon manifest (HoYo removed scattered zips in a recent release): `SophonAssetDictRef` map +
  `PkgVersionProperties{remoteName,fileSize,md5}` per asset + per-VO `Audio_*_pkg_version` from
  `audio_lang_14`. `Fetch.Persistent.cs` additionally queries the **game dispatcher**
  (`ClientGameResURL/.../res_versions_external`, `ClientDesignDataURL/.../data_versions`) and can
  override Sophon entries (`IsRequireForcePersistent`); writes `..._persist` revisions and updates
  `data_revision/res_revision/.../audio_revision/ChannelName/ScriptVersion/PatchDone` after repair.
  Branch URLs come from `Helper/Metadata/PresetConfig.cs:SophonChunkUrls` after `EnsureReassociated`.
- So Collapse's API surface is strictly larger: HYP-equivalent launcher resource API **plus**
  dispatcher + persistent manifests + `audio_lang_14` + `ctable.dat` handling + plugin/SDK/WPF zip
  enumeration for cleanup.

## 2. Repair vs update, hashing, full vs diff

| | Starward | Collapse |
|---|---|---|
| Repair entry | `GameInstallService.cs:ExecuteRepairTaskAsync` ← `PrepareForInstallOrRepairAsync` (latest chunk build + optional local-version build for dedup) | `RepairManagement/Genshin/GenshinRepair.cs:CheckRoutine`: `ResetAndFetchAssets→CountAssetIndex→Check→Summarize`; `Repair.cs:RepairAssetTypeGeneric` |
| Update entry | `ExecuteUpdateTaskAsnyc`: Patch build if `localVersion ∈ DiffTags`, else latest+local chunk builds, else legacy package patch/major | `InstallManagerBase.Sophon*.cs`: (A) `SophonPatch` hdiff path `EnumerateUpdateAsync`+`DownloadPatchAsync`+`ApplyPatchUpdateAsync`, (B) `SophonUpdate` chunk-diff fallback `EnumerateUpdateAsync(oldPair,newPair)` + `WriteUpdateAsync` |
| Hash | **MD5 (lowercase hex) + size only**. `GameInstallHelper.cs:CheckFileMD5Async` (streamed blocks). File-level skip, per-reused-chunk slice MD5 (`FileSliceStream`), chunk-cache compressed MD5, final `_tmp` MD5; mismatch → delete tmp + throw | Size first, then `XxHash64` if `xxh64hash` present else MD5 (`Check.cs:CheckAssetAllType`). Per-chunk decompressed hash verified on the fly during copy/download (`SophonAsset.Download.cs:InnerWriteStreamToAsync`); mismatch → retry from network. `UseFastMethod` can skip hashing (not for us) |
| Full vs diff | **Repair never uses diffs**: every target file verified; missing/corrupt bytes re-fetched as **full chunks** (zstd blobs). Update prefers `Patch` (ldiff blobs + `HPatch.PatchZstandard`) or legacy `hdiffmap.json/hdifffiles.txt/deletefiles.txt` + 7z, falling back to chunk-dedup (only chunks with empty `OriginalFileFullPath` download; rest copied from `OriginalFileFullPath:Offset` after slice-MD5 check) | Repair: full-file `WriteToStreamAsync` per broken asset (chunk-parallel, per-chunk resume/skip via `CheckChunkMd5HashAsync`). Update (A) hdiff: needs intact `OriginalFile` (hash-checked, else `PatchMethod=DownloadOver` fallback; 5 retries → force full). Update (B) chunk-diff: joins old+new **server** manifests by `AssetName` and `ChunkDecompressedHashMd5`; `ChunkOldOffset=-1` = download, else copy from old file / staged `chunk_collapse/` blob / network; old-reference bytes are hash-verified after copy with network fallback |
| Version gate | **Repair = Install path: no `DiffTags` check.** Works from any/unknown local version (missing `config.ini` → `localFile=null` → full fetch, still skips already-correct files by MD5). This is the behavior the user relies on ("patch to new version directly") | Update refuses/redirects when local version has no diff entry (`SophonPatchBranch.IsFound=false` → fallback to B). Repair itself is version-agnostic, but the surrounding launcher gates repair on version checks (the user's complaint). Overall flow also depends on dispatcher persistent revisions |

Both therefore **can** repair corruption (final/full verification + network fallback), but only
Starward's repair path is *unconditionally* version-agnostic by construction.

## 3. Disk-space behavior (the deciding factor)

Starward (`GameInstallHelper.cs`, `GameInstallService.cs:ClearDeprecatedFiles`):
- No sparse files / preallocation (`FileSliceStream.SetLength` throws).
- **Temp-then-atomic-move everywhere**: chunk mode writes `FullPath_tmp` (`OpenOrCreate`, resume by
  `Length < Offset+UncompressedSize`), streaming zstd-decompress via `Pipe+DecompressionStream`;
  single/package mode writes `path_tmp` with `Range:` resume; hdiff outputs `target_tmp`; hardlink via
  `*.link`. Verify MD5 → `Move(tmp,final,true)`, else `Delete(tmp)` + rollback. No separate retained
  chunk cache in the common path (only transient `InstallPath/chunk/{id}` reuse probe); no preload blobs.
- Peak transient per file ≈ `old file + new _tmp` while that file is processed; files are
  processed in parallel. No whole-game duplicate.
- **Starward-handling cleanup (post-task ONLY, never an orphan sweep)** — `ClearDeprecatedFiles`
  runs after Install/Update/Repair (never Predownload), and only when `PredownloadVersion is null`
  does it also sweep `**/*_tmp`, `**/*.hdiff`, dirs `chunk/`, `ldiff/`, `staging/`.
  Otherwise it deletes exactly: downloaded compressed packages + each
  `DeprecatedFileConfig.deprecated_files[].Name` joined under install path (source:
  `GET getGameDeprecatedFileConfigs`, node `deprecated_file_configs`,
  `GamePackageService.cs` via `HoYoPlayClient.cs`). Update-patch additionally
  deletes consumed diffs/sources/`hdiffmap.json|hdifffiles.txt|deletefiles.txt`
  (`GameInstallHelper.cs`) and per-`DeleteTags[localVersion]` files
  (`GameInstallService.cs`, populated `GamePackageService.cs`).
  Pre-task does NO deletes (only `SetAttributes(Normal)`).
- **Does NOT purge unknown extra files** (only the deprecated list; `GameInstallService.cs`
  `// todo clear useless audio` — unselected audio is never cleaned). `res_category_dir`
  (`{"category":"...","is_delete":true}`) and `blacklist_dir` (`{"fileName":"..."}`) only
  *exclude* entries from the download work list (`GamePackageService.cs` blacklist apply,
  `GamePackageService.cs` res-category read consumed by the `GamePackageService.cs` filter); they are
  never deleted. Audio `cache→res` move (`GameInstallService.cs`, `File.Move`
  overwrite) and `config.ini` bump (`SetGameConfigIniAsync`) are moves/rewrites, not purges.

Collapse (`Hi3Helper.Sophon/*`, `InstallManagerBase.Sophon*.cs`, `GenshinInstall.cs:GetUnusedFileInfoList`):
- Quota gate `EnsureDiskSpaceSufficiencyAsync` (volume free space + dialog) — UI-oriented, skip for CLI.
- **Writes in place for its plain-HTTP repair path** (`Hi3Helper.Http/DownloadClient.cs` opens
  the final path directly, and `Hi3Helper.Http/ChunkSession.cs` deletes before writing when
  `overwrite` is on — the default, `CollapseLauncher/Classes/Interfaces/Class/ProgressBase.cs`).
  "Write the broken file where it lies" is therefore a shipped launcher behaviour, not an
  invention: the chunk path below is the one that stages.
- Temps: `*_tempSophon`, `*_tempUpdate`, `*.temp`, plus retained `GamePath/chunk_collapse/` (preload
  compressed blobs + `.verified` + `preload.verified` flag) and `GamePath/ldiff/` (patch blobs),
  plus `%LocalLow%/_sophonMetadataCache/` manifests. Preload phase downloads compressed blobs **before**
  apply (extra transient ≈ compressed delta retained alongside game), then apply writes `_tempUpdate`
  alongside the original → `Move`. Patch path holds `ldiff/{PatchName}` + `target.temp` + old file.
- Streaming: `PerformWriteStreamThreadAsync` (`ResponseHeadersRead` + zstd stream, `MD5.TransformBlock`
  on the fly, pooled buffers, a bounded per-chunk `Parallel.ForEachAsync`, `FileStream.Lock`
  disabled via `NOSTREAMLOCK`), a timeout/retry, speed limiter.
- Deletion comes in **distinct Collapse-handling mechanisms** (do not conflate with Starward's
  post-task sweep above):
  - (a) **Manual orphan purge** — `GenshinInstall.GetUnusedFileInfoList` override
    (`CollapseLauncher/Classes/InstallManagement/Genshin/GenshinInstall.cs`), triggered only by
    user cleanup UI (`HomePage.xaml.cs`, `MainPage.Navigation.cs` → `CleanUpGameFiles()`
    → `InstallManagerBase.PkgVersion.cs`), never auto before/after repair. Expected set =
    `Repair.ResetAndFetchAssets()` union (Sophon fake-`pkg_version` via
    `GenshinInstall.PkgVersion.cs` + dispatcher persistent via `Fetch.Persistent.cs` +
    plugin/SDK/WPF zip entries via `SimpleZipArchiveReader`), diffed against full
    `EnumerateFiles("*", AllDirectories)`. Protected: server `FilesCleanupIgnoreList` regexes
    (`PresetConfig.cs`, normally `[]` for GI — `GenshinInstall.cs` sets none) matched
    against `RelativePath` via `WhereMatchPattern` (`PatternMatcher.cs`, case-insensitive,
    non-matching kept) **plus** per-line `^Audio_<entry>_pkg_version$` built from
    `..._Data/Persistent/audio_lang_14` (`GenshinInstall.cs`). Note the base-class
    `config.ini/pkg_version/Persistent/ScreenShot` protections (`InstallManagerBase.PkgVersion.cs`)
    do NOT apply to this override (except insofar as those files are in the union).
  - (b) **Repair-time redundant pass** — `Check.cs` → `CheckRedundantFiles` (before the hash loop)
    marks `*deletefiles*` entries + `*.diff/*_tmp/*.hdiff` (`Check.cs`) as `Unused`, deleted in
    `Repair.cs` during `RepairAssetTypeGeneric`.
- **Files-cleanup scope for this tool (Collapse-handling, no SDK/WPF/dispatcher)**:
  expected set = `{live chunk-manifest paths} ∪ {config.ini} ∪ {the legacy
  in-install girpr.lock} ∪ {cleaned blacklist paths} ∪ {verification-state names
  and root pkg_version}`, plus dispatcher persistent paths under
  `--with-persistent`, plus filename-pattern keeps for `audio_lang_*` +
  `Audio_*_pkg_version` (Collapse `GenshinInstall.cs` parity). Everything not
  kept by these rules purges into the
  single `deleted_extra_bytes` counter — including temps (`*_tmp`, `*.hdiff`, `chunk/`,
  `ldiff/`, `staging/`, legacy `*.diff`, `*deletefiles*`), unselected audio, `ScreenShot/`,
  logs, and server bookkeeping files (`ctable.dat` purges when absent from the live
  manifest; the exe is normally kept via manifest membership).
  `--purge-before` and `--purge-after` run this same cleanup; only the timing differs.

## 4. Verdict

**Adopt Starward's `Repair-in-Chunk-mode` as the patching core, plus Collapse's extra-file purge as a
pre/post phase. Specific to GI, minimal API surface = Starward HYP + Sophon chunk only.**

Reasons:
1. Meets the hard constraint: Starward Repair is version-agnostic (latest chunk build + full-file MD5
   verification; corrupt/unknown-version files converge by re-fetching full chunks). It never requires an
   intact source file, unlike hdiff/patch paths which both launchers only use for *update* with fallback.
   Collapse's repair is equally capable but wrapped in version gating + dispatcher/persistent overhead.
2. Lowest disk usage during patching: write the broken file **where it lies** — only the bytes
   that are wrong, at the offsets they belong at — plus local-chunk slice reuse (verified) and
   a whole-file MD5 gate; no retained `chunk_collapse/` preload store, no `ldiff/` store, no
   manifest cache inside the game dir, no multi-GB zip staging, and no per-file `_tmp` beside
   the file being repaired. Peak transient is the file's *growth*, not its size, and the
   chunk path's `*_tmp` staging is available as a mode for an install that wants the target
   left untouched. Collapse's preload-then-apply and patch-blob retention use strictly more
   transient space.
3. Smallest implementation: HYP `getGameBranches` → `getBuild(latest)` [+ `getBuild(local)` for
   dedup] → manifest download/verify/parse → per-file repair → **Starward-handling post-phase**
   (`getGameDeprecatedFileConfigs` delete + audio cache→res move + `config.ini` bump).
   No hdiff/7z pipeline, no SDK/WPF/plugin zips, no speed limiter, and no quota dialog. The
   dispatcher (`res_versions`/`data_versions`) and persistent revisions are a separate opt-in
   tier (`--with-persistent`, §6); the default path stays the small one described here.
4. Collapse contributes the one thing Starward lacks: full extra-file purge by set-difference
   (**Collapse-handling**, `GetUnusedFileInfoList` parity scoped to v1 — no SDK/WPF/dispatcher union).
   Cheap to implement, big low-disk win before/after patching, and directly replicates the user's
   current two-launcher workflow in one tool.

The resulting tool:
- Runs the same files-cleanup before or after (`--purge-before` / `--purge-after`);
  dry-run lists bytes. Expected set per §3: `config.ini`, a legacy in-install
  `girpr.lock`, manifest membership, blacklisted paths, verification-state names
  and root `pkg_version`, and the `audio_lang_*` / `Audio_*_pkg_version` pattern.
- Bounds file parallelism (`--jobs`; chunks sequential within a file) for HDD vs SSD.
- Does not download SDK/WPF/plugin content. Dispatcher persistent files are an opt-in
  tier (`--with-persistent`, §6). Audio cache→res moves run whenever the game config
  names distinct dirs (Starward parity).
- Emits structured logs and exit codes for automation, provides `--check-only`, and is
  resume-safe (intact files are skipped by size+MD5).

## 5. File / API reference (for implementers)

Starward: `src/Starward.Core/HoYoPlay/{HoYoPlayClient.cs,GameId.cs,LauncherId.cs,
GameBranch.cs,GameSophonChunkBuild.cs,GameSophonPatchBuild.cs,GamePackage.cs,GameConfig.cs,
GameDeprecatedFile.cs,GameScanInfo.cs}`, `src/Starward.RPC/GameInstall/{GamePackageService.cs
(discovery/manifests),GameInstallService.cs (orchestration/cleanup/config.ini),GameInstallHelper.cs
(MD5/download/chunk/7z/HPatch),GameInstallFile.cs (chunk↔local mapping),Sophon.proto,DiffMap.cs,
PkgVersionItem.cs,GameInstallContext.cs,GameInstallOperation.cs,GameInstallDownloadMode.cs,
FileSliceStream.cs,GameUninstallService.cs}`. Libs: `Starward.NativeLib` (HPatch/7z — **not needed**
for repair-chunk), `ZstdSharp.Port`, `Vanara.PInvoke.Kernel32` (hardlink — optional), `Grpc+Polly`
(retry — keep retry, drop grpc), `Google.Protobuf`.
Collapse: `Hi3Helper.Sophon/{SophonManifest.cs,SophonUpdate.cs,SophonPatch*.cs,SophonAsset.{Download,
Update,Diff}.cs,Structs/Sophon*.cs,Helper/{Extension,ChunkStream}.cs,Protos/*.proto}`,
`CollapseLauncher/Classes/{RepairManagement/{Genshin/{Fetch.cs,Fetch.Persistent.cs,Check.cs,Repair.cs,
GenshinRepair.cs},BSDiff.cs},InstallManagement/{Genshin/{GenshinInstall.cs,GenshinInstall.PkgVersion.cs},
Base/InstallManagerBase*.cs},Helper/Metadata/PresetConfig.cs}`. Libs: `Google.Protobuf`,
`Hi3Helper.ZstdNet`, `SharpHPatchZ`, `System.IO.Hashing` (xxh64 — not needed; MD5 suffices),
`Hi3Helper.{Http,EncTool,Win32,SimpleZipArchiveReader}`.

---

## 6. Persistent-directory update

GI has two live content paths. The launcher path (§1–§5) installs chunked files
under `*_Data/StreamingAssets/` and tracks them with `config.ini` and
`pkg_version`. The in-game hot-patch path reads the dispatcher
(`query_cur_region`) and keeps full replacement files plus verification state
under `*_Data/Persistent/`:

- `res_versions_persist`, `data_versions_persist` and
  `silence_data_versions_persist` — the game's hotfix asset databases, mirrored
  byte-for-byte from the dispatcher manifests;
- `base_res_version_hash` — lowercase-hex MD5 of
  `StreamingAssets/res_versions_streaming`, the base-install marker;
- `res_revision`, `res_revision_eternal`, `data_revision`, `silence_revision`,
  `audio_revision`, `ChannelName`, `ScriptVersion` and `PatchDone` — plain-text
  revision values (no trailing newline) describing the installed hotfix set.

The game trusts local state only when both halves agree with the dispatcher's
current numbers. A missing or stale marker makes it re-verify and re-download
hotfixes. Hotfix bytes are served as full files over HTTP; they are not in any
Sophon chunk manifest, so the chunk repair path cannot repair them. Collapse
writes both halves in its repair routine; Starward writes neither.

`girpr` maintains this state under `--with-persistent` (flag contract in
`README.md`, implementation design in [`docs/03`](03-repair-rust-implementation-plan.md)
§7). The default repair makes no dispatcher or persistent request and writes no
markers. With the flag:

- The dispatcher is queried with `meta.latest` for the selected `--biz`; all
  three regions are supported. Key and seed come from Collapse's release
  metadata and are checked against a pinned versioned seed; a mismatch is a
  metadata failure (exit `2`).
- The three manifests are fetched from the dispatcher descriptors, verified by
  size and MD5, and mirrored in the post phase.
- Rows are placed under `StreamingAssets/` or `Persistent/` by suffix and by
  comparison with the base manifest's MD5; the comparison decides which
  hotfixes are forced into `Persistent/`. Each hotfix is downloaded as a full
  file (primary URL, alternate on 404), gated by size and XXH64/MD5, and
  published atomically after a second verification read.
- Before repair, both base trees are created, stale `Persistent/ctable.dat` and
  `DXSETUP/` are removed, and `ctable.dat` is copied to `ctable_streaming.dat`
  when the source exists and the target differs. Stranded non-patch
  `.pck`/`.usm`/`.cuepoint` files are moved from `Persistent/` back to
  `StreamingAssets/` before `--purge-before`.
- The marker block and all three mirrors are published in the post phase,
  before the final `config.ini` bump. A failed persistent file (or a ceiling
  stop) suppresses the whole block; a byte-intact install still gets the full
  block refreshed. `--check-only` verifies persistent files, mirrors and
  markers, and read-only modes list the work without writing it.
- Persistent paths are part of the path scope and all byte/file ceilings. The
  verification-state files are kept by the purge on every run, flag or no flag.

The repair core, exit codes and record stream stay as described above. `PLAN`
and `SUMMARY` gain flag-only persistent keys; no new exit code is introduced.
