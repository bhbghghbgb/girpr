# What does a Sophon-only GI 7.2.0 install contain — and is it launchable?

**Research question (adjacent to `tools/persistent-launch-research`).** The F: torrent
copy is known runnable but has no `Persistent/`. Launchers repair via Sophon (HYP
`getBuild` chunk manifests). Does a copy containing *only* Sophon-tracked files
launch, and specifically: are the `*_streaming` metadata files that drive the
in-game verifier (`base_res_version_hash` = MD5 of
`StreamingAssets/res_versions_streaming`) part of the Sophon set?

**Headline answer: YES, all of them are in Sophon.** Every `*_streaming` /
`*_revision` / `svc_catalog` / `ctable_streaming.dat` / `pkg_version` file the
question names is a normal Sophon-tracked base file, byte-identical to the
torrent copy. A Sophon repair restores them, so girpr needs **no new fetch path**
for them. The only thing a Sophon-only install lacks for *not re-verifying* is the
`Persistent/` marker block — already the subject of the shipped opt-in Persistent
feature (docs/01 §6 / docs/03 §7), not a Sophon gap.

---

## 1. Authoritative vs. generated (what is safe to commit)

| Kind | Location | Committed? | Notes |
|---|---|---|---|
| Method + conclusions | this `README.md`, `PROCESS.md` | **yes** | summaries only, no raw inventories |
| Scripts | `scripts/*.py` | **yes** | re-runnable; wrap the existing `tools/persistent-sophon-research` helpers, no duplicated logic |
| Live Sophon file list | `generated/sophon_files_720.tsv` (3,511 rows) | **no** (`.gitignore`) | re-downloaded manifests; full dumps are deliberately not committed |
| Set comparison + MD5 check | `generated/compare_summary.json` | **no** | regenerate with `scripts/compare_sets.py` |
| Torrent/base copy inventory | `../persistent-launch-research/snapshots/F-fresh/F-snapshot.jsonl` | **no** (already gitignored upstream) | fresh F: copy == torrent path set (proved in prior research) |
| Prior findings | `../persistent-launch-research/notes/*.md`, `../persistent-sophon-research/{README,FINDINGS}.md`, docs/01 §6 | **yes** | authoritative for launch behaviour and dispatcher-vs-Sophon byte layout |

Nothing was written into `reference-projects/`. No game bytes were downloaded
(manifests only, a few MB). No F: game file was modified.

---

## 2. Method

1. Re-ran the live `getBuild` probe for `hk4e_global` (branch `main`, tag `7.2.0`,
   package `ScSYQBFhu9`) using the re-runnable helpers in
   `tools/persistent-sophon-research` (`common.py`, `probe.py`, `crossmatch.py`).
   Downloaded and zstd/gzip-decoded all 5 advertised manifests and parsed every
   `SophonChunkFile` path/size/MD5 (`scripts/dump_sophon_files.py`).
2. Compared that exact path set to the gitignored `F-snapshot.jsonl` (the fresh
   torrent copy, 8,068 files; path set == torrent, per prior research)
   (`scripts/compare_sets.py`).
3. Hashed the 11 named files **on F: only, sequentially** (one HDD) and compared
   their MD5 to the manifest MD5 — proving a Sophon repair restores byte-identical
   verification inputs.
4. Cross-checked the dispatcher hotfix names
   (`../persistent-sophon-research/samples/res_versions_external.names.tsv`) to see
   whether any `*_streaming` file is hotfixed (i.e. could be stale in a base copy).
5. Audited `reference-projects/Starward/src` for every write/require around
   `config.ini`, `pkg_version`, `*_streaming`, audio, SDK/WPF and Persistent.

---

## 3. Evidence

### 3.1 Exact Sophon file set, build 7.2.0

Live `getBuild` advertises 5 manifests:

| manifest (`matching_field`) | files | uncompressed bytes |
|---|---:|---:|
| `game` | 2,795 | 135,834,094,560 |
| `en-us` | 179 | 18,841,159,491 |
| `zh-cn` | 179 | 14,535,288,274 |
| `ja-jp` | 179 | 17,238,542,671 |
| `ko-kr` | 179 | 15,301,272,770 |
| **all 5 (unique paths)** | **3,511** | 201,750,357,766 |

The 4 language manifests are the same `.pck` set for 4 voice languages; Starward
and girpr filter to the selected language. With only English selected the set is
**`game` + `en-us` = 2,974 files / 154,675,254,051 bytes**.

### 3.2 Sophon(7.2.0, game+en-us) == torrent minus the UGC editor

| set | files | bytes |
|---|---:|---:|
| Sophon unique paths (all 4 VO) | 3,511 | 201,750,357,766 |
| Torrent / fresh F: copy | 8,068 | 155,182,786,241 |
| intersection | **2,974** | — |
| Sophon-only (all are zh/ja/ko audio + their `Audio_*_pkg_version`) | 537 | — |
| torrent-only | **5,094** | 507,532,190 |

Every one of the 5,094 torrent-only paths is either
`BeyondAssets/BeyondAssistEditor/…` (5,093 files) or root `beyond_pkg_version`
(1 file). Arithmetic is exact:

