# Finding & Verdict — Low-disk Genshin repair/patch (Starward vs Collapse)

Scope: Genshin Impact only (`hk4e_cn` / `hk4e_global` / `hk4e_bilibili`), Rust tool, automation-friendly
(no UI), minimize disk usage before / during / after patching. Repair-capable: local files may be
corrupt or any older version; after the run the game must be at the current live version.

## 1. How each launcher discovers the file list

### Starward (`reference-projects/Starward`)

- Local version: `src/Starward.RPC/GameInstall/GamePackageService.cs:GetLocalGameVersionAsync`
  reads `config.ini` (`game_version=` regex, last match). Missing file = install.
- Metadata API: `src/Starward.Core/HoYoPlay/HoYoPlayClient.cs` (`BuildHypUrl` / `BuildSophonUrl`).
  - CN host `https://hyp-api.mihoyo.com/hyp/hyp-connect/api/{api}?launcher_id=...&language=...`,
    global `https://sg-hyp-api.hoyoverse.com/...`. Launcher IDs
    (`src/Starward.Core/HoYoPlay/LauncherId.cs`): CN `jGHBHlcOq1`, global `VYTpXlbWo8`,
    bili-Genshin `umfgRO5gh5`. Game IDs (`GameId.cs`): `hk4e_cn=1Z8W5NHUQb`,
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
  `SophonChunk{id,uncompressed_md5,offset,compressed_size,uncompressed_size,compressed_md5}`.
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
  Chunk identity may be `xxh64` (parsed from chunk name) with MD5 fallback.
- Genshin glue (`RepairManagement/Genshin/Fetch.cs:BuildPrimaryManifest`) fakes the legacy `pkg_version`
  from the Sophon manifest (post-5.6 HoYo removed scattered zips): `SophonAssetDictRef` map +
  `PkgVersionProperties{remoteName,fileSize,md5}` per asset + per-VO `Audio_*_pkg_version` from
  `audio_lang_14`. `Fetch.Persistent.cs` additionally queries the **game dispatcher**
  (`ClientGameResURL/.../res_versions_external`, `ClientDesignDataURL/.../data_versions`) and can
  override Sophon entries (`IsRequireForcePersistent`); writes `..._persist` revisions and updates
  `data_revision/res_revision/.../audio_revision/ChannelName/ScriptVersion/PatchDone` after repair.
  Branch URLs come from `Helper/Metadata/PresetConfig.cs:SophonChunkUrls` after `EnsureReassociated`.
- So Collapse's API surface is strictly larger: HoYoPlay-equivalent launcher resource API **plus**
  dispatcher + persistent manifests + `audio_lang_14` + `ctable.dat` handling + plugin/SDK/WPF zip
  enumeration for cleanup.

## 2. Repair vs update, hashing, full vs diff

