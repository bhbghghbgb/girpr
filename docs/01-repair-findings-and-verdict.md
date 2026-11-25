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
- `reference-projects/GenshinSlimmer` — reference for understanding the role of the different
  persistent files written by the game client (§6.2.4).
- `reference-projects/GI-Live-Hotfix-Downloader` — reference for understanding the process of
  querying the dispatcher for hotfix versions and the download of persistent hotfix files
  (§6.7).

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
- Peak transient per file ≈ `old file + new _tmp` while that file is processed; files processed in
  parallel (`Parallel.ForEachAsync`, unbounded in code → we will bound it). No whole-game duplicate.
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
- **v1 files-cleanup scope for this tool (Collapse-handling, no SDK/WPF/dispatcher)**: expected set =
  `{latest chunk-manifest paths} ∪ {config.ini} ∪ {the legacy in-install girpr.lock} ∪ {cleaned blacklist paths}`
  plus filename-pattern keeps for `audio_lang_*` + `Audio_*_pkg_version` (Collapse
  `GenshinInstall.cs` parity). Everything not kept by rules 0-4 purges into the
  single `deleted_extra_bytes` counter — including temps (`*_tmp`, `*.hdiff`, `chunk/`,
  `ldiff/`, `staging/`, legacy `*.diff`, `*deletefiles*`), unselected audio, `ScreenShot/`,
  logs, and server bookkeeping files (the exe / `pkg_version` / `Persistent/` / `ctable.dat`
  purge only when absent from the live manifest — the exe is normally kept via rule 1).
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
   No hdiff/7z pipeline, no SDK/WPF/plugin zips, no speed limiter, no
   quota dialog. Collapse's dispatcher (`res_versions`/`data_versions`), persistent revisions and
   `ctable` juggling were excluded from this base design — not needed to leave a launchable GI —
   but they are now available under `--with-persistent` (§6). The default path stays the small one
   described here.
4. Collapse contributes the one thing Starward lacks: full extra-file purge by set-difference
   (**Collapse-handling**, `GetUnusedFileInfoList` parity scoped to v1 — no SDK/WPF/dispatcher union).
   Cheap to implement, big low-disk win before/after patching, and directly replicates the user's
   current two-launcher workflow in one tool.

Extra behavior changes recommended (beyond 1:1 port):
- **Files-cleanup both before and after** (`--purge-before` / `--purge-after` run the
  same Collapse `GetUnusedFileInfoList`-parity cleanup; only timing differs);
  dry-run lists bytes. Expected set per §3 v1 scope; keep minimal
  (`config.ini` + a legacy in-install `girpr.lock` + manifest membership +
  blacklisted paths + `audio_lang_*` / `Audio_*_pkg_version` pattern).
- Bound file parallelism (`--jobs`; chunks sequential within a file) for HDD vs SSD.
- Skip SDK/WPF downloads in v1 (game launches without them; add `--with-sdk --with-wpf` later if a
  channel proves otherwise). Dispatcher persistent writes are no longer skipped outright: they ship
  behind `--with-persistent` (§6). Always move audio cache→res if the
  game config names distinct dirs (Starward parity, prevents stranded duplicates).
- Structured logs + exit codes for automation; `--check-only` verify mode; resume-safe reruns
  (idempotent: intact files skipped by size+MD5).

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

## 6. Persistent-directory update — research findings

Recorded originally as a standalone research report, before this feature existed.
Its findings and decisions are now the basis of the shipped `--with-persistent`
opt-in tier (`README.md` documents the flag; docs/03 §7 holds the implementation
design); the repair-only default is unchanged. The evidence is kept here in full — marker
names, URLs, CDN shapes and the decision tables — so the flag's contract can be
audited. The decision tables record the state at research time and their chosen
answers; those answers are the ones the shipped tier implements.

This research answers three questions raised by the `girpr` repair-only design:

1. What are the "already verified" marker files the game writes into the install,
   and does `Collapse` write them?
2. What is the *full* process `Collapse` uses to update `Persistent/` (the in-game
   hot-patch path), which `Starward` does not implement?
3. Which prerequisite steps of the two launchers did `girpr` skip, and which of
   those become necessary once `girpr` updates `Persistent/`?

Scope is GI only (`hk4e_cn` / `hk4e_global` / `hk4e_bilibili`). All launcher code
references are to the checkouts in `reference-projects/`. Web findings are marked
as such and carry URLs. The original §4 verdict flagged the deliberate omission:

> "dispatcher persistent files ... game launches without them. Add `--with-sdk/
> --with-wpf` only if a channel's launch check proves otherwise."

That verdict was about *launchability*; this research is about *not re-triggering
the in-game verifier and not re-downloading hotfixes*. The persistent half of that
omission is now closed by the flag described here.

## 6.1. The two update mechanisms, and why `Persistent/` is different

GI has two content-delivery paths that are both live:

| | Launcher / Sophon path | In-game hot-patch path |
|---|---|---|
| Work list source | HYP `getBuild` → Sophon chunk manifests | **Game dispatcher** (`query_cur_region`) → `res_versions_external` + `data_versions` manifests |
| Bytes on disk | Game files under `*_Data/StreamingAssets/`, audio, etc. | Full replacement files under `*_Data/Persistent/` |
| Patch shape | Chunked, zstd-compressed, assembled at byte offsets | **Full files, plain HTTP, no patching** |
| Version state | `config.ini` `game_version` | `Persistent/{res,data,silence,audio}_revision`, `ScriptVersion`, `ChannelName`, `PatchDone` |
| Verification state | `pkg_version` (+ `res_versions_streaming`, `base_res_version_hash`) | `res_versions_persist` (+ `data_versions_persist`, `silence_data_versions_persist`, `*_versions*persist*`) |

`girpr` implements a *repair* of the first path only (Starward-style chunk
repair plus a Collapse-style extra-file purge). The second path was untouched by
that base design, and the purge actively deleted its files (§6.4.3) — which is
why the keep-set was widened for every run before the opt-in tier shipped.

The user's observation — "the game verifies the files upon start, the result is
stored inside the game folder, so copying the game after verification doesn't
cause reverification" — is explained by **three layers of state**, not by a single
boolean flag. The one that literally travels with a copied folder and suppresses
a full re-verify is `base_res_version_hash`:

* `*_Data/Persistent/base_res_version_hash` is the lowercase-hex MD5 of
  `*_Data/StreamingAssets/res_versions_streaming`. At startup the game logs
  `IsBaseResVersionHashChanged => baseResVersionHashInPersist = …, md5 in
  streamingResVersion = …` and starts a verification pass when the two differ
  (log quoted at <https://www.reddit.com/r/Genshin_Impact/comments/10zmgtb/>;
  a reversing report shows `CreateFileW` on that exact path at startup,
  <https://xia0ji233.github.io/2025/02/14/Game6>). This is the "already verified"
  marker for the **base install**. Collapse computes it exactly as above
  (`Fetch.cs:256-267`), but only *writes* it in its repair routine.
* The game's current asset database for hotfixed files is
  `*_Data/Persistent/res_versions_persist` (plus `data_versions_persist` and other
  `*versions*persist*` / `audio_versions*` files named in the game's own string
  table, <https://gist.github.com/khang06/622d020dec0484b2854ee4eef17b56c1>). It
  is an NDJSON list of assets with `remoteName`, `md5`, `fileSize` and, in newer
  versions, `hash` — an XXH64 digest (`PkgVersionProperties.xxh64hash` is
  deserialized from JSON `hash`, see
  `Hi3Helper.EncTool/Parser/AssetIndex/PropertyVersioning.cs:26-28`; Collapse
  prefers it over MD5 when present, `Check.cs:170-172`). It is
  *generated by the game on first launch after verification* and then reused.
* The revision files (`res_revision`, `data_revision`, `silence_revision`,
  `audio_revision`, both `res_revision` and its misspelled twin
  `res_revision_eternal`) tell the client which server revision that database
  corresponds to. If they match the dispatcher's current numbers, the game trusts
  the local database and skips the large resource check; if not, it re-checks
  (community reports call this the "2,580-file resource check") and re-downloads
  into `Persistent/`.

`Collapse` writes **both halves**. That is why a Collapse-repaired install does not
re-verify in-game, and why `girpr`'s install does.

---

## 6.2. Does `Collapse` write the marker files? — Yes, all of them

### 6.2.1 The revision / version block

`CollapseLauncher/Classes/RepairManagement/Genshin/Fetch.cs:252-293`
(`SavePersistentRevision`) writes, into `<game>/<exe-stem>_Data/Persistent/`:

| File | Value written | Source | Line |
|---|---|---|---|
| `base_res_version_hash` | lowercase hex MD5 of `StreamingAssets/res_versions_streaming` | computed locally | 257-267 |
| `DownloadPref` | binary template from Collapse's *encrypted* launcher metadata (`GetGameDataTemplate`) | preset | 269-274 — optional; not reproducible from public data and **not** a verification flag, so girpr should skip it |
| `data_revision` | `dispatchQuery.DataRevisionNum` (decimal text) | dispatcher proto `RepoDesignDataNumber` | 277 |
| `res_revision` | `dispatchQuery.ResRevisionNum` | dispatcher `RepoResVersionProperties.ResVersionNumber` | 279 |
| `res_revision_eternal` | same `ResRevisionNum` (the code comments that "eternal" is HoYo's own typo for "external") | dispatcher | 280-282 |
| `silence_revision` | `dispatchQuery.SilenceRevisionNum` | dispatcher `RepoDesignDataSilenceNumber` | 283-284 |
| `audio_revision` | `dispatchQuery.AudioRevisionNum` | derived from `{ClientGameResURL}/StandaloneWindows64/base_revision` | 285-286 |
| `ChannelName` | `dispatchQuery.ChannelName` (`OSRELWin` / `CNRELWin`) | preset + region id | 287-288 |
| `ScriptVersion` | `dispatchQuery.GameVersion` (e.g. `7.2.0`) | local version used for the dispatcher query | 289-290 |
| `PatchDone` | *same value as `audio_revision`* | dispatcher | 291-292 |

All are plain text with no trailing newline (`File.WriteAllTextAsync(...,
$"{value}")`), except `DownloadPref` which is a binary template and
`base_res_version_hash` which is a 32-char lowercase hex string.

`Repair.cs:108-111` calls `SavePersistentRevision` **after** the repair loop and
only if the dispatcher query succeeded (commit
[9cce3af](https://github.com/CollapseLauncher/Collapse/commit/9cce3af95a300b6d0316b7b8c3d507717f58b879)
made the write conditional so repair still works when no dispatch token is
available). `Repair.cs:113-125` then copies
`StreamingAssets/ctable.dat` → `StreamingAssets/ctable_streaming.dat`, and
`Fetch.cs:201-210` (`TryDeleteDownloadPref`) deletes a stale `Persistent/ctable.dat`
up front. Both are prerequisites the repair tool currently lacks.

**Phase scope matters:** this whole marker block is written only by Collapse's
**Genshin repair routine** — not by its Sophon update, not by predownload, and not
by a fresh install (the only install-side call is commented out in
`GenshinInstall.PkgVersion.cs:55-62`). A check that finds no broken files leaves
the old markers untouched. That is a strong precedent for making the marker
writes a repair-only post-phase in girpr too.

**There is no GI-specific "Persistent verified" marker.** All the files above are
*version-state* markers; none is a per-file integrity flag. (For contrast,
Collapse's Star Rail subsystem does write literal `<name>_<crc>.hash` "already
verified" marks for persistent audio/video assets —
`StarRailRepairV2.Repair.cs:25,96-100`.) The only GI fast-path is `UseFastMethod`,
which skips hash comparison entirely and checks size only (`Check.cs:184-188`).

### 6.2.2 The persistent asset database (the "hotfix marker")

`CollapseLauncher/Classes/RepairManagement/Genshin/Fetch.Persistent.cs:133-198`
(`ParsePersistentManifest`) does two things per dispatcher manifest:

```csharp
await using Stream networkStream = (await client.TryGetCachedStreamFrom(persistentUrls.RefUrl, ...)).Stream;
await using FileStream resStream = File.Create(persistentUrls.RefPath);   // <-- writes the marker
await using CopyToStream bridgeStream = new CopyToStream(networkStream, resStream, ...);
```

with:

* `RefUrl = {ClientGameResURL}/StandaloneWindows64/res_versions_external`,
  `RefPath = Persistent/res_versions_persist` (lines 81-83);
* `RefUrl = {ClientDesignDataURL}/AssetBundles/data_versions`,
  `RefPath = Persistent/data_versions_persist` (lines 97-99).

So `Collapse` writes the game's hotfix asset database itself, byte-for-byte from
the server manifest, instead of waiting for the game to generate it. The NDJSON
lines are then parsed into `PkgVersionProperties` and merged into the repair list
(`TryReplaceAssetToPersistent`, lines 213-237), overriding the same path from the
Sophon base manifest. Collapse does **not** write the `silence_data_versions_persist`
or `audio_versions*` variants that appear in the game's string table, even though
`QueryProperty.ClientDesignDataSilURL` is available (`DispatchHelper.cs:86-87`).
GenshinSlimmer does maintain `silence_data_versions_persist` (and discovers every
`*versions*persist*` / `audio_versions*` manifest), and
GI-Live-Hotfix-Downloader writes it from
`client_silence/General/AssetBundles/data_versions`. The shipped tier (docs/03
§7) therefore mirrors the silence manifest too; B6 is extended accordingly. `audio_versions*` stays
out of scope.

### 6.2.3 Other files in the same family

* `pkg_version` and `Audio_<lang>_pkg_version` in the game root are written as a
  *fake* list rebuilt from the Sophon manifest, because HoYo removed the old
  scattered zip/`pkg_version` files in 5.6
  (`GenshinInstall.PkgVersion.cs:112-190, 192-237`). This is the **launcher's**
  verification list, distinct from the in-game one. The removal is confirmed by
  `Starward`'s repair path: it can repair an older install to a newer version
  without writing or updating `pkg_version` at all, and the game still launches.
  No in-game step depends on the root file.
  **Live 7.2.0 refinement** (`tools/sophon-minimal-research`): the global Sophon
  `game` manifest still ships a root `pkg_version` (and all four
  `Audio_*_pkg_version` files) as ordinary tracked files, byte-identical to the
  torrent copy — HoYo removed the *scattered zips*, not the single root list.
  Starward's "repair without writing `pkg_version`" therefore works because the
  Sophon repair restores that file; girpr needs no synthesis (C2's keep rule is
  still required for the purge, §6.4.3). The same live check shows every
  `*_streaming` / `*_revision` / `svc_catalog` / `ctable_streaming.dat` input in
  §6.1 is a byte-identical Sophon base file, so `base_res_version_hash` is always
  recomputable after a Sophon repair; a Sophon-only install is the torrent game
  minus the `BeyondAssets/` UGC editor and is runnable.
* `audio_lang_14` in `Persistent/` is the voice-language selection file; Collapse
  reads it to decide which audio manifests to include
  (`Fetch.cs:80-83, 106-133`) and girpr reads the equivalent path from
  `getGameConfigs.audio_pkg_scan_dir`.

### 6.2.4 Independent confirmation from the wild (web)

The community tool **GenshinSlimmer** documents exactly this mechanism and exploits
it deliberately:

* "Launch the game once to the start door, so it generates the new
  `res_versions_persist`."
* "Manifest Synchronization Engine: ... direct `res_versions_persist`
  manifest synchronization. Stubbed 0KB files are registered as valid 0KB assets
  in the game's own asset database, passing integrity verification instantly and
  permanently stopping in-game patch re-downloads."
* "In-Game Revision Manager: ... synchronize internal revision numbers
  (`res_revision`, `silence_revision`, `data_revision`) ... to completely bypass
  the 2,580-file resource check."
* It also deletes `Persistent/*versions_remote*` ("unpatched server manifests")
  and `Persistent/VideoAssets/*.tmp` ("partial in-progress download queues") to
  stop a re-download loop, and queries `query_cur_region` directly with
  `dispatchSeed` / `key_id=5` and an RSA private key.

Source: <https://github.com/dnullptr/GenshinSlimmer> (README and `GenshinSlimmer.ps1`,
v14). This corroborates that the marker pair is (database file + revision numbers),
and that writing them correctly is sufficient to suppress the in-game check.

Two field reports and the game's own string table sharpen this:

* [GenshinSlimmer issue #19](https://github.com/dnullptr/GenshinSlimmer/issues/19)
  records that the client checks `res_revision_eternal` **alongside**
  `res_revision` — updating only the latter does not stop the re-download
  (PR #20 added the dual write). It also describes the failure loop in detail:
  on a new server revision the client pulls `res_versions_remote`, writes partial
  `Persistent/VideoAssets/*.tmp` files, and keeps treating cutscenes as full-size
  until both are removed. Those two families are therefore **harmful stale state**,
  not inert clutter.
* [Collapse issue #166](https://github.com/CollapseLauncher/Collapse/issues/166)
  quotes a real game log:
  `IsBaseResVersionHashChanged=> baseResVersionHashInPersist = …, md5 in
  streamingResVersion = …` followed by `Package has changed!!!!! Clear state
  files in persist dir!!!!!`. A `CreateFileW` hook log in
  <https://xia0ji233.github.io/2025/02/14/Game6/> shows the game itself opening
  `Persistent/base_res_version_hash` at startup. This is direct evidence the
  game reads the marker and **wipes** the Persistent state on mismatch.
* The game string table at
  <https://gist.github.com/khang06/622d020dec0484b2854ee4eef17b56c1> names
  `res_versions_persist`, `data_versions_persist`, `silence_data_versions_persist`,
  `res_revision`, `data_revision`, `silence_revision`, `ChannelName`,
  `ScriptVersion`, `base_res_version_hash`, `cache_versions*`,
  `*versions_remote*`, `ctable.dat` and `ctable_streaming.dat` in one cluster,
  plus the game's own `md5 not matched` / `streaming md5 not matched` errors.
  It does **not** name `PatchDone`, `DownloadPref`, `audio_revision` or the
  "eternal" twin in that dump — treat those as lower-confidence Collapse-parity
  entries. The "2,580" figure remains a single-source (GenshinSlimmer) claim; the
  mechanism is corroborated, the number is not.

### 6.2.5 Starward writes none of them

A full search of `reference-projects/Starward` finds **zero** occurrences of
`Persistent`, `res_versions`, `data_versions`, `PatchDone`, `ScriptVersion`,
`ctable`, `verified`, `hotfix` or a game-side dispatcher. Its only game-folder
writes are `config.ini` (`game_version`, `channel`, `sub_channel`, `cps`,
`sdk_version`, `game_biz`, `wpf_version`, `predownload`), the audio scan/selection
file, audio cache→res moves, SDK/WPF extractions, and transient cleanup artifacts
(`*_tmp`, `*.hdiff`, `chunk/`, `ldiff/`, `staging/`, `deletefiles.txt`,
`hdiffmap.json`, `hdifffiles.txt`). Its dead localization string
`DownloadGamePage_SkipVerification` is never called from code; there is no
mark-as-verified feature. So Starward neither writes markers nor updates
`Persistent`, confirming the premise in the request.

---

## 6.3. `Collapse`'s `Persistent` update, end to end

The full chain, in call order. Paths are relative to `<game>`; `{exe}` is
`Path.GetFileNameWithoutExtension(GamePreset.GameExecutableName)`
(`GenshinRepair.cs:39`), i.e. `GenshinImpact` for global.

### Step 0 — Preset prerequisites

`GenshinRepair.cs:39-47` pulls `DispatcherRegionID` (from
`GetRegServerNameID()`), `DispatcherURL`, `DispatcherKey`,
`DispatcherKeyLength`, and the `{exe}_Data/Persistent` /
`{exe}_Data/StreamingAssets` paths from the game preset. The URL/keys are in
Collapse's runtime metadata (`PresetConfig.cs:353,369,372,378,482,533`), fetched
from its release repo and decrypted there; they are not hard-coded in the source
tree.

### Step 1 — Repair entry

`GenshinRepair.cs:68-121`: `StartCheckRoutine` → `CheckRoutine` →
`ResetAndFetchAssets` → `Fetch`. `ResetAndFetchAssets` clears the previous asset
index/Sophon refs and calls `TryUnassignReadOnlyFiles(GamePath)` — a prerequisite
for overwriting protected hotfix files.

### Step 2 — Base manifest

`Fetch.cs:137-198` (`BuildPrimaryManifest`): deletes stale `Persistent/ctable.dat`,
gets `getBuild` branch URLs from the preset, fetches and parses the main Sophon
manifest, and rebuilds a fake `pkg_version` plus a `SophonAssetDictRef` map of
base assets. Every base asset is added with `isForceStoreInStreaming=true`,
`isForceStoreInPersistent=false`.

### Step 3 — Dispatcher query

`Fetch.cs:298-341` + `Hi3Helper.EncTool/Parser/YSDispatchHelper/DispatchHelper.cs`:

1. `DispatchHelper` builds
   `string.Format(DispatchURLPrefix, regionSubdomain, $"{channel}{version}", dispatchKey)`.
   Region subdomains are `osusadispatch` / `oseurodispatch` / `osasiadispatch` /
   `oschtdispatch` / `cngfdispatch` / `cnqddispatch` (lines 173-194); channel is
   `OSRELWin` (global) or `CNRELWin` (region id ≥ 4, lines 43-46). GenshinSlimmer
   shows the concrete server shape:
   `https://osusadispatch.yuanshen.com/query_cur_region?version=OSRELWin7.2.0&...&dispatchSeed=<seed>&key_id=5`,
   returning `{"content": "<base64 RSA-encrypted protobuf>"}`.
2. The response is decrypted (`YSDispatchDec.DecryptYSDispatch` with
   `DispatcherKey`/bit-length) and parsed as `GenshinGateway` protobuf
   (`.../Proto/GenshinGateway.proto`): `GatewayProperties` (tag 3) with
   `RepoResVersionURL` (8), `RepoResVersionProperties` (22: `ResVersionNumber`,
   `ResVersionMapJSON`, `ResVersionHash`), `RepoDesignDataURL` (9),
   `RepoDesignDataNumber` (14), `RepoDesignDataJSON` (19),
   `RepoDesignDataSilence*` (18/20/27).
3. `LoadDispatch` (lines 76-98) turns those into:
   * `ClientGameResURL = {RepoResVersionURL}/output_{resNum}_{resHash}/client`
   * `ClientDesignDataURL = {RepoDesignDataURL}/output_{dataNum}_{dataHash}/client/General`
   * `ClientAudioAssetsURL = {RepoResVersionURL}/output_{audioRev}_{audioHash}/client`,
     where the audio revision/hash come from a second GET to
     `{ClientGameResURL}/StandaloneWindows64/base_revision` (lines 153-166, response
     is `"<revision> <hash>"`), and `AudioRevisionNum` is that revision.
   * the four revision numbers + `ChannelName` + `GameVersion`.
4. `GetCachedDispatcherQuery` (`GenshinRepair.cs:154-164`) memoises the query for
   five minutes.

### Step 4 — Persistent manifests and file placement

`Fetch.Persistent.cs:45-118, 133-198`:

1. `CreateDirectory` for both `Persistent/` and `StreamingAssets/`.
2. Two manifests are fetched and mirrored to disk as `res_versions_persist` and
   `data_versions_persist` (§6.2.2).
3. For each NDJSON line that is a JSON object, `GetRelativePathByRemoteName`
   (lines 239-278) maps by suffix:
   * `*.pck` → `AudioAssets/`, asset type Audio;
   * `blocks*.blk` / `*svc_catalog` → `AssetBundles/`, type Block;
   * `*.usm` / `*.cuepoint` → `VideoAssets/`, type Video;
   * anything else → `""` (install root), type Generic.
4. `asset.isPatch` decides the base directory: `Persistent/` for patches,
   `StreamingAssets/` for base files (lines 161-164). Note `isPatch` is
   `[JsonIgnore]` (`PropertyVersioning.cs:29-30`) — **it never comes from the
   dispatcher manifest**; it is purely an internal decision made in step 5.
5. `IsRequireForcePersistent` (lines 200-211): if the same local path already
   exists in the base Sophon manifest with a *different MD5* (case-insensitive
   compare), the dispatcher file is a hotfix and is forced into `Persistent/`
   (`isPatch = true`, `isForceStoreInPersistent = true`). This MD5 comparison is
   the semantic core of the whole feature.
6. With `isPatch` known, `RelativePathSelector` (lines 120-131) chooses the
   primary/alternate download CDN: for `res_versions_external` the normal file
   source is `ClientAudioAssetsURL/StandaloneWindows64` and the *patch* source is
   `ClientGameResURL/StandaloneWindows64`; they swap when forced to Persistent.
   For `data_versions` both are `ClientDesignDataURL`. URLs are assembled as
   `{primary}/{relativePath}/{original remoteName}` **before** `remoteName` is
   overwritten with the local path (line 193).
7. `TryReplaceAssetToPersistent` (lines 213-237) adds or replaces the entry in the
   shared asset index by local path, so the repair loop sees one winner per path.
   A forced-persistent entry has a different path and is appended alongside the
   untouched StreamingAssets entry.

### Step 5 — Post-fetch filtering (`Fetch.cs:65-103`)

Force-fetch BiliBili SDK if present; remove plugin assets; if the persistent parse
succeeded, drop unselected voice languages using `Persistent/audio_lang_14`;
exclude `ctable_streaming.dat` and any `ctable.dat` without a URL.

### Step 6 — Check (`Check.cs:26-80`)

Delete `DXSETUP/` (it causes false-positive 4302-x integrity errors, lines 82-89);
move *non-patch* `.pck`/`.usm`/`.cuepoint` files from `Persistent/` back to
`StreamingAssets/` (lines 91-145); mark `*deletefiles*`, `*.diff`, `*_tmp`,
`*.hdiff` as `Unused` (lines 233-319); then for every asset compare size and
hash (XxHash64 when available, else MD5, lines 148-229) and keep only broken ones.

### Step 7 — Repair (`Repair.cs:28-219`)

For each broken asset, in bounded parallel:

* `Unused` → delete;
* no `remoteURL` → Sophon chunk path (`SophonAsset.WriteToStreamAsync`);
* otherwise → **plain HTTP full-file download** from `remoteURL` with
  `remoteURLAlternative` fallback on 404 (`RunDownloadTask`), opening the final
  path directly. There is no patching in `Persistent/`, and — worth noting as a
  Collapse weakness rather than a model to copy — **no post-download hash
  validation on this HTTP path**; only the next check pass would catch a bad
  download. girpr should re-verify after writing.

### Step 8 — Markers and bookkeeping

`SavePersistentRevision` (§6.2.1), then the `ctable.dat` → `ctable_streaming.dat`
copy. A `config.ini` bump belongs to the launcher's versioning layer, not to
`GenshinRepair` itself.

---

## 6.4. Gap analysis: what `girpr` skips, and what it now needs

`girpr`'s pipeline is `metadata → manifest → plan → [purge-before] → repair →
post` (`src/repair/mod.rs`; see the README action graph). Its live API surface is
`getGameConfigs`, `getGameBranches`, `getBuild` (latest + local) and
`getGameDeprecatedFileConfigs` (`crates/fetch/src/hyp.rs:9-10, 463-491`).
The "girpr today" column records the research-time baseline; the gaps it names
are what the shipped `--with-persistent` tier fills, while the default path
remains as described.

| # | Launcher prerequisite | Collapse | Starward | girpr today | Needed for Persistent? |
|---|---|---|---|---|---|
| 1 | Dispatcher `query_cur_region` + RSA/protobuf decode + per-region keys/subdomains | yes (Step 3) | no | **no** | **Required** — the only source of the hotfix file list and revisions |
| 2 | Persistent manifests `res_versions_external` / `data_versions` | yes | no | **no** | **Required** — defines which files + their MD5/size/URL |
| 3 | Save those manifests as `res_versions_persist` / `data_versions_persist` | yes | no | **no** | **Required** — the game's asset database / verification marker |
| 4 | Write revision/version marker files (§6.2.1) | yes | no | **no** | **Required** — without them the game re-verifies or re-downloads |
| 5 | `base_res_version_hash` + revision markers | yes | no | **no** | **Required** — base-install verification marker; `DownloadPref` is a launcher-metadata blob and can be skipped |
| 6 | Extension-based placement + force-persistent override | yes | no | **no** | **Required** — decides StreamingAssets vs Persistent |
| 7 | Full-file plain-HTTP downloader with alt URL | yes | no (Sophon only) | **no** (Sophon chunk streaming only) | **Required** — Persistent files are full files, not chunks |
| 8 | Strip read-only flags before overwrite | yes (tree-wide) | sets attributes Normal pre-task | partial (per repaired file only) | Required — hotfix files are often read-only |
| 9 | Create `<exe>_Data/Persistent` **and** `StreamingAssets` before fetching | yes (`Fetch.Persistent.cs:63-64`) | no | partial (game root only) | **Required** — both trees must exist before the plan is built |
| 10 | Delete stale `Persistent/ctable.dat`; delete `DXSETUP/`; copy `ctable.dat` → `ctable_streaming.dat` | yes | no | **no** | Cheap prerequisite; avoids false integrity failures |
| 11 | Move non-patch `.pck`/`.usm`/`.cuepoint` out of Persistent | yes | no | **no** | Recommended — avoids stranded duplicates after a hotfix ends |
| 12 | Read/write `audio_lang_14` to filter voice assets | yes | reads via config | reads via config; writes only with `--audio` | Already mostly present; verify it is the same path |
| 13 | `config.ini` version bump | yes | yes | yes (unscoped post-phase) | Already present — keep ordering consistent with marker writes |
| 14 | Deprecated-file delete / extra-file purge | yes (manual UI) | yes | yes (`--purge-*`) | **Conflict** — see §6.4.3 |
| 15 | SDK/WPF/BiliBili/plugin zips, quota dialog, speed limiter, in-game manifest cache | yes | partly | no | Not needed for launch; but if extending the purge keep-set, consider SDK/WPF/plugin files too |
| 16 | `pkg_version` rebuild from Sophon | yes | reads legacy | no (no rebuild) | Not needed for the in-game path, but **must at least be kept** by the purge — see §6.4.3 |

### 6.4.1 Why the dispatcher step is the hard one

It is not another HYP JSON call: the response is an **encrypted protobuf** with a
region-specific RSA key and a `dispatchSeed`/`key_id` that changes with game
version. `girpr` would have to either port `Hi3Helper.EncTool` + `GenshinGateway.proto`
(New dependency surface: RSA decryption, protobuf, Brotli metadata) or embed a
maintained copy of the key/seed data. GenshinSlimmer demonstrates a pure-PowerShell
implementation and notes the seed is versioned (`7.2.0 = 018bc9560abc707f`,
`7.0.0 = d9f572e5464f160f`) with a fallback to reading
`%LOCALAPPDATA%/miHoYo/Genshin Impact/info.txt`. Collapse keeps the values in its
remote metadata (`config_master.json` / `config_GIGlb.json`) and exposes them as
`ProtoDispatchKey`. This is a policy decision for the implementation phase, not a
research gap.

### 6.4.2 Where the new phases fit

A minimal, pipeline-preserving design:

1. **Metadata phase** — after `getGameConfigs`, fetch/decode the dispatcher query.
   Treat failure as best-effort (Collapse itself warns and continues), but then
   Persistent support is unavailable for that run and must be reported.
2. **Manifest phase** — fetch the two persistent manifests, verify them by MD5 if
   the dispatcher provides one, mirror them to `<exe>_Data/Persistent/*_persist`,
   and produce `PersistentAsset` entries merged with the Sophon plan by local
   path (force-persistent wins).
3. **Repair phase** — persist assets take a full-file path: verify size+MD5 (the
   manifest's `md5`/`hash`), skip if intact, otherwise download from
   `remoteURL`/alternate over plain HTTP, then re-verify. The existing
   `--repair-mode` / byte-ceiling / lock machinery should account for these bytes.
4. **Post phase** — write all markers **after** every persistent file has passed
   its gate, then bump `config.ini`. Mirror Collapse's ordering (markers last): a
   failed run must not claim a revision it did not install.
5. Scope and read-only flags: `--check-only` must check persistent files too;
   `--dry-run`/`--print-plan` must list them without writing markers;
   `--include`/`--exclude` must not silently drop the marker writes from an
   otherwise in-scope Persistent update.

### 6.4.3 The purge conflict (highest regression risk)

This is the pre-fix state that motivated the run-wide keep-set now in place
(`README.md`); the opt-in tier cannot be built on top of a purge that deletes its
own markers. `src/repair/purge.rs:1-55, 348-360, 395-443` documents the v1 gap itself:

> "V1 GAP (deliberate, documented): Collapse's expected set is the union of the
> Sophon primary manifests + dispatcher persistent manifests ... v1 only uses
> `{latest Sophon manifest paths}` — no dispatcher ..."

The keep rules are: manifest membership, `config.ini` + legacy lock, blacklist,
and `audio_lang_*`/`Audio_*_pkg_version`. **Every file in `Persistent/` — including
`res_versions_persist`, all revision files, and every downloaded hotfix — is
currently classified `Purge`** when `--purge-after`/`--purge-only` runs. `pkg_version`
in the game root is likewise not a keep category (only `Audio_*_pkg_version` is
matched by pattern). Adding the Persistent updater without extending the expected
set would make the tool delete its own work in the same run, and on an untouched
install would delete the game's existing verification markers (`base_res_version_hash`
in particular), forcing a full re-verify at next launch. The HoYoPlay launcher's
own list is `pkg_version` (per GenshinSlimmer), so deleting that can likewise invite
a full launcher-side re-check.

Required changes: add the dispatcher-derived persistent paths **and** the marker
file names (`res_versions_persist`, `data_versions_persist`, `silence_data_versions_persist`,
`audio_versions*`, `*_persist`, `*_revision`, `ChannelName`, `ScriptVersion`,
`PatchDone`, `base_res_version_hash`, `audio_lang_*`, `cache_versions_*`, `ctable*`)
to the expected/keep inputs, plus `pkg_version`; treat `*versions_remote*` and
`VideoAssets/*.tmp` as purge candidates (they are game-generated partial-download
state, per GenshinSlimmer). Do **not** add Sophon's `preload.verified` / `*.verified`
— those are Collapse-internal resume state that the game never reads. Consider
adding SDK/WPF/plugin files to the keep set at the same time, since widening the
keep inputs is the natural moment to revisit the documented v1 purge scope.

Keeping `*versions_remote*` and `VideoAssets/*.tmp` as **purge candidates** is
load-bearing, not cosmetic: GenshinSlimmer issue #19 records that those leftovers
keep telling the client the cutscenes should be full size, so a 40–50 GB
re-download resumes even after the revisions are corrected. Any future widening
of the keep-set must not accidentally protect them.

### 6.4.4 Caveats from Collapse's implementation (do not copy blindly)

* `*_persist` manifests are written with `File.Create` **before** parsing, so a
  failed/truncated download still leaves a partial marker file. girpr should
  write to a temp file and publish atomically, and gate marker writes on a
  successfully parsed manifest.
* `base_res_version_hash` is only written when
  `StreamingAssets/res_versions_streaming` exists — on an install missing that
  file, Collapse simply omits the marker. Decide explicitly how girpr handles
  that case. The C16(a) omit branch is effectively unreachable on a
  Sophon-repaired install: `res_versions_streaming` is itself a tracked base
  file (live 7.2.0 MD5 `f5ceb98fd93b8c69603d7b5600d11a08`, byte-identical to
  the torrent), so keep the branch for malformed inputs but do not expect it in
  normal operation.
* The dispatcher query is cached for five minutes (`GenshinRepair.cs:154-164`),
  so a repair started inside that window can write slightly stale revisions.
  girpr's single-shot run does not need the cache.
* Manifest entries are NDJSON objects with `remoteName`, `md5`, `hash` (XXH64)
  and `fileSize`; Collapse marks `isPatch`/force flags `[JsonIgnore]` and
  recomputes them, although the live dispatcher does send `isPatch` on some rows
  (§6.4.5). A blank line in a dispatcher manifest actually throws in Collapse
  (`IsCurrentLineJson` indexes `chars[0]`/`chars[^1]`), aborting all persistent
  processing — girpr should skip blank lines.
* Collapse's HTTP repair path does no post-download hash validation (§6.3 Step 7);
  girpr's whole-file MD5 gate is a strict improvement worth keeping.
* `audio_lang_14` is written by the installer, not by the repair routine
  (`InstallManagerBase.Sophon.cs:1337-1375`); repair only reads it.

### 6.4.5 Server-side confirmation: no Sophon for Persistent hotfix bytes

Collapse's Sophon-vs-HTTP choice is a *client-side* gate (`Repair.cs:167` routes
on `string.IsNullOrEmpty(asset.remoteURL)`), so the code alone cannot prove the
server has no Sophon path for the same bytes. A live probe
(`tools/persistent-sophon-research/`, 2026-11-23 UTC, `hk4e_global`,
`query_cur_region` res revision `49530578`, `OSRELWin7.2.0`) settled this:

* The dispatcher query was fetched with the key/seed unwrapped from Collapse's
  live release metadata (the RSA key is byte-identical to GenshinSlimmer's copy),
  decrypted and decoded in full, including the fields the local `.proto` omits
  (root 11/12/14, properties 38–47). None contains a Sophon/chunk/manifest pointer.
* Both persistent manifests were downloaded and MD5-checked against the
  dispatcher (3,164 rows: 2,631 `res_versions_external` + 533 `data_versions`).
  The server **does** send `isPatch` on 10 rows — Collapse's parser discards it
  (`PropertyVersioning.cs:29-30`); girpr should tolerate it.
* **Every** Sophon manifest advertised by the live API was downloaded
  (15 chunk manifests across builds 7.2.0/7.1.0/7.0.0 + 5 `getPatchBuild` patch
  manifests) and cross-matched by normalized path and MD5. **Zero** hotfix MD5s
  occur in any of them. The 1,661 path+MD5 matches are base-install copies in
  `StreamingAssets/` that Collapse's MD5-diff rule keeps out of `Persistent/`;
  the remaining 376 dispatcher-only MD5s (369 replaced + 7 new paths) are the
  genuine hotfix set, including all 10 `isPatch` rows.
* CDN probes agree: real Sophon chunks return 200 at the advertised prefix but
  404 under dispatcher-derived `output_<rev>_<hash>` paths; hotfix full files
  return 200 with the exact manifest MD5, ignore `?chunk=`/`?sophon=`, and answer
  `Range` with 206.

**Consequence for girpr:** there is no Sophon repair to fall back to for a
`Persistent/` file. A full-file HTTP downloader (primary + alternate URL, with a
post-download size/hash gate) is **mandatory**, not an implementation detail —
the chunk-streaming repair core cannot be reused for hotfixes. This confirms
gap row 7 in §6.4 and the design in §6.4.2 step 3. The hotfix CDN supports `Range`,
so resumable downloads are an optional improvement; Collapse itself does not use
them.

Scope/caveats: one region, client version and revision; an unadvertised private
manifest cannot be disproved, and CN/Bili regions were not probed. Reproduction
scripts and exact URLs/statuses are in `tools/persistent-sophon-research/README.md`.

---

## 6.5. Opt-in `Persistent` update: conflicts and decision blockers

The feature is **opt-in**: one master flag, `--with-persistent` (B1 chose this
single-flag envelope), leaves the repair-only default byte-identical — same plan,
same `SUMMARY`, same exit codes. This section records the conflicts and blockers
that were settled before that flag was implemented:

* **§6.5.1 conflicts** — places where the new work (or its §6.4 prerequisites) meets
  an existing `girpr` rule that already has a defined meaning. Each row gives the
  competing meanings, the semantics this research suggests, and an **Answer**
  column holding the final call.
* **§6.5.2 blockers** — choices that reading the two launchers or the live probe
  cannot settle. The §6.4 prerequisites are deliberately **not** blockers: the
  decoder, the three manifests (res, data, silence), the full-file downloader,
  the marker writes, the
  read-flag stripping and the directory/ctable/DXSETUP work are implementation
  tasks, not open questions. Likewise, capturing a dispatcher response and all
  three manifests into a `gircap` bundle (§6.6 step 2) is a prerequisite
  for testing, not a decision.

"Suggested" is the report's recommendation; the last column records the chosen
answer. B4 is conditional: (b) is the launch target, with (a) as the fallback if
implementation-time research cannot verify CN/Bili.

A second pass over the published outputs added C19, C20 and B9 and folded a
further scope question into C7 (the former C18). All are now decided: (a) for
C19 and B9, the combined (d)+(f) for C7, and (d) for C20 (revised from (a) —
the keep set needs the manifests even in a cleanup-only run). Each changes a published
result (marker bytes, `SUMMARY`, or an exit code); none is a schema or
implementation question. C19 resolves the remaining tension with the Collapse
precedent in §6.2.1; C7's merged row records the C7(d)-vs-C8 tension and how it
was settled.

Two answers deliberately **do not** copy Collapse, and should not be "fixed" to
match it later: C8's all-or-nothing gate (Collapse's `Repair.cs:108-111` calls
`SavePersistentRevision` whenever the dispatcher query succeeded, even if the
persistent-manifest parse failed or no file was broken — see also its 5-minute
query cache at `GenshinRepair.cs:154-164`) and C19's always-rewrite. Both are
recorded decisions in §6.5.1, not drift.

### 6.5.1 Conflicts with existing `girpr` functions and contracts

| # | Conflict: new/prerequisite vs. existing `girpr` rule | Possible semantics | Suggested semantic | Answer |
|---|---|---|---|---|
| C1 | **Purge keep-set vs. Persistent files and markers.** `--purge-after`/`--purge-only` currently keep only live-manifest paths, `config.ini`, the legacy lock, the blacklist and `audio_lang_*`/`Audio_*_pkg_version`; every `Persistent/` file is a purge candidate (§6.4.3). | (a) widen the keep-set for every run; (b) widen only when `--with-persistent` is passed; (c) leave the purge alone and document the loss. | **(a)** — widening is keep-only, so it cannot write wrong bytes; (b) would let an unflagged run delete the markers of an install that was already repaired. | **(a)** — chosen. |
| C2 | **Root `pkg_version` vs. purge.** Only `Audio_*_pkg_version` is a keep pattern today; the root `pkg_version` is swept, although the HoYoPlay launcher uses it (§6.4.3). | (a) always keep it; (b) keep only with `--with-persistent`; (c) no change. | **(a)** — same reasoning as C1; this is a correctness fix for all runs, not part of the feature. | **(a)** — chosen. |
| C3 | **Plan-half "touches nothing" vs. mirroring the persistent manifests.** Collapse writes `*_persist` inside its fetch step; `girpr` allows exactly one plan-half write (`--plan-out`). | (a) fetch and parse in the plan half, publish the mirrored files in the execute/post half; (b) write them in the plan half as a flag-scoped exception; (c) never mirror, let the game generate the database. | **(a)** — the plan stays inspectable and read-only; (c) gives up the marker that suppresses re-verification. | **(a)** — chosen. |
| C4 | **Read-only modes vs. marker writes.** `--dry-run`, `--check-only` and `--print-plan` write nothing (README, "read-only modes"). | (a) all three list persistent work but write no markers or manifest mirrors; (b) `--dry-run` writes markers; (c) `--check-only` writes markers for intact files. | **(a)** — a read-only mode that writes verification state is a contradiction, and the design in §6.4.2 step 4 puts marker writes last. | **(a)** — chosen. |
| C5 | **What "install intact" means under `--check-only`.** The check currently verifies the Sophon plan only; with the flag on, a byte-intact install with a stale/missing revision file will still re-verify in-game. | (a) check persistent files *and* the marker block; (b) check file bytes only; (c) keep Sophon-only. | **(a)** — with `--with-persistent` on, "intact" must include the state the feature exists to maintain; otherwise exit `0` claims a result the game will not honour. This narrows the set of installs `--check-only` calls clean, which should be stated in the help text. | **(a)** — chosen. |
| C6 | **Dispatcher failure vs. the exit-code table.** `1` usage, `2` metadata/network, `3` write, `4` check-only damage, `5` ceiling. A failed RSA/protobuf decode is none of the HYP/Sophon metadata calls. | (a) exit `2` with a distinct log line; (b) exit `1` ("cannot satisfy this request in this environment"); (c) exit `0` with a degraded report. | **(a)** — it is a server-metadata failure; (c) would report success for a request that was not performed. | **(a)** — chosen. |
| C7 | **Path scope (`--include`/`--exclude`) vs. install-wide markers.** A scoped run already forgoes the `config.ini` bump. Explicit `--audio` under a scope is **not** refused outright: it is exit `1` only when the scan file it must write (`getGameConfigs.audio_pkg_scan_dir`) is outside the scope; with that file in scope the run proceeds (`src/repair/phases/metadata.rs:465-479`, `tests/repair_scope.rs:225-268`). Marker files (the §6.2.1 block plus the two `*_persist` manifests) are install-wide state, but like the audio list they do not guarantee that out-of-scope files were installed. **Merged from the former C18:** (d) answers where the marker *files* may be written, but a revision *value* asserts that every hotfix at that revision is installed — C8's reason for the all-or-nothing gate — so a scope that admits the marker paths while excluding any dispatcher hotfix path publishes a false assertion and the game trusts a partially installed revision. | Marker-file scope: (a) refuse `--with-persistent` + any active scope (exit `1`); (b) allow, suppress markers and the bump, report `config_bumped=0`-style; (c) allow and write markers unconditionally; (d) follow the `--audio` precedent — refuse only when any marker file the run would write is outside the scope (matched on cleaned rel paths), otherwise write them. Dispatcher-path scope: (e) write markers anyway — (d) taken literally; (f) suppress the marker block and manifest mirrors whenever any dispatcher hotfix path in the current persistent set is out of scope, and report it; (g) refuse the combination (exit `1`). | **(d)** for the marker-file question — `girpr` already accepts exactly the `--audio` case, and a scoped run already suppresses the `config.ini` bump, so the partial state stays visible; refusing every scoped run would be stricter than any existing rule. **(f)** for the dispatcher-path question — C8's invariant is load-bearing: the marker *file* is harmless to write, but the revision *value* tells the game to skip its check and must describe a complete revision. Suppressing mirrors the scoped `config.ini` rule (`SUMMARY config_bumped=0`), and unlike (g) it keeps (d)'s chosen "allow scoped runs" semantics; (e) is the only option that can hand the game a knowingly incomplete install. Both suppressions must be visible on the data plane. | **(d)** for marker-file scope and **(f)** for dispatcher-path scope — chosen. |
| C8 | **Per-file failure gate vs. all-or-nothing markers.** Any failed file already returns exit `3` and skips the post-phase, so `config.ini` never advances over damage. | (a) one global gate in the post phase: no marker block if any persistent file failed; (b) write markers for the files that passed; (c) write markers unconditionally. | **(a)** — a revision file asserts that *all* files at that revision are present; a partial assertion is worse than a stale one. | **(a)** — chosen. |
| C9 | **Marker write order vs. `config.ini` bump.** `ScriptVersion` records the game version; the bump is the last post-phase act. | (a) markers first, bump last, both skipped on failure; (b) bump first; (c) no bump when only markers changed. | **(a)** — preserves the existing "version advances only after the bytes are in place" invariant and Collapse's "markers last" ordering within that window. | **(a)** — chosen. |
| C10 | **`--repair-mode` vs. full-file downloads.** `auto`/`inplace`/`temp`/`discard` and their `SUMMARY` keys describe the chunk writer. Persistent files are full-file HTTP with a post-download MD5 gate (§6.4.5). | (a) persistent uses its own path; the mode group describes base files only; (b) apply the mode to persistent files too; (c) add a mode value. | **(a)** — do not pretend a chunk mode governs a path that has no chunks; document that the mode group excludes persistent bytes so the `SUMMARY` is not read as covering them. | **(a)** — chosen. |
| C11 | **Byte ceilings vs. persistent bytes.** The download/write/verified counters and `--max-files` count chunk-plan files only; a `--max-download-bytes 0` run must not be able to download hotfixes. | (a) fold persistent file sizes into the existing estimates and live counters, with the same pre-flight refusal and runtime stop; (b) add parallel `--max-persistent-*` ceilings; (c) exclude persistent bytes. | **(a)** — a ceiling that can be bypassed by a new code path is a broken contract; the full sizes are known from the manifest, so the pre-flight refusal stays exact. B7 covers the contract change this implies. | **(a)** — chosen. |
| C12 | **"Unflagged `SUMMARY` is byte-identical" vs. C1/C2.** Keeping markers and `pkg_version` changes `deleted_extra_bytes` on an install that contains them, even for a run with no new flags. | (a) accept the change and document it as a deliberate safety fix; (b) gate the keep-set on `--with-persistent`; (c) add a separate keep-only flag. | **(a)** — the byte-identical promise is about flags, not about installs whose contents changed; silently deleting verification state to preserve a number is the wrong trade. B5 records the decision. | **(a)** — chosen. |
| C13 | **`--scan` and byte reporting.** The scan reads every planned file once and publishes `scan_bytes` and exact estimates; persistent files are not in the Sophon plan. | (a) add persistent files to the scan; (b) exclude them and state that their full-file MD5 gate is the check; (c) a separate persistent scan pass. | **(b)** — the repair already reads each persistent file in full for its MD5 gate; scanning them again doubles the I/O for figures that are already exact from the manifest. Make the exclusion visible in `PLAN`/`--plan-out`. | **(b)** — chosen. |
| C14 | **Audio selection vs. persistent audio filtering.** `.pck` rows are filtered by `Persistent/audio_lang_14`; `girpr` already reads `audio_pkg_scan_dir` via `getGameConfigs` (§6.4 row 12). | (a) reuse `meta.audio` and the existing scan-file read; (b) write a second `Persistent/audio_lang_14`; (c) include every language. | **(a)** — one source of truth, read-only; first verify the two paths are the same file before assuming parity. | **(a)** — chosen. |
| C15 | **Atomic publish vs. purge classification of `*_tmp`.** `purge` treats `*_tmp` as a candidate; Collapse also leaves partial `*_persist` files on a failed download (§6.4.4). | (a) download to a temp name, full-file MD5, then atomic publish before the post-repair re-plan; (b) write the final path directly; (c) add a persistent-`_tmp` keep pattern. | **(a)** — util-path's atomic publish already exists; (b) is exactly the partial-marker failure §6.4.4 warns about. | **(a)** — chosen. |
| C16 | **Missing `res_versions_streaming` vs. `base_res_version_hash`.** Collapse simply omits the marker when the source file is absent (§6.4.4); `girpr` normally refuses malformed inputs at plan time. | (a) omit that one marker and report it; (b) refuse the run; (c) synthesize the hash from the Sophon manifest. | **(a)** — match the game's and Collapse's behaviour; make the omission visible on the data plane rather than failing a repair that is otherwise sound. | **(a)** — chosen. |
| C17 | **Tree-wide read-only stripping (§6.4 row 8) vs. per-file clearing.** Collapse calls `TryUnassignReadOnlyFiles` on the whole game tree up front; `girpr` clears the flag only when an in-place open fails, and otherwise falls back to `{path}_tmp` for that one file. Hotfix files are often read-only. | (a) strip read-only flags tree-wide when the flag is on; (b) strip only on persistent paths, before overwriting each file; (c) keep per-file clearing and rely on the temp path. | **(b)** — a tree-wide attribute change is a surprising mutation of files the run does not otherwise touch; a persistent file that still cannot be opened should fall back to the same temp path as any other file, not fail the run. | **(b)** — chosen. |
| C19 | **A run that repairs no persistent file vs. refreshing the markers.** §6.2.1 records Collapse's precedent that a check finding no broken files leaves the old markers untouched, and C8's gate is phrased around files that *failed*. A byte-intact install with stale, missing or wrong-version markers is exactly the state that makes the game re-verify — the feature's reason for existing — so "nothing was broken" must not silently mean "nothing was published". | (a) always write the full marker block and both manifest mirrors under the flag, after every persistent file passes its gate (including files that passed unchanged); (b) write them only when at least one persistent file was repaired (Collapse parity); (c) write only markers that are stale/missing, leaving correct ones untouched. | **(a)** — the flag asks for an install *state*, not a repair count; markers already correct are rewritten with identical bytes, so this is idempotent, whereas (b) leaves the explicit request unfulfilled on the common "files fine, markers stale" install. C8 still gates on failure, not on whether work happened, so this does not weaken the all-or-nothing rule. | **(a)** — chosen. |
| C20 (revised) | **`--purge-only` vs. persistent work.** `--purge-only` runs the post-phase but deliberately puts no bytes in place: it skips the repair loop and the `config.ini` bump (`SUMMARY config_bumped=0`). Marker writes are placed in that same post-phase (C3/C9), so the mode would otherwise publish revision claims for hotfixes a cleanup-only run never installed. The original answer (a) also skipped the dispatcher and the manifests, which left hotfix bytes under `Persistent/` that were in those manifests with no keep rule — a cleanup-only run deleted them as orphans, the exact state the flag exists to avoid. | (a) skip all persistent work; (b) query and write markers anyway, treating them as bookkeeping; (c) refuse `--with-persistent --purge-only` (exit `1`); (d) query the dispatcher and verify the manifests for the keep set only, publishing no markers, mirrors or version claim. | **(d)** — a revision claim without bytes in place is still the desync the `config.ini` bump rule prevents, so nothing is published. But the keep set needs the manifests: a forced-persistent hotfix has no filename rule, and only its membership in `expected_rels` keeps it. (b) publishes a false claim; (a) deletes manifest bytes; (c) adds a flag-combination error no other mode pairing has. | **(d)** — chosen, superseding (a). |

### 6.5.2 Blockers that need a decision

Only items that need a product decision rather than more analysis in this
document. Each is a yes/no or a choice between concrete alternatives; the
suggested answer is a recommendation. B4's remaining channel verification was
completed during implementation/planning and is recorded in
`tools/persistent-sophon-research/CHANNELS.md`.

| # | Blocker | Possible answers | Suggested answer | Answer |
|---|---|---|---|---|
| B1 | **Flag envelope.** What does the single opt-in actually promise? | (a) one master flag enabling dispatcher + manifests + markers; (b) separate flags for each; (c) no flag, auto-detect from install contents. | **(a)** — a partial opt-in can produce an install with hotfixes but no markers, which is worse than no feature; (c) makes the default run's network behaviour nondeterministic. | **(a)** — chosen. |
| B2 | **Dispatcher key/seed source (§6.4.1).** The response is encrypted protobuf with a versioned seed and a region-specific RSA key; `girpr` has none of that today. | (a) port `Hi3Helper.EncTool` + `GenshinGateway.proto` and fetch the key/seed from Collapse's release metadata; (b) embed a maintained key/seed table with an explicit update path; (c) require the user to supply key/seed; (d) support only pre-captured manifests, never a live query. | **(a)** for the first release, with (b) as the fallback if depending on a third party's metadata repository is unacceptable. This is a maintenance-policy call, not a code gap. | **(a)** — chosen, with (b) as fallback if the third-party metadata dependency is unacceptable. |
| B3 | **Dispatcher failure while opted in.** Collapse warns and continues; its default is not an opt-in request. | (a) fail (exit `2`); (b) degrade to base-only repair and report a `persistent=unavailable` flag; (c) try again on the next run only. | **(a)** — a user who explicitly asked for a persistent update should not receive `exit 0` for a run that skipped it. Best-effort behaviour stays for runs without the flag (where the dispatcher is never queried). | **(a)** — chosen. |
| B4 | **Channel coverage for the first release.** The live probe (§6.4.5) covered `hk4e_global` only; CN/Bili dispatcher fields are unverified. | (a) global only, with a clear refusal for the other two `--biz` values while the flag is on; (b) all three at launch; (c) global + CN only. | **(a)** — ship the verified path behind the flag and add channels when each has a captured dispatcher response and manifest pair; research cannot certify the unprobed regions. | **(b)** — all three at launch; the follow-up verification was completed during implementation/planning and recorded in `tools/persistent-sophon-research/CHANNELS.md` (B4(b)); the **(a)** global-only fallback was not needed. |
| B5 | **Keep-set fix vs. the unflagged `SUMMARY` contract (C12).** The scope question in C7 is now settled (allow scoped runs; refuse only when a marker file is out of scope, and suppress markers when a dispatcher hotfix path is); what remains is whether widening the purge expected set with the markers and root `pkg_version` (C1/C2) may change `deleted_extra_bytes` for runs that pass no new flag. | (a) ship the keep-set fix independently of `--with-persistent` and document the change; (b) gate the keep-set fix behind the flag, accepting that unflagged runs can delete existing markers; (c) add a separate keep-only flag. | **(a)** — a deliberate safety fix is worth a one-time change to a deletion total; (b) leaves a destructive hole in the default path. | **(a)** — chosen. |
| B6 | **How far beyond Collapse to go (§6.4.3).** Collapse writes only `res_versions_persist`/`data_versions_persist`; GenshinSlimmer also manages `silence_data_versions_persist`, `audio_versions*`, `*versions_remote*`, `VideoAssets/*.tmp` and SDK/WPF/plugin keep files. | (a) mirror Collapse exactly, but add the extra *keep* inputs; (b) implement the full GenshinSlimmer behaviour including the deletions; (c) markers only, no other keep changes. | **(a)** — girpr is a repair tool, not a slimmer; deleting game-generated download state is out of scope, but protecting those files from the purge is cheap insurance. | **(a)** — chosen; extended in docs/03 §7 to also **maintain `silence_data_versions_persist`** (GenshinSlimmer / GI-Live-Hotfix-Downloader), while keeping `audio_versions*` and the deletions out of scope. |
| B7 | **Ceiling contract (C11).** Folding persistent bytes into the existing ceilings changes what those flags mean for opted-in runs. | (a) fold them in and document the broader meaning; (b) add dedicated persistent ceilings so the existing ones keep their exact definition; (c) leave persistent bytes unbounded. | **(a)** — enforceable ceilings are the point of the tier; (b) is defensible if a caller needs to budget the two paths separately, but doubles the flag surface. (c) is not acceptable. | **(a)** — chosen. |
| B8 | **Legal/ToS posture.** Querying `query_cur_region` directly and shipping/deriving its RSA key is a product risk that source inspection cannot answer. | (a) acceptable, same as Collapse and the community tools; (b) acceptable only with user-supplied keys; (c) not acceptable — drop live dispatch and support captured bundles only. | **(a)** is this report's technical recommendation; the final call belongs to the project owner and is recorded here rather than assumed. | **(a)** — chosen. |
| B9 | **Which version is authoritative for the dispatcher query and the markers.** Collapse queries `query_cur_region` with the *locally installed* version (`GameVersion.VersionString`) and writes that value into `ScriptVersion`. girpr repairs an arbitrary older install to `meta.latest`: the local version describes the *pre-repair* state, and a fresh install has no local version at all. The version in the query selects the revision set the dispatcher returns (§6.3 Step 3), so this choice decides both which hotfix bytes are fetched and which revision numbers are published. | (a) `meta.latest` is authoritative: query with it; if the response's own version field (probe `game_version_from_props`) disagrees, exit `2` rather than publish a revision the install does not have; (b) local version authoritative (Collapse parity): query with `meta.local_version`, write `ScriptVersion` from the response and still bump `config.ini` to latest — two version claims in one install, and no query at all on a fresh install; (c) dispatcher authoritative: install its hotfix set and bump `config.ini` to its version. | **(a)** — the bytes girpr puts in place are the latest base build, so only its revision set can describe the finished install; (b) can fetch stale hotfixes and make the game re-verify, which is this feature's failure case, and cannot start from a fresh install. Requires that the versioned key/seed from B2(a) cover `meta.latest`, and that the version disagreement be reported as a metadata failure rather than silently reconciled. | **(a)** — chosen. |

---

## 6.6. Recommended next steps (at research time)

Recorded when the research was written; steps 1-4 now describe the shipped opt-in
tier (docs/03 §7 holds the implementation design). Step 5 is an **operator
step**, not a repo test: see `README.md`'s "Manual verification" note. The original list follows:

1. Decide the dispatcher strategy (port the Collapse decoder vs. a minimal
   maintained key/seed table) and add it behind a dedicated flag so the existing
   repair-only default and its byte contracts stay byte-identical. Apply the
   output decisions in §6.5 (C7, C19, C20, B9) with that strategy — they fix which
   markers are published, in which modes, and for which version, not how the
   decoder is built.
2. Capture a real dispatcher response + all three persistent manifests into a
   `gircap` bundle (`docs/06`) before coding, so the protobuf fields, NDJSON
   schema (`hash` field in 7.1+) and exact marker contents can be pinned by tests.
3. Extend the purge expected set first (§6.4.3), independently of the updater, so
   an install's existing markers are never deleted.
4. Implement the four new phases of §6.4.2 with the marker writes last — including
   the third (`silence`) manifest mirror — and add `--check-only`, `--dry-run`
   and byte-ceiling coverage for persistent files.
5. Verify on a real install: update Persistent with `girpr`, copy the game to a
   second machine, launch once, and confirm no verification/re-download occurs.
   Assert `res_revision == res_revision_eternal`, that no `*versions_remote*` or
   `VideoAssets/*.tmp` files remain, and that the `audio_versions` /
   `silence_data_versions_persist` state is what the game expects.

## 6.7. Additional reference projects

Two repositories found during this research are direct, independent documentation
sources for the *in-game* half of the update mechanism, which neither existing
reference project covers. Both are pinned as git submodules under `reference-projects/`:

```powershell
git submodule add https://github.com/dnullptr/GenshinSlimmer.git reference-projects/GenshinSlimmer
git submodule add https://github.com/GesthosNetwork/GI-Live-Hotfix-Downloader.git reference-projects/GI-Live-Hotfix-Downloader
```

* `reference-projects/GenshinSlimmer` — reference for understanding the role of the
  different persistent files written by the game client (the *verification state*,
  §6.2.4).
* `reference-projects/GI-Live-Hotfix-Downloader` — reference for understanding the
  process of querying the dispatcher for hotfix versions and the download of
  persistent hotfix files. It generates `GenshinImpact_Data/Persistent` from live
  dispatch and shows the revision format (`1284249_ba7ad33643` — number + hash)
  with separate `res` / `silence` / `data` revisions, i.e. the same triple Collapse
  writes as separate scalar files. It also mirrors `silence_data_versions_persist`
  (and `data_versions_medium_persist`), which is why the shipped tier (docs/03
  §7) maintains the silence mirror. It is a second, independent implementation of
  the download half.

Why `GenshinSlimmer` is worth keeping locally:

* `README.md` and `GenshinSlimmer.ps1` pin the exact role of
  `res_versions_persist`, the revision files, `*versions_remote*` and
  `Persistent/VideoAssets/*.tmp`, including version-specific schema changes
  (the 7.1 `hash` field).
* `Get-LiveServerRevisions` contains a self-contained `query_cur_region`
  implementation (dispatcher URL, `dispatchSeed`/`key_id`, RSA key, protobuf
  field parser) that can be used to cross-check a Rust port of Collapse's
  `DispatchHelper`.
* It is actively maintained against current game versions, so it can serve as a
  canary when the dispatcher/manifest format changes again.

Optional companion (metadata, not code): Collapse's release-data repo
<https://github.com/CollapseLauncher/CollapseLauncher-ReleaseRepo> holds
`metadata/v3/stable/config_master.json` and `config_GIGlb.json`, which is where
`ProtoDispatchKey` and the dispatcher URL templates actually come from. A shallow
clone (or a direct fetch of those two files) is enough if the dispatcher strategy
in §6.4.1 needs to be pinned locally.

## 6.8. Source index

Launcher code (all under `reference-projects/Collapse/` unless noted):

* `CollapseLauncher/Classes/RepairManagement/Genshin/Fetch.Persistent.cs` — dispatcher
  manifest fetch, `res_versions_persist`/`data_versions_persist` writes, placement,
  force-persistent rule.
* `CollapseLauncher/Classes/RepairManagement/Genshin/Fetch.cs` — primary manifest,
  post-filters, `SavePersistentRevision` (lines 252-293), dispatcher query (298-341).
* `CollapseLauncher/Classes/RepairManagement/Genshin/GenshinRepair.cs` — entry chain,
  dispatcher cache (154-164), preset paths (39-47).
* `CollapseLauncher/Classes/RepairManagement/Genshin/Check.cs` / `Repair.cs` — check,
  move-back, plain-HTTP download, marker timing, ctable copy.
* `CollapseLauncher/Classes/InstallManagement/Genshin/GenshinInstall.PkgVersion.cs`
  — fake `pkg_version` from Sophon.
* `Hi3Helper.EncTool/Parser/YSDispatchHelper/{DispatchHelper.cs,QueryProperty.cs}` and
  `Hi3Helper.EncTool/Parser/Proto/GenshinGateway.proto` — dispatcher decode.
* `CollapseLauncher/Classes/Helper/Metadata/PresetConfig.cs` — dispatcher keys/URLs.
* `reference-projects/Starward/src/Starward.RPC/GameInstall/{GameInstallService.cs,
  GamePackageService.cs,GameInstallHelper.cs}` — no-Persistent behaviour, config.ini
  and audio-marker writes.
* `reference-projects/GenshinSlimmer/` — reference for understanding the role of the
  different persistent files written by the game client (`README.md`,
  `GenshinSlimmer.ps1`, §6.2.4).
* `reference-projects/GI-Live-Hotfix-Downloader/` — reference for understanding the
  dispatcher hotfix-version query and the download of persistent hotfix files.

Live probes: `tools/persistent-sophon-research/` — re-runnable Python scripts
(metadata unwrap, dispatcher query/decrypt/parse, HYP/Sophon enumeration,
cross-match, CDN probes) plus `README.md` with the §6.4.5 evidence; `samples/` is
gitignored.

Web:

* <https://github.com/dnullptr/GenshinSlimmer> (README v14 and `GenshinSlimmer.ps1`)
  — independent documentation of `res_versions_persist`, the revision files,
  `*versions_remote*`, `VideoAssets/*.tmp`, and the `query_cur_region` endpoint.
* <https://github.com/GesthosNetwork/GI-Live-Hotfix-Downloader> — a second
  implementation of live-dispatch Persistent generation.
* <https://gist.github.com/khang06/622d020dec0484b2854ee4eef17b56c1> — GI string
  table naming the game-side manifest/marker files.
* <https://www.reddit.com/r/Genshin_Impact/comments/10zmgtb/> — game log showing
  `IsBaseResVersionHashChanged` comparing `baseResVersionHashInPersist` with the
  MD5 in `res_versions_streaming`.
* <https://xia0ji233.github.io/2025/02/14/Game6> — reversing report showing the
  game opening `Persistent/base_res_version_hash` at startup.
* <https://github.com/CollapseLauncher/Collapse/issues/166> — real game log of
  `IsBaseResVersionHashChanged` followed by `Clear state files in persist dir!!!!!`.
* <https://github.com/dnullptr/GenshinSlimmer/issues/19> — field report that the
  client checks `res_revision_eternal` alongside `res_revision`, and that stale
  `res_versions_remote` + `VideoAssets/*.tmp` cause the large re-download loop.
* <https://rentry.org/genshbackup> and related threads — community "reset
  verification state" fix (deletes `base_res_version_hash`, `cache_versions_*`,
  `ChannelName`, `res_versions_remote`, `ScriptVersion`, `VideoAssets/`).
* <https://github.com/CollapseLauncher/Collapse/commit/9cce3af95a300b6d0316b7b8c3d507717f58b879>
  — the `base_res_version_hash` / marker write was made conditional on dispatcher
  availability.

girpr: `README.md` (action graph, v1 limits, flag contract), this document §§4-6,
`docs/02` §5, `src/repair/mod.rs`, `src/repair/purge.rs`, `src/repair/phases/{metadata,post}.rs`,
`crates/fetch/src/hyp.rs`.