```
154,675,254,051 (Sophon game+en-us) + 507,532,190 (BeyondAssets) = 155,182,786,241 (torrent)
```

So the Sophon-tracked set is byte-for-byte (by path and size) the **entire known
runnable torrent game minus the BeyondAssets UGC editor**. The editor is
explicitly out of scope in docs/01 §6, and the E: synced reference install — which
launched successfully in the prior research — has no `BeyondAssets/` at all.
**A Sophon-only copy is therefore runnable.** (This is set-equivalence + E:
empirical parity, not a fresh launch of a hardlinked copy in this research; see
`PROCESS.md` for the optional live check.)

### 3.3 The named files are all in Sophon, and byte-identical to the torrent base

| file | in Sophon? | Sophon MD5 == F: MD5? |
|---|---|---|
| `StreamingAssets/res_versions_streaming` | yes | **yes** `f5ceb98fd93b8c69603d7b5600d11a08` |
| `StreamingAssets/data_versions_streaming` | yes | yes `6a17c9881ccd61515875f4da298e1c31` |
| `StreamingAssets/silence_versions_streaming` | yes | yes `467660a0be27940bff89714f2ba0043d` |
| `StreamingAssets/VideoAssets/video_versions_streaming` | yes | yes `164eef8d33fe6b8039087b0b7b728de1` |
| `StreamingAssets/AssetBundles/res_revision` | yes | yes `f991471795f68ac6b679e0e61464c955` |
| `StreamingAssets/AssetBundles/data_revision` | yes | yes `f991471795f68ac6b679e0e61464c955` |
| `StreamingAssets/AssetBundles/silence_revision` | yes | yes `d944bba0b2d2d77ba102df0aa53af79f` |
| `StreamingAssets/ctable_streaming.dat` | yes | yes `cd5f73407c6c4733f3c86a145e9f94c0` |
| `StreamingAssets/svc_catalog` | yes | yes `c58968de11c7df4cb8ede4d3241f8498` |
| root `pkg_version` | yes | yes `8a2d95fb1caadf8eabcf80343a00c199` |
| `GenshinImpact.exe` | yes | yes `f2066f2954541dba36380445910310a0` |
| root `config.ini` | **no** | n/a — launcher bookkeeping; game launches without it (prior research) |

`res_versions_streaming`'s MD5 is exactly the `base_res_version_hash` value
established in `tools/persistent-launch-research`. **Because that source file is
Sophon-tracked, girpr can always recompute the marker after a Sophon repair.**

Do not confuse the two similarly named families:

* `StreamingAssets/AssetBundles/{res,data,silence}_revision` — **base** files, in
  Sophon, MD5-matched above (values from the 7.2.0 base build).
* `Persistent/{res,data,silence}_revision` (+ `res_revision_eternal`) — **hotfix
  markers** written from the dispatcher (e.g. `49530578`), **not** in Sophon.

Dispatcher cross-check: only `svc_catalog` of the named files also appears in
`res_versions_external` (row `svc_catalog / c58968de… / a818633d… / 202217`). Its
MD5 is **identical** to the base/Sophon copy, so Collapse's MD5-diff rule keeps it
in `StreamingAssets`; it is not a hotfix. None of the other `*_streaming` files
appear in any dispatcher manifest. Hence these base-verification inputs cannot be
"stale" because of hotfixing — they are always the base version, by design, and
that is exactly what `base_res_version_hash` is defined against.

### 3.4 Starward as the minimal-launcher reference

`reference-projects/Starward/src` contains **zero** occurrences of `Persistent`,
`res_versions*`, `data_versions*`, `PatchDone`, `ScriptVersion`, `ChannelName`,
`hotfix`, `ctable` or `base_res_version_hash` (case-insensitive grep). docs/01 §6.2.5 is confirmed. Concrete writes/requires:

* **`config.ini`** — `GameInstallService.cs:788-850` (`SetGameConfigIniAsync`):
  writes `[General]` with `game_version` (latest, or local for predownload),
  `channel`, `sub_channel`, `cps` (`hyp_hoyoverse` global), `sdk_version`,
  `game_biz`, `wpf_version` (when a WPF package exists). Read for version detection
  at `GamePackageService.cs:47-65` and `GameLauncherService.cs:144-160`.
* **Audio selection** — `GamePackageService.cs:83-98` (`SetAudioLanguageAsync`)
  writes the selected languages to `GameConfig.AudioPackageScanDir`. The live HYP
  config (`samples/probe_summary.json`) sets that to
  `GenshinImpact_Data/Persistent/audio_lang_14`. **Refinement of §2.5:** Starward
  does create one file under `Persistent/`, but only via this config-driven path —
  there is no literal `Persistent` string in its source. File is created with
  `Directory.CreateDirectory` first.
