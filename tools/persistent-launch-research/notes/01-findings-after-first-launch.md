# Findings after the first launch of the fresh F: copy

User action (2026-11-24): launched `F:\Genshin Impact game\GenshinImpact.exe`
directly (elevated), logged in, chose a region, pressed Start, let the game run
the download + integrity check + "preparing to load data", entered the game and
Alt+F4'd. **`config.ini` did not appear** in the game root.

## Result summary

- 461 files added, **0 removed, 0 base files changed**.
- Bytes: 155,182,786,241 -> 156,347,250,236 = **+1,164,463,995 bytes
  (~1.08 GiB)**, matching the UI's ~1102.90 MB.
- `F:` now has 8529 files, `Persistent/` has 393 files.

## Startup / verification sequence (from `logs/after-launch/output_log.txt`)

```
15:42:04.514  DownloadPref load pref file failed, use default, clear revision files
15:42:04.603  IsBaseResVersionHashChanged=> baseResVersionHashFile not exist
15:42:23.458  Wrote 0 persistent code switches
15:44:25.822  Wrote 3 persistent code switches        (after login + region select)
15:44:25.918  OnConnectRegionDispatch=> download job type = Normal
15:44:26.136  Not set common res exist flag, force download
15:44:27.*     FilesDownloadPipe=> StartDownload
15:45:14.250  File num to verify = 2632               (DownloadError.log)
16:01:41.964  CheckAllFinish return true
16:01:42.710  DeserializeBinary (.../Persistent/AssetBundles/blocks/00/31049740.blk) onlyStreamingFalse
16:01:44.921  LoginMainPageContext OnDownloadBunddleFinish
```

The check took **16 min 27 s**. On the already-synced E: copy the same lines
read `File num to verify = 4` / `CheckAllFinish return true` within ~0.1 s.
This is direct evidence that the marker block is what suppresses the 2632-file
verification: the game itself logs `baseResVersionHashFile not exist` and
`clear revision files` when the base marker is absent.

`DownloadPref` is rewritten several times. Its JSON starts as
`{"scriptVersion":"7.2.0","resCategoryStatusDict":{"0":0},"incrementVerify":false,...}`,
grows to all categories `0`, then after the check becomes `"0":1` with
`incrementVerify:true`, i.e. a *download-progress* state ("common res exists"),
not a verification flag. It is opaque on disk (4290 bytes) and F:'s copy differs
from E:'s (E: has all categories `1`), confirming it is per-install game state
that girpr need not author.

## Marker files created (all under `GenshinImpact_Data/Persistent/`)

Values are **identical** to E:'s reference copy:

| file | value on F: after launch |
|---|---|
| `base_res_version_hash` | `f5ceb98fd93b8c69603d7b5600d11a08` |
| `res_revision` / `res_revision_eternal` | `49530578` |
| `data_revision` | `49419538` |
| `silence_revision` | `49291486` |
| `audio_revision` / `PatchDone` | `49008628` |
| `ChannelName` | `OSRELWin` |
| `ScriptVersion` | `7.2.0` |
| `base_revision` | **not created on F:** (E:-only stale file) |
| `res_versions_persist` | MD5 `9a1b5efc2e2b11475e74c88789d0f339` (identical to E:) |
| `data_versions_persist` | MD5 `09f2c36615dc806f67f55e3d2c8a0c8d` (identical to E:) |
| `silence_data_versions_persist` | MD5 `2bcf33810856cb0820725300ff2d8666` (identical to E:) |
| `ctable.dat` | 28,160,000 B, copy of `StreamingAssets/ctable_streaming.dat` |
| `CodeSwitch.config` | `65000\r\n65002\r\n66006\r\n` |
| `DownloadError.log` | `File num to verify = 2632` / `CheckAllFinish return true` |
| `test_file` | `WRITE_FAILED` (write probe) |
| `audio_lang_14` | `English(US)` — **11 bytes, no CRLF** (E:'s is 13 bytes with CRLF, probably launcher-rewritten) |
| `text_lang` | `1\n2\n…\n15` (identical to E:) |
| `AudioAssets/English(US)/VO_7.2_a_0.pck` | 230,283 B hotfix audio |

No `*versions_remote*`, no `VideoAssets/*.tmp`, no `.hdiff`/`_tmp` files were
left behind by a clean first run.

## Comparison with the E: copy

- F: `Persistent/` (393 files) is a strict subset of E:'s (395 files). F: lacks
  only `DownloadError.log.bak` (log rotation) and `base_revision`.
- All 391 common `Persistent/` files (including every hotfix `.blk`) have
  **identical size and MD5** — the first-launch download produces exactly the
  same hotfix set that E: accumulated.
- `StreamingAssets/ctable.dat` was **not** created on F: (E: has it); the game
  only creates `Persistent/ctable.dat`. `StreamingAssets/ctable.dat` is therefore
  a launcher artifact, not a game requirement.
- No `config.ini` was created; the game starts without it.

## Cross-check with reference projects

`reference-projects/GenshinSlimmer/README.md` independently states that
`res_versions_persist` is the game's asset database and that synchronizing it
plus `res_revision`/`res_revision_eternal`/`silence_revision`/`data_revision`
bypasses the "2,580-file resource check" and stops patch re-downloads, and that
`*versions_remote*` + `VideoAssets/*.tmp` must be deleted to stop loops. This
matches our observed 2632-file check and the byte-identical mirrors.

## Implication for girpr

The docs/08 and docs/09 marker block is confirmed by the real game:
`base_res_version_hash` + the three `*_versions_persist` mirrors + the
revision block are what the game itself writes and reads. `DownloadPref`,
`CodeSwitch.config`, `DownloadError.log`, `test_file`, `ctable.dat`, caches,
`audio_lang_14` and `text_lang` are game/launcher artifacts that girpr does not
need to author; keeping them out of the purge is still wise (C1/C2).
`base_revision` and `audio_lang_14`'s CRLF are launcher-side extras.

## Still to verify

A **second** launch of F: should now show `File num to verify = 4` (skip). If
the user confirms, that closes the loop from "fresh torrent copy" to "copied
after verification does not re-verify".
