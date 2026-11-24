# Hand-off / how to reproduce

Companion to `README.md`. Only `README.md`, this file, `.gitignore` and `scripts/`
are committed. Everything under `generated/` is regenerated and gitignored.

## Environment / constraints (inherited from the prior research)

* Working tree `E:\hoccode\hocrust\girpr`; context `docs/01-03`, `docs/08`,
  `tools/persistent-{launch,sophon}-research/`.
* `F:\Genshin Impact game` = torrent copy (8,068 files / 144.53 GiB), now with a
  full synced `Persistent/` (393 files) and no `config.ini`; all base files
  unchanged. `E:\Volume 1\gem\power\Genshin Impact game` = older synced reference
  (no `BeyondAssets/`), launched successfully before.
* Both are HDDs: **never parallelise hashing within one disk**. This research
  hashes files on F: only, sequentially. E: and F: could be worked concurrently
  but were not needed here. If F: disappears, stop and wait for the user.
* Never modify F: game files in place; never write into `reference-projects/`.
  Only the user can launch the game (escalation + login); prefer the game closed.

## Reproduce

```powershell
$py = 'E:\hoccode\hocrust\girpr\tools\persistent-sophon-research\.venv\Scripts\python.exe'
cd 'E:\hoccode\hocrust\girpr\tools\sophon-minimal-research'

& $py scripts\dump_sophon_files.py    # live getBuild -> generated/sophon_files_720.tsv
& $py scripts\compare_sets.py         # -> generated/compare_summary.json, set stats + F: MD5 check
```

Both scripts import (do not copy) `common.py`, `probe.py` and `crossmatch.py` from
`tools/persistent-sophon-research`, so the live queries and the protobuf parser
stay single-sourced. `dump_sophon_files.py` reads
`persistent-sophon-research/samples/probe_summary.json` for the branch package id;
regenerate that first with `probe.py` if the samples are missing.

Starward audit (read-only; no script — plain grep over
`reference-projects/Starward/src`):

```powershell
rg -i 'persistent|res_versions|data_versions|PatchDone|ScriptVersion|ChannelName|hotfix|ctable|base_res_version_hash' reference-projects/Starward/src   # -> 0 matches
rg 'pkg_version|SetGameConfigIniAsync|SetAudioLanguageAsync|ClearDeprecatedFiles' reference-projects/Starward/src
```

## Key numbers to expect (2026-11-24 UTC snapshot)

* Sophon 7.2.0: `game` 2,795 + 4×179 audio = 3,511 unique paths.
* `game`+`en-us` (selected voice) = 2,974 files / 154,675,254,051 bytes.
* Torrent = 8,068 files / 155,182,786,241 bytes = those 2,974 + 5,093
  `BeyondAssets/BeyondAssistEditor/*` + `beyond_pkg_version` (507,532,190 bytes).
* All 11 named target files (10 in Sophon + exe) MD5-match the F: base bytes;
  `res_versions_streaming` = `f5ceb98fd93b8c69603d7b5600d11a08`.

## Optional live check (only with the user's participation)

If a direct launch demonstration is wanted, build a Sophon-only **hardlink** copy
on F: (same volume; no extra ~144 GB) and let the user launch it:

1. Create a new empty dir on F:, e.g. `F:\sophon-only`.
2. For each of the 2,974 `(game + en-us)` paths from
   `generated/sophon_files_720.tsv`, `New-Item -ItemType HardLink` from the
   corresponding F: game file (preserve relative paths). Do **not** hardlink or
   modify the existing game's `Persistent/` or `config.ini`.
3. Ask the user to launch `F:\sophon-only\GenshinImpact.exe`, log in, press Start,
   and report whether the one-time verification/download appears; capture the
   user-data logs in `%LOCALAPPDATA%\miHoYo\Genshin Impact\` afterwards.
4. Expected: it launches (set-equivalent to torrent minus UGC editor) and, with no
   `Persistent/`, performs the known 2,632-file check + ~1.08 GiB hotfix download.
   That is the pre-existing launcher-free behaviour, not a Sophon defect.
5. Remove the scratch tree when done (`Remove-Item -Recurse` on the links only —
   this does not touch the original files).

This step was **not** performed here; the conclusion rests on set equivalence and
the E: reference. End the turn with the game closed before resuming any launch test.

## What would change the conclusion

* A future revision where a named `*_streaming` file stops being in the Sophon
  manifest — re-run `dump_sophon_files.py` and check the TARGET section.
* CN/Bili regions (different branches/manifests) — not probed.
* Any evidence that BeyondAssets is required for launch — contradicted today by
  E:'s successful launches without it.
