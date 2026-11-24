# How to reproduce / continue this research

This document is the hand-off for another agent. It records the *procedure* that
produced the answer, not just the conclusions. Read
`00-findings-before-launch.md` through `05-ablation-C-manifests.md` for the
results; this file says how to get there and what the ablation proved.

**Committed vs. generated.** Only this `notes/*.md`, the `*-short/` dirs and
`scripts/` are in git. The full generated artifacts stay on disk in `logs/`,
`snapshots/` and `notes/*.txt` (gitignored): raw logs contain session tickets,
and the inventories/diffs are large. To regenerate the committed compact
summaries from them at any time, run `python scripts/make_short.py`.
`README.md` lists every script and the exact regeneration commands; each
`*-short/README.md` and `notes-short/*.md` states the full-file source.

## Environment / constraints

- Working tree: `E:\hoccode\hocrust\girpr`. Context docs: `docs/08`, `docs/09`.
- Fresh game: `F:\Genshin Impact game` (torrent copy, 7.2.0, English VO, no
  `Persistent/`, no `config.ini`, 8068 files / 155,182,786,241 bytes).
- Synced reference: `E:\Volume 1\gem\power\Genshin Impact game` (full
  `Persistent/` from prior hotfix sync, plus stale 7.1 files and caches).
- Both are HDDs: **never parallelise hashing within one disk**. E: and F: are
  separate spindles and may be worked at the same time. If F: disappears, stop
  and wait for the user to reconnect it.