* **`pkg_version` — read, never written.** Two read sites: (a) legacy
  `SingleFile` fallback when `ResListUrl` is present
  (`GamePackageService.cs:561-572` → `GetPkgVersionItemsAsync` `:868-896` →
  `DownloadAndParsePkgVersionAsync` `:900`); (b) SDK re-extraction gate
  (`GameInstallHelper.cs:938-960`, using `GameChannelSDK.PkgVersionFileName`).
  It is not synthesized like Collapse's fake `pkg_version`; for 7.2.0 it simply
  arrives as an ordinary Sophon-tracked file (§3.3).
* **`*_streaming` files — no code at all.** They are downloaded like any other
  base file from the Sophon chunk manifests; no special generation or verification.
* **SDK / WPF** — `GameInstallHelper.cs:922-976` (`DownloadGameChannelSDKAsync`,
  7z extract to install root, gated by `config.ini sdk_version` + `pkg_version`
  integrity check) and `:1003+` (`DownloadWPFPackageAsync`, gated by
  `wpf_version`). Called from `GameInstallService.cs:332-334` / `460-461` / `682-683`.
* **Audio cache→res move** — `GameInstallService.cs:425-445` (`File.Move` from
  `AudioPackageCacheDir` to `AudioPackageResDir` if they differ; live config names
  `Persistent/AudioAssets` → `StreamingAssets/AudioAssets`).
* **Pre-task read-only strip** — `GameInstallService.cs:232-236` tree-wide
  `File.SetAttributes(Normal)`.
* **Transient cleanup** — `GameInstallService.cs:722-777` (`ClearDeprecatedFiles`,
  post-task only, skipped for predownload): compressed packages, the HYP deprecated
  list, and (when no predownload) `*_tmp`, `*.hdiff`, dirs `chunk/`, `ldiff/`,
  `staging/`.
* `GameConfig` has `enable_write_verify_result` / `write_verify_result_path`
  fields, but no code reads them and the live HYP sets them `false`/empty — Starward
  writes no verification markers.

So Starward's launchable-install recipe is exactly: **Sophon base files (which
include every `*_streaming` file and `pkg_version`) + `config.ini` + voice
selection + optional SDK/WPF + cleanup**. No `Persistent/` verification state.

---

## 4. Conclusions / what girpr must do

1. **No new file set is needed.** girpr's existing Sophon repair already restores
   every `*_streaming` file, `svc_catalog`, `ctable_streaming.dat` and `pkg_version`
   — byte-identical to the torrent. They cannot be missing or stale after a
   successful Sophon repair, and girpr's purge (manifest-membership keep rule)
   already protects them.
2. **`base_res_version_hash` is computable from any Sophon-repaired install.**
   girpr must keep writing it (MD5 of `StreamingAssets/res_versions_streaming`,
   docs/03 §7) and must never let the purge delete it. C16's "omit if source missing"
   case is unreachable on a normal Sophon install.
3. **Launchability of a Sophon-only copy is fine.** It is the torrent game minus
   the BeyondAssets UGC editor; E: (no BeyondAssets) is a proven launchable
   reference. `config.ini` is not in Sophon but girpr already writes it in the post
   phase (`src/repair/phases/post.rs:147`) and keeps it in the purge.
4. **"Does not re-verify" is the remaining gap, and it is already scoped.** With no
   `Persistent/` the game does a one-time 2,632-file check + ~1.08 GiB dispatcher
   hotfix download. That is fixed by the opt-in dispatcher/Persistent support in
   docs/01 §6 / docs/03 §7 (full-file HTTP hotfixes + `*_persist` mirrors + revision markers +
   `base_res_version_hash` last), *not* by any extra Sophon work. Per
   docs/01 §6.4.5 those hotfix bytes have no Sophon path.
5. **Net change to girpr: none for this question.** Keep the current Sophon repair,
   the existing `config.ini` bump, and the shipped Persistent tier. No special
   handling for the `*streaming` files, no `pkg_version` synthesis, no fetch of
   `svc_catalog`/`ctable_streaming.dat` outside the Sophon plan.
6. **One plan-time fix, not a new workstream.** docs/03 §7.5.2 item 1 copies
   `StreamingAssets/ctable.dat` → `ctable_streaming.dat` as a Collapse-parity
   prerequisite, but neither the Sophon manifest nor the F: torrent copy contains
   `StreamingAssets/ctable.dat` (it is an E:-only launcher artifact; prior research
   `notes/00`/`notes/01`). On a Sophon-only install that source is absent while the
   target already ships byte-identical in Sophon. The step is therefore a
   conditional no-op on a missing source and must never overwrite an intact target.
   docs/01 §6.2.3/§6.4.4 and docs/03 §7.1.3/§7.5.2/§7.11 were updated accordingly.

### Caveats

* One live snapshot (`hk4e_global`, tag `7.2.0`, re-queried 2026-11-24 UTC); CN/Bili
  regions and future revisions were not probed — re-run `scripts/` to refresh.
* "Runnable" is established by exact set equality with the known-runnable torrent
  minus BeyondAssets plus E:'s successful launches without BeyondAssets, not by a
  fresh launch in this research. The optional hardlink launch check is in
  `PROCESS.md`.
* F: still holds the full torrent copy + a synced `Persistent/` and no `config.ini`;
  nothing in this research changed it.