| | Starward | Collapse |
|---|---|---|
| Repair entry | `GameInstallService.cs:ExecuteRepairTaskAsync` ← `PrepareForInstallOrRepairAsync` (latest chunk build + optional local-version build for dedup) | `RepairManagement/Genshin/GenshinRepair.cs:CheckRoutine`: `ResetAndFetchAssets→CountAssetIndex→Check→Summarize`; `Repair.cs:RepairAssetTypeGeneric` |
| Update entry | `ExecuteUpdateTaskAsnyc`: Patch build if `localVersion ∈ DiffTags`, else latest+local chunk builds, else legacy package patch/major | `InstallManagerBase.Sophon*.cs`: (A) `SophonPatch` hdiff path `EnumerateUpdateAsync`+`DownloadPatchAsync`+`ApplyPatchUpdateAsync`, (B) `SophonUpdate` chunk-diff fallback `EnumerateUpdateAsync(oldPair,newPair)` + `WriteUpdateAsync` |
| Hash | **MD5 (lowercase hex) + size only**. `GameInstallHelper.cs:CheckFileMD5Async` (512 KiB blocks). File-level skip, per-reused-chunk slice MD5 (`FileSliceStream`), chunk-cache compressed MD5, final `_tmp` MD5; mismatch → delete tmp + throw | Size first, then `XxHash64` if `xxh64hash` present else MD5 (`Check.cs:CheckAssetAllType`). Per-chunk decompressed hash verified on the fly during copy/download (`SophonAsset.Download.cs:InnerWriteStreamToAsync`); mismatch → retry from network. `UseFastMethod` can skip hashing (not for us) |
| Full vs diff | **Repair never uses diffs**: every target file verified; missing/corrupt bytes re-fetched as **full chunks** (zstd blobs). Update prefers `Patch` (ldiff blobs + `HPatch.PatchZstandard`) or legacy `hdiffmap.json/hdifffiles.txt/deletefiles.txt` + 7z, falling back to chunk-dedup (only chunks with empty `OriginalFileFullPath` download; rest copied from `OriginalFileFullPath:Offset` after slice-MD5 check) | Repair: full-file `WriteToStreamAsync` per broken asset (chunk-parallel, per-chunk resume/skip via `CheckChunkMd5HashAsync`). Update (A) hdiff: needs intact `OriginalFile` (hash-checked, else `PatchMethod=DownloadOver` fallback; 5 retries → force full). Update (B) chunk-diff: joins old+new **server** manifests by `AssetName` and `ChunkDecompressedHashMd5`; `ChunkOldOffset=-1` = download, else copy from old file / staged `chunk_collapse/` blob / network; old-reference bytes are hash-verified after copy with network fallback |
| Version gate | **Repair = Install path: no `DiffTags` check.** Works from any/unknown local version (missing `config.ini` → `localFile=null` → full fetch, still skips already-correct files by MD5). This is the behavior the user relies on ("patch to new version directly") | Update refuses/redirects when local version has no diff entry (`SophonPatchBranch.IsFound=false` → fallback to B). Repair itself is version-agnostic, but the surrounding launcher gates repair on version checks (the user's complaint). Overall flow also depends on dispatcher persistent revisions |

Both therefore **can** repair corruption (final/full verification + network fallback), but only
Starward's repair path is *unconditionally* version-agnostic by construction.

## 3. Disk-space behavior (the deciding factor)

Starward (`GameInstallHelper.cs`, `GameInstallService.cs:ClearDeprecatedFiles` L722-777):
- No sparse files / preallocation (`FileSliceStream.SetLength` throws).
- **Temp-then-atomic-move everywhere**: chunk mode writes `FullPath_tmp` (`OpenOrCreate`, resume by
  `Length < Offset+UncompressedSize`), streaming zstd-decompress via `Pipe+DecompressionStream`;
  single/package mode writes `path_tmp` with `Range:` resume; hdiff outputs `target_tmp`; hardlink via
  `*.link`. Verify MD5 → `Move(tmp,final,true)`, else `Delete(tmp)` + rollback. No separate retained
  chunk cache in the common path (only transient `InstallPath/chunk/{id}` reuse probe); no preload blobs.
- Peak transient per file ≈ `old file + new _tmp` while that file is processed; files processed in
  parallel (`Parallel.ForEachAsync`, unbounded in code → we will bound it). No whole-game duplicate.
- **Starward-handling cleanup (post-task ONLY, never an orphan sweep)** — `ClearDeprecatedFiles`
  runs after Install/Update/Repair (never Predownload), and only when `PredownloadVersion is null`
  does it also sweep `**/*_tmp`, `**/*.hdiff`, dirs `chunk/`, `ldiff/`, `staging/` (L747-773).
  Otherwise it deletes exactly: downloaded compressed packages (L729-737) + each
  `DeprecatedFileConfig.deprecated_files[].Name` joined under install path (L738-746, source:
  `GET getGameDeprecatedFileConfigs`, node `deprecated_file_configs`,
  `GamePackageService.cs:993-996` via `HoYoPlayClient.cs:222-226`). Update-patch additionally
  deletes consumed diffs/sources/`hdiffmap.json|hdifffiles.txt|deletefiles.txt`
  (`GameInstallHelper.cs:730-841`) and per-`DeleteTags[localVersion]` files
  (`GameInstallService.cs:508-519`, populated `GamePackageService.cs:329-376`).
  Pre-task does NO deletes (only `SetAttributes(Normal)`).
- **Does NOT purge unknown extra files** (only the deprecated list; `GameInstallService.cs:681`
  `// todo clear useless audio` — unselected audio is never cleaned). `res_category_dir`
  (`{"category":"...","is_delete":true}`) and `blacklist_dir` (`{"fileName":"..."}`) only
  *exclude* entries from the download work list (`GamePackageService.cs:128-143,742-769`); they are
  never deleted. Audio `cache→res` move (`GameInstallService.cs:427-444,643-660`, `File.Move`
  overwrite) and `config.ini` bump (`SetGameConfigIniAsync`, L788-850) are moves/rewrites, not purges.

Collapse (`Hi3Helper.Sophon/*`, `InstallManagerBase.Sophon*.cs`, `GenshinInstall.cs:GetUnusedFileInfoList`):
- Quota gate `EnsureDiskSpaceSufficiencyAsync` (volume free space + dialog) — UI-oriented, skip for CLI.
- Temps: `*_tempSophon`, `*_tempUpdate`, `*.temp`, plus retained `GamePath/chunk_collapse/` (preload
  compressed blobs + `.verified` + `preload.verified` flag) and `GamePath/ldiff/` (patch blobs),
  plus `%LocalLow%/_sophonMetadataCache/` manifests. Preload phase downloads compressed blobs **before**
  apply (extra transient ≈ compressed delta retained alongside game), then apply writes `_tempUpdate`
  alongside the original → `Move`. Patch path holds `ldiff/{PatchName}` + `target.temp` + old file.
- Streaming: `PerformWriteStreamThreadAsync` (`ResponseHeadersRead` + zstd stream, `MD5.TransformBlock`
  on the fly, `ArrayPool` 4 KiB, per-chunk `Parallel.ForEachAsync(max(8,CPU))`, `FileStream.Lock`
  disabled via `NOSTREAMLOCK`), 30 s timeout/retry, speed limiter.
- Deletion comes in **two separate Collapse-handling mechanisms** (do not conflate with Starward's
  post-task sweep above):
  - (a) **Manual orphan purge** — `GenshinInstall.GetUnusedFileInfoList` override
    (`CollapseLauncher/Classes/InstallManagement/Genshin/GenshinInstall.cs:176-263`), triggered only by
    user cleanup UI (`HomePage.xaml.cs:969-981`, `MainPage.Navigation.cs:396-405` → `CleanUpGameFiles()`
    → `InstallManagerBase.PkgVersion.cs:151-188`), never auto before/after repair. Expected set =
    `Repair.ResetAndFetchAssets()` union (Sophon fake-`pkg_version` via
    `GenshinInstall.PkgVersion.cs:112-190` + dispatcher persistent via `Fetch.Persistent.cs:44-117` +
    plugin/SDK/WPF zip entries via `SimpleZipArchiveReader`), diffed against full
    `EnumerateFiles("*", AllDirectories)`. Protected: server `FilesCleanupIgnoreList` regexes
    (`PresetConfig.cs:86-94`, normally `[]` for Genshin — `GenshinInstall.cs:145-172` sets none) matched
    against `RelativePath` via `WhereMatchPattern` (`PatternMatcher.cs:93-108`, case-insensitive,
    non-matching kept) **plus** per-line `^Audio_<entry>_pkg_version$` built from
    `..._Data/Persistent/audio_lang_14` (`GenshinInstall.cs:228-247`). Note the base-class
    `config.ini/pkg_version/Persistent/ScreenShot` protections (`InstallManagerBase.PkgVersion.cs:456-535`)
    do NOT apply to this override (except insofar as those files are in the union).
  - (b) **Repair-time redundant pass** — `Check.cs:26-67` → `CheckRedundantFiles` (before the hash loop)
    marks `*deletefiles*` entries + `*.diff/*_tmp/*.hdiff` (`Check.cs:232-319`) as `Unused`, deleted in
    `Repair.cs:150-165` during `RepairAssetTypeGeneric`.
- **v1 purge scope for this tool (Collapse-handling, no SDK/WPF/dispatcher)**: expected set =
  `{latest chunk-manifest paths} ∪ {config.ini, exe per game config, blacklist file, res_category file,
  audio scan file, audio_lang_14 + Audio_*_pkg_version for selected langs}`; allowlist additionally keeps
  user data both launchers never touch (`ScreenShot/**`, log dirs). Temp names (`*_tmp`, `*.hdiff`,
  `chunk/`, `ldiff/`, `staging/`, legacy `*.diff`, `*deletefiles*`) belong to Starward-handling temp
  sweep, NOT to the purge comparison (purge skips them; sweep deletes them) so byte accounting stays split
  (`freed_temp_bytes` vs `deleted_extra_bytes`). This is exactly the user's Collapse usage
  (before + after Starward).

## 4. Verdict

**Adopt Starward's `Repair-in-Chunk-mode` as the patching core, plus Collapse's extra-file purge as a
pre/post phase. Specific to Genshin, minimal API surface = Starward HoYoPlay + Sophon chunk only.**

Reasons:
1. Meets the hard constraint: Starward Repair is version-agnostic (latest chunk build + full-file MD5
   verification; corrupt/unknown-version files converge by re-fetching full chunks). It never requires an
   intact source file, unlike hdiff/patch paths which both launchers only use for *update* with fallback.
   Collapse's repair is equally capable but wrapped in version gating + dispatcher/persistent overhead.
2. Lowest disk usage during patching: per-file `<file>_tmp` streaming (resume by length) + local-chunk
   slice reuse (verified) + final atomic move; no retained `chunk_collapse/` preload store, no `ldiff/`
   store, no manifest cache inside the game dir, no multi-GB zip staging. Peak ≈ largest single file
   ×2 transient, processed with bounded file parallelism (HDD-safe). Collapse's preload-then-apply and
   patch-blob retention use strictly more transient space.
3. Smallest implementation: HoYoPlay `getGameBranches` → `getBuild(latest)` [+ `getBuild(local)` for
   dedup] → manifest download/verify/parse → per-file repair → **Starward-handling post-phase**
   (`getGameDeprecatedFileConfigs` delete + temp sweep + audio cache→res move + `config.ini` bump).
   No dispatcher (`res_versions`/`data_versions`), no persistent
   revisions, no `ctable` juggling, no hdiff/7z pipeline, no SDK/WPF/plugin zips, no speed limiter, no
   quota dialog. Collapse's extras are launcher conveniences, not needed to leave a launchable Genshin.
4. Collapse contributes the one thing Starward lacks: full extra-file purge by set-difference
   (**Collapse-handling**, `GetUnusedFileInfoList` parity scoped to v1 — no SDK/WPF/dispatcher union).
   Cheap to implement, big low-disk win before/after patching, and directly replicates the user's
   current two-launcher workflow in one tool.

Extra behavior changes recommended (beyond 1:1 port):
- **Starward-handling, extended**: run the temp sweep (`*_tmp`, `*.hdiff`, `chunk/`, `ldiff/`,
  `staging/`) **before** patching too (Starward only sweeps post-task) to reclaim space.
- **Collapse-handling**: purge-extra both before (`--purge-before`, repair-time `CheckRedundantFiles`
  parity — frees space for the repair itself) and after (`--purge-extra`, manual-cleanup parity);
  dry-run lists bytes. Expected set per §3-Collapse v1 scope; keep allowlist minimal
  (`config.ini`, server-config metadata files, selected `audio_lang_*`, `ScreenShot/`, log dirs).
- Bound file parallelism (`--io-threads`; chunks sequential within a file) for HDD vs SSD.
- Skip SDK/WPF downloads and dispatcher persistent writes in v1 (game launches without them; add
  `--with-sdk --with-wpf` later if a channel proves otherwise). Always move audio cache→res if the
  game config names distinct dirs (Starward parity, prevents stranded duplicates).
- Structured logs + exit codes for automation; `--check-only` verify mode; resume-safe reruns
  (idempotent: intact files skipped by size+MD5).

## 5. File / API reference (for implementers)

Starward: `src/Starward.Core/HoYoPlay/{HoYoPlayClient.cs,GameId.cs,LauncherId.cs,LauncherConfig.cs,
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
`CollapseLauncher/Classes/{RepairManagement/Genshin/{Fetch.cs,Fetch.Persistent.cs,Check.cs,Repair.cs,
GenshinRepair.cs,BSDiff.cs},InstallManagement/{Genshin/{GenshinInstall.cs,GenshinInstall.PkgVersion.cs},
Base/InstallManagerBase*.cs},Helper/Metadata/PresetConfig.cs}`. Libs: `Google.Protobuf`,
`Hi3Helper.ZstdNet`, `SharpHPatchZ`, `System.IO.Hashing` (xxh64 — not needed; MD5 suffices),
`Hi3Helper.{Http,EncTool,Win32,SimpleZipArchiveReader}`.