- Only the user can launch the game (escalation) and log in; end the turn with
  explicit actions and observations. Prefer the game closed before resuming.
  The game writes logs to
  `%LOCALAPPDATA%\miHoYo\Genshin Impact\` (not the game folder).
- `/reference-projects/` holds Collapse, Starward, GenshinSlimmer and
  GI-Live-Hotfix-Downloader for cross-checking.

## Procedure that was followed

### 1. Preserve before touching anything

1. Copy the small files from E:'s `Persistent/` (including `DownloadPref` as
   `.bin`) and E:'s `config.ini` into `snapshots/E-marker-files/`.
2. Copy the user-data logs (`info.txt`, `LocalLog.log`, `output_log.txt`,
   `output_log.txt.last`, `logs/NativeSDK.log`, `logs/MiHoYoSDK.log`,
   `AppData\Roaming\GME\GenshinImpact.exe\GMESDK_*.log`) into `logs/`.
   The game rotates `output_log.txt` on every launch — copy it *before* asking
   the user to launch again.
3. Snapshot the untouched F: copy:
   `python scripts/snapshot.py "F:\Genshin Impact game" snapshots/F-fresh/F-snapshot.jsonl --hash-max-size 2097152`

### 2. Establish that the two copies share the same base

- `python scripts/torrent_inspect.py <F:/*.torrent>` -> file list/size; compare
  its path set to the F: snapshot (equal: 8068 files, 0 extra/missing).
- `python scripts/compare_copies.py F-snapshot.jsonl E-snapshot.jsonl` ->
  0 common files with differing MD5/size; differences are only extras.
- Verify `base_res_version_hash` = MD5(`StreamingAssets/res_versions_streaming`)
  with `Get-FileHash`; it is `f5ceb98f…` on both copies.

### 3. Read the marker roles from the game's own logs

Grep the preserved logs for `IsBaseResVersionHashChanged`, `DownloadPref`,
`persistent code switches`, `OnConnectRegionDispatch`, `File num to verify`,
`CheckAllFinish`, `Finish get totalSize`. On E: (already synced) the lines are
`IsBaseResVersionHashChanged` with equal hashes and `File num to verify = 4`;
its `DownloadError.log` history shows the one-time `2632`-file check. This is
where the "skip verification" hypothesis came from, before the live experiment.

### 4. Ask the user to launch the fresh copy once

Instructions given: launch the exe directly (no launcher "locate game"/Update),
log in, choose region, press Start, let download + integrity check + "preparing
to load data" finish, enter game, close it. Observed UI stages: Login → Choose
region → Press start → "calculating files for download" → "downloading
resources" (~1102.90 MB) → "verifying file integrity" (~16.5 min) → "preparing
to load data" → in-game.

### 5. Diff the result

`python scripts/collect_after_launch.py after-launch` copies the rotated logs,
snapshots F: again and diffs against the baseline. Result: 461 added, 0 removed,
0 base files changed; all 20 marker files created; values byte-identical to E:.
`python scripts/verify_persist_manifest.py <snapshot> <synced-game-dir>` checks
manifest rows against files on disk (remember: many rows are base files under
`StreamingAssets/`, not `Persistent/`).

### 6. Ask the user to launch F: a second time

Result: `File num to verify = 4`, instant, no download; only a subset of markers
has its mtime refreshed. This confirms the skip.

## Conclusions (one paragraph)

The game launches without `config.ini`. With no `Persistent/` it logs
`baseResVersionHashFile not exist` / `clear revision files`, verifies 2632 files
and downloads ~1.08 GiB of hotfixes, then writes: `base_res_version_hash` (MD5
of `StreamingAssets/res_versions_streaming`), the three `*_versions_persist`
manifest mirrors, `res_revision` + `res_revision_eternal`, `data_revision`,
`silence_revision`, `audio_revision`, `PatchDone`, `ChannelName`, `ScriptVersion`.
Those are exactly the block `docs/09` plans to write. `DownloadPref`
(game-written download-progress JSON), `CodeSwitch.config`, `DownloadError.log`,
`test_file`, `ctable.dat`, `audio_lang_14`, `text_lang`, SDK/web caches,
`base_revision`, `StreamingAssets/ctable.dat`, root `config.ini`, BeyondAssets
and the stale `.blk` files are *not* required for the skip; girpr should only
keep them out of the purge. No `*versions_remote*` or `VideoAssets/*.tmp` files
appear on a clean run, but they must stay purge candidates per GenshinSlimmer
issue #19.

## Marker ablation — completed

Design (all reversible): `python scripts/ablation.py backup` copied F:'s
`Persistent/` (393 files, 1103.31 MB) to
`E:\Symlinks\Temp\opencode\girpr-test\persist-backup`; for each round only one
group of small marker files was moved out, the user launched F:/logged in/
chose region/pressed Start and closed at the first sign of a full check, and
`ablation.py restore` put the exact backup back. Decisive datum: the new line in
`Persistent/DownloadError.log` (`File num to verify = 4` vs `2632`) plus the
startup `output_log.txt` lines.

| round | removed | result | note |
|---|---|---|---|
| A | `base_res_version_hash` only | **full reset**: 2632-file check, ~1 GB re-download, `*_versions_remote` created, revision files and hotfixes deleted | `03-ablation-A-base-hash.md` |
| B | `res_revision` + `res_revision_eternal` | fast path (4 files); game re-queried dispatcher and recreated both with `49530578` | `04-ablation-B-revisions.md` |
| C | all three `*_versions_persist` | fast path (4 files); res/data mirrors stayed missing, only `silence_*` was recreated; read-only bit cleared on all hotfix files | `05-ablation-C-manifests.md` |

**Answer for the repair tool:** `base_res_version_hash` is the single
load-bearing "skip verification" marker. The revision files and the res/data
mirrors are self-healing *when the base hash and hotfix bytes are intact*, so
girpr's priority is: never delete/omit `base_res_version_hash` (keep-set, C1/C2)
and write it correctly (MD5 of `StreamingAssets/res_versions_streaming`).
Writing the whole `docs/09` block is still correct and idempotent — it avoids
the extra dispatcher round trip / "freeze" seen in B, keeps the manifest mirrors
available for future revisions, and matches what the game itself writes — but
only the base hash is strictly required for the immediate skip.

F: was restored after every round and is left in the clean synced state
(393 files, 1103.31 MB, all markers present).
