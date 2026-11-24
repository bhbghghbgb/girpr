# Ablation A — remove only `Persistent/base_res_version_hash`

Setup: F: was in the clean, synced state after the second launch
(`File num to verify = 4`). `ablation.py backup` copied the whole `Persistent/`
to E: (393 files, 1103.31 MB); `ablation.py remove base_res_version_hash`
moved only that one file out. User launched F:, logged in, chose region, pressed
Start, saw "calculating files for download" then "downloading resources
(1045.46 MB)", and closed the game during the download.

## Observation

The game **discarded essentially the whole Persistent state** and started a
fresh hotfix download:

- `output_log.txt`: `IsBaseResVersionHashChanged=> baseResVersionHashFile not
  exist`, followed by `Not set common res exist flag, force download` and two
  `FilesDownloadPipe=> StartDownload` lines. (No `clear revision files` line
  this time — it was present on the very first launch when `DownloadPref` was
  also absent; here `DownloadPref` was intact and loaded with all categories
  `1`, but the missing base hash alone was enough.)
- `Persistent/` went from 393 files / 1103 MB to **27 files** at the moment the
  game was closed; the F: tree went from 8529 files / 156.35 GB to 8163 files /
  155.32 GB. 95 `.blk` files were in the process of being re-downloaded.
- Deleted by the game: `DownloadError.log` (rotated to `.bak`), `res_revision`,
  `data_revision`, `silence_revision`, all hotfix `.blk`/`.pck` files.
- Regenerated immediately: `base_res_version_hash` (same value
  `f5ceb98fd93b8c69603d7b5600d11a08`, recomputed from
  `StreamingAssets/res_versions_streaming`), `ChannelName`, `ScriptVersion`,
  `audio_revision`, `res_revision_eternal`, `CodeSwitch.config`, `test_file`,
  `ctable.dat`, `audio_lang_14`, `text_lang`, `DownloadPref`, the three
  `*_versions_persist` mirrors.
- **New, harmful state appeared**: `res_versions_remote` (370958 B),
  `data_versions_remote` (71416 B), `silence_data_versions_remote` (523 B) —
  the unpatched server manifests that GenshinSlimmer says must be deleted to
  stop the re-download loop. Also `base_revision` (`49008628 644b3d946e`) and
  `DownloadError.log.bak`.
- Note the asymmetry: `res_revision_eternal` survived with the old value while
  `res_revision` was deleted — the reset does not clean both halves uniformly.

Small files from this state are preserved under `snapshots/F-ablation-A/`.

## Conclusion

`base_res_version_hash` is the **master verification marker**. A repair tool
that deletes or fails to write it forces the game to wipe its entire hotfix
database, re-fetch the server manifests into `*_versions_remote`, and re-download
~1 GB of hotfixes. This is the single highest-priority file for the purge
keep-set (C1/C2 in docs/01 §6) and for the marker write (C19 in docs/03 §7).

State restored from the E: backup before any further ablation.
