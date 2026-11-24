# Second launch of F: — verification skipped

User action: accidentally started the E: copy for a few seconds and killed it
via Task Manager (E:'s marker mtimes were checked afterwards and are unchanged —
the reference is intact), then launched F: again, logged in, chose a region,
pressed Start, let it finish, and Alt+F4'd.

## Result

`F:\Genshin Impact game\GenshinImpact_Data\Persistent\DownloadError.log`:

```
11-24 15:45:14 [Normal]File num to verify = 2632     <- first launch
11-24 16:01:41 [Normal]CheckAllFinish return true
11-24 16:17:59 [Normal]File num to verify = 4        <- second launch
11-24 16:17:59 [Normal]CheckAllFinish return true
```

The 2632-file check / 16.5-minute pass and the "downloading resources" stage
were gone; only 4 files were checked within the same second. The user observed
the earlier stages flash by and the time being spent at "preparing to load data".

From the fresh `output_log.txt` (F: path confirmed by `DeserializeBinary (F:/...)`):

```
16:17:15.752  Load DownloadPref: {... "0":1 ... "incrementVerify":true ...}
              (no "load pref file failed" / no "clear revision files")
16:17:15.766  IsBaseResVersionHashChanged=> baseResVersionHashInPersist =
              f5ceb98fd93b8c69603d7b5600d11a08, md5 in streamingResVersion =
              f5ceb98fd93b8c69603d7b5600d11a08
16:17:58.900  Wrote 3 persistent code switches
16:17:58.922  OnConnectRegionDispatch=> download job type = Normal
16:17:59.618  File num to verify = 4
16:17:59.775  CheckAllFinish return true
16:18:00.158  LoginMainPageContext OnDownloadBunddleFinish
```

No `Not set common res exist flag, force download` line (present on first run),
because `DownloadPref`'s `"0":1` now says the common res set exists.

## Filesystem delta, second launch vs first launch

0 added, 0 removed, 19 changed. Of those, 9 are SDK/web caches (telemetry DBs,
HoYoPass blob) and 10 are game state:

| file | what changed |
|---|---|
| `ChannelName`, `ScriptVersion`, `base_res_version_hash` | mtime only (same bytes rewritten) |
| `silence_data_versions_persist`, `silence_revision` | mtime only (re-fetched/rewritten each launch) |
| `CodeSwitch.config`, `test_file` | mtime only |
| `DownloadPref`, `DownloadError.log` | bytes changed (progress state + new log lines) |

Not touched on the second launch: `res_revision`, `res_revision_eternal`,
`data_revision`, `audio_revision`, `PatchDone`, `res_versions_persist`,
`data_versions_persist`, `ctable.dat`, `audio_lang_14`, `text_lang`, and every
hotfix file. Consequence: the game rewrites a *subset* of the marker block on a
clean launch; writing the whole block (as `docs/09` C19 specifies) is harmless
and idempotent, and the untouched files show the tool does not need to emulate
the game's selective rewrite logic.

## Verdict

A torrent-only install plus the game's first-run hotfix sync produces an install
that skips verification on subsequent launches. The skip is driven by the
marker block + persisted manifests, not by `config.ini` (still absent), the
launcher, or the caches.
