# Baseline findings (before launching the fresh F: copy)

## 1. The two copies are the same base install

- F: (torrent) = 8068 files / 155,182,786,241 bytes / 144.53 GiB, exactly the
  torrent's `file count` and `total` (`notes/torrent-files.txt`). The set of
  relative paths matches the torrent **exactly**: 0 files missing, 0 extra.
- E: = 6502 files / 159,533,438,713 bytes.
- `compare_copies.py` reports **0 common files with a different MD5 or size**.
  Every difference is an *extra* file on one side:
  - E: extras: 3528 files = ~3000 `Persistent/` hotfix files, ~2758 `webCaches`,
    350 stale `StreamingAssets/AssetBundles/blocks/**/*.blk` (old-version
    leftovers, matching the user's warning), 23 `SDKCaches`, `config.ini`,
    `upload_err.log`, `Persistent/ctable.dat`, `StreamingAssets/ctable.dat`.
  - F: extras: 5094 files, essentially all `BeyondAssets/**` + UGC editor files.
- So E: is a valid reference for "what the game adds on top of the torrent copy",
  with the caveat that it also carries stale files from older versions.

## 2. What is needed to launch / to avoid verification

Confirmed from the game's own logs (copies in `logs/`):

- Startup log line (both E: sessions, 14:28 and 15:12 on 2026-11-24):
  `IsBaseResVersionHashChanged=> baseResVersionHashInPersist =
  f5ceb98fd93b8c69603d7b5600d11a08, md5 in streamingResVersion =
  f5ceb98fd93b8c69603d7b5600d11a08`.
  The two values are equal, so the game did **not** wipe/reset Persistent state.
  `Persistent/base_res_version_hash` is exactly the lowercase MD5 of
  `StreamingAssets/res_versions_streaming` (verified with Get-FileHash on both
  copies). This is the base-install verification marker.
- `info.txt` (user data) = `version:7.2_rel
  OSRELWin7.2.0_48751704_49530578_49419538_49291486_49008628` — client build
  number plus res/data/silence/audio revisions, all of which appear as separate
  marker files in E:'s `Persistent/`.
- After login + region selection:
  `Genshin Start Log: NetworkManager SetServerDispatchConfig isGlobal=False`,
  `OnConnectRegionDispatch=> download job type = Normal`,
  `Genshin Download Log: BundleDownloadWorld OnStartVerify Normal`,
  `File num to verify = 4`, `CheckAllFinish return true`.
  When the persistent state was still being built (11-16), the same log reads
  `File num to verify = 2632` followed by `CheckAllFinish return true`; a smaller
  hotfix on 11-19 changed it to `File num to verify = 7`.
  => With a correct marker set the game only re-checks a handful (4) of files;
  with a missing/stale set it checks ~2632 and re-downloads.
- `DownloadError.log` records those three `Finish get totalSize: 13861.90mb /
  16440.78mb / 14592.40mb` probes on launches even when nothing is downloaded.

## 3. What the game writes (E: `GenshinImpact_Data/Persistent/`)

Small files, with values and mtimes captured verbatim under
`snapshots/E-marker-files/`:

| file | bytes | value / purpose |
|---|---|---|
| `base_res_version_hash` | 32 | `f5ceb98fd93b8c69603d7b5600d11a08`; base-install verify marker |
| `res_revision` | 8 | `49530578` |
| `res_revision_eternal` | 8 | `49530578` (HYV's typo for "external"; checked alongside res_revision) |
| `data_revision` | 8 | `49419538` |
| `silence_revision` | 8 | `49291486` |
| `audio_revision` | 8 | `49008628` |
| `PatchDone` | 8 | `49008628` (= audio_revision) |
| `base_revision` | 19 | `49008628 644b3d946e` (revision + hash from base_revision GET) |
| `ChannelName` | 8 | `OSRELWin` |
| `ScriptVersion` | 5 | `7.2.0` |
| `DownloadPref` | 4290 | opaque binary; `output_log.txt` logs its plaintext JSON (`scriptVersion`, `resCategoryStatusDict`). **Game-written** (`Load/Save DownloadPref` log lines), not just launcher metadata |
| `res_versions_persist` | 370958 | 2631 NDJSON rows (vs 2624 in `res_versions_streaming`): the game's hotfix asset database |
| `data_versions_persist` | 71416 | 533 rows, 368 of which have different md5/hash/size than `data_versions_streaming` (the hotfixes) |
| `silence_data_versions_persist` | 523 | 4 rows, all different from `silence_versions_streaming`; the files themselves live in `StreamingAssets/AssetBundles/blocks/00/` on E:, i.e. a persisted manifest can also describe base-tree files |
| `ctable.dat` | 28160000 | byte-identical to `StreamingAssets/ctable_streaming.dat` (and to E:'s `StreamingAssets/ctable.dat`; MD5 `cd5f7340…`) |
| `CodeSwitch.config` | 21 | `65000\r\n65002\r\n66006\r\n`, written after region selection (log: `Wrote 3 persistent code switches`; before login it says `Wrote 0 persistent code switches`) |
| `DownloadError.log(.bak)` | 4591/23153 | download/verify progress log |
| `test_file` | 12 | literal `WRITE_FAILED`; likely a write-access probe file |
| `audio_lang_14` | 13 | `English(US)\r\n` (voice language selection) |
| `text_lang` | 35 | `1\n2\n…\n15` (stale, 2024-11-20) |
| `AudioAssets/English(US)/VO_7.2_a_0.pck` | 230283 | one hotfix audio file |

Plus ~3000 hotfix files under `Persistent/AssetBundles/blocks/**` (and a couple
in `AudioAssets`, `VideoAssets`). 375 of the persist-manifest rows resolve to an
existing file under `Persistent/`; the remaining ~2793 rows are base copies that
stay under `StreamingAssets/` (they are in the torrent).

## 4. Launcher extras the game does not need

- Root `config.ini`: not in the torrent. The torrent page supplies a launcher
  template with `game_version`, `channel`, `sub_channel`, `cps=hoyoverse`,
  `uapc={...}`, `downloading_mode`, `wpf_version=7.2.0.49085326`,
  `plugin_OQKz9izioj_version=1.0.0`. E:'s actual `config.ini` has
  `game_version=7.2.0`, `wpf_version=7.2.0.49085326`, `game_biz=hk4e_global`,
  `cps=hyp_hoyoverse`, `channel=1`, `sub_channel=0`, empty `sdk_version`.
  These are launcher/versioning fields, not game verification markers.
- `BeyondAssets/**`, `beyond_pkg_version` (UGC editor "Beyond") — present in
  F:, absent in E: — independent update; girpr will not handle it.
- `webCaches/**`, `SDKCaches/**` — SDK/webview caches written by the game/SDK,
  nothing to do with content verification.
- 350 stale `.blk` files in E:'s `StreamingAssets/AssetBundles/blocks/**` are
  old-version leftovers; not verification state.

## 5. Implication for girpr's marker handling

- The load-bearing "skip verification" set is: correct
  `base_res_version_hash` + the three `*_versions_persist` mirrors +
  matching `res_revision`/`res_revision_eternal`/`data_revision`/
  `silence_revision` (and harmless Collapse-parity `audio_revision`,
  `PatchDone`, `ChannelName`, `ScriptVersion`, `base_revision`).
- `DownloadPref` is written by the game itself on every launch, so a repair
  tool need not synthesize it (docs/01 §6 and docs/03 §7 already say skip it).
- `ctable.dat` in `Persistent/` and `StreamingAssets/` are copies of
  `ctable_streaming.dat`; a repair tool only needs to keep/produce the
  `ctable_streaming.dat` source.
- `test_file`, `CodeSwitch.config`, `DownloadError.log`, caches are game
  artifacts, not things girpr should author.
- `config.ini` is launcher bookkeeping, not in-game verification state.

The fresh-copy launch will tell us which of these are created immediately at
startup vs only after login + region selection, and whether the game can start
without `config.ini` at all.
