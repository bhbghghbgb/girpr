# Persistent / hotfix launch research (real-install experiment)

Goal (see `docs/08` and `docs/09`): determine empirically, on real game copies,

1. what the game needs to launch / to skip verification;
2. what the game writes when it downloads hotfixes, and each file's purpose;
3. which files are launcher-only extras (Collapse / Starward / HoYoPlay) that the
   game does not need;
4. the minimum marker set a repair tool must write so the game trusts the install.

## Copies

| path | role | state |
|---|---|---|
| `F:\Genshin Impact game` | originally the fresh minimal copy (torrent, 7.2.0, English VO; no `Persistent/`, no `config.ini`) | launched twice + 3 ablation rounds; **restored after every experiment** to the clean synced state (393 `Persistent/` files), no `config.ini` |
| `E:\Volume 1\gem\power\Genshin Impact game` | synced reference; launched 2026-11-24 14:28 and 15:12 | has full `Persistent/` state from prior hotfix sync; some stale 7.1 files in `StreamingAssets` |
| `F:\...rutracker...html` | installation page incl. the launcher's `config.ini` template | full file stays on F:; short summary in `notes-short/torrent-summary.md` |
| `F:\...rutracker....torrent` | 8068 files / 144.53 GiB | full file stays on F:; short summary in `notes-short/torrent-summary.md` |

Both disks are HDD: never run parallel hashes on the same disk; E: and F: may be
worked at the same time (different spindles). If F: disappears, stop and wait for
the user to reconnect it.

## Layout — committed vs. generated

**Committed:** `notes/*.md` (findings + `PROCESS.md`), `notes-short/`,
`logs-short/`, `snapshots-short/` (compact, self-explaining summaries), and
`scripts/`. The `*-short/` dirs contain only the relevant extracts, never full
dumps or raw logs.

**Gitignored but left in place where the scripts write them:**
`snapshots/` (full inventories and marker-file copies),
`logs/` (raw user-data logs, which contain session tickets), `notes/*.txt`
(full generated diff/analysis/torrent dumps), and Python caches. Regenerate all
of them with the scripts below; `scripts/make_short.py` regenerates the
committed `*-short/` summaries from them. See `notes/PROCESS.md`.

- `scripts/snapshot.py ROOT OUT.jsonl [--hash-max-size N]` — inventory + selective MD5
- `scripts/diff_snap.py BEFORE AFTER` — added / removed / changed files
- `scripts/compare_copies.py A B` — differences between two inventories
- `scripts/torrent_inspect.py FILE.torrent` — list torrent files and sizes
- `scripts/analyze_manifests.py FRESH SYNCED` — compare streaming vs persist manifests
- `scripts/verify_persist_manifest.py SNAPSHOT SYNCED_GAME_DIR` — check manifest rows against files
- `scripts/collect_after_launch.py [LABEL]` — one-shot post-launch log + snapshot collector
- `scripts/ablation.py {backup,remove,restore,status}` — reversible marker ablation
- `scripts/make_short.py` — regenerate `logs-short/` and `snapshots-short/`

## Baseline findings (before launching F:)

See `notes/00-findings-before-launch.md`. Highlights:

- The F: copy exactly matches the torrent: 8068 files, 155,182,786,241 bytes,
  and its base files are **byte-identical** to E: (0 differing common files).
- `Persistent/base_res_version_hash` = MD5 of
  `StreamingAssets/res_versions_streaming` = `f5ceb98fd93b8c69603d7b5600d11a08`
  (verified on both copies).
- E:'s markers: `res_revision=49530578`, `res_revision_eternal=49530578`,
  `data_revision=49419538`, `silence_revision=49291486`,
  `audio_revision=49008628`, `PatchDone=49008628`, `ChannelName=OSRELWin`,
  `ScriptVersion=7.2.0`, `base_revision="49008628 644b3d946e"`.
- The game's own logs prove it reads/writes these: `IsBaseResVersionHashChanged=>
  ...`, `Load/Save DownloadPref`, `Wrote 0/3 persistent code switches`,
  `OnConnectRegionDispatch`, `File num to verify = 4/2632`, `CheckAllFinish`.
- `DownloadPref` is a **game-written** download-state blob (its JSON is printed
  in `output_log.txt`); not purely launcher metadata as `docs/08` assumed.
- The game writes only `Persistent/ctable.dat` (28 MB, byte-identical to
  `StreamingAssets/ctable_streaming.dat`, MD5 `cd5f73407c6c4733f3c86a145e9f94c0`).
  E:'s extra `StreamingAssets/ctable.dat` is a launcher artifact.
- The launcher's `config.ini` template in the torrent page contains keys the E:
  copy's `config.ini` does not (`uapc`, `downloading_mode`,
  `plugin_OQKz9izioj_version`, `cps=hoyoverse` vs `hyp_hoyoverse`).
- F: has `BeyondAssets/` + `beyond_pkg_version` (UGC editor; out of scope, girpr
  will not touch it). E: does not have them.

## Results

1. First launch from the fresh copy: verification of 2632 files, ~1.08 GiB
   hotfix download, creation of the full marker block — see
   `notes/01-findings-after-first-launch.md`.
2. Second launch: `File num to verify = 4`, no download — see
   `notes/02-findings-second-launch.md`.

3. Three reversible marker-ablation rounds (`notes/03`–`05`) proved that
   `base_res_version_hash` alone is load-bearing for the verification skip;
   revision files and the res/data manifests are regenerated or unnecessary on
   the fast path. F: was restored to the clean synced state after every round.

`notes/PROCESS.md` is the hand-off: it gives the exact reproduction steps,
environment constraints and the ablation matrix. Full generated artifacts are
gitignored; `notes-short/`, `logs-short/` and `snapshots-short/` hold the
compact, sanitized extracts that are committed instead.
