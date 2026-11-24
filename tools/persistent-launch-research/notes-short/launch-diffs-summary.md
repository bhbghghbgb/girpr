# Launch / ablation filesystem deltas (extracts of the gitignored `notes/*-diff.txt`)

Regenerate the full snapshots and diffs with:

```
python scripts/collect_after_launch.py after-launch
python scripts/collect_after_launch.py second-launch
python scripts/collect_after_launch.py ablation-A   # -B, -C likewise
python scripts/diff_snap.py snapshots/F-fresh/F-snapshot.jsonl snapshots/F-fresh/F-after-launch.jsonl
```

All comparisons below are against the **fresh baseline**
(`F-snapshot.jsonl`, 8,068 files / 155,182,786,241 B).

| run | added | removed | changed | bytes | verification |
|---|---|---|---|---|---|
| first launch | 461 | 0 | 0 | +1,164,463,995 (~1.08 GiB) | 2,632 files, 16.5 min |
| second launch | 0 | 0 | 19 | +41,058 | 4 files, 0.16 s |
| ablation A (`base_res_version_hash` removed) | 95 (partial re-download) | many (state wiped) | — | -1.0 GB at stop | full reset planned, ~1,045 MB shown |
| ablation B (revision pair removed) | 0 | 0 | 23 | +94,306 | 4 files |
| ablation C (three mirrors removed) | 0 | 2 | 397 (mostly `RA`->`A` attribute) | +94,306 | 4 files |

First launch: 20 marker files under `Persistent/` created (`base_res_version_hash`,
`ChannelName`, `ScriptVersion`, `PatchDone`, `res_revision`,
`res_revision_eternal`, `data_revision`, `silence_revision`, `audio_revision`,
`base_revision` only on reset, the three `*_versions_persist`, `DownloadPref`,
`DownloadError.log`, `CodeSwitch.config`, `test_file`, `ctable.dat`,
`audio_lang_14`, `text_lang`, `VO_7.2_a_0.pck`), plus ~440 hotfix `.blk` files,
SDK/web caches. 0 base files changed, no `config.ini`.

Second launch: only mtimes refreshed on `ChannelName`, `ScriptVersion`,
`base_res_version_hash`, `silence_data_versions_persist`, `silence_revision`,
`CodeSwitch.config`, `test_file`; `DownloadPref`/`DownloadError.log` bytes
changed; 10 SDK cache files changed. `res_*`/`data_*` markers and hotfix files
untouched.

Ablation A: game deleted all revision files and hotfixes, wrote
`res_versions_remote` / `data_versions_remote` / `silence_data_versions_remote`,
recreated `base_res_version_hash` and the mirrors, and began downloading ~1 GB.

Ablation B: game recreated `res_revision` and `res_revision_eternal` with
`49530578`; everything else byte-identical.

Ablation C: `res_versions_persist` and `data_versions_persist` stayed missing;
`silence_data_versions_persist` was recreated byte-identical; read-only flag
cleared on all hotfix files; no `*_versions_remote*` appeared.

F: was restored from the E: backup after every ablation; final state 393 files /
1103.31 MB with all markers present.
