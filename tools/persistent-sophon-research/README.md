# Does the server provide Sophon for Genshin `Persistent/`? — live research

**Headline answer: CONFIRMED NO — for the actual hotfix bytes.**
The server does **not** advertise any Sophon chunk manifest containing the bytes
of Genshin's in-game `Persistent/` hotfix files. It *does* advertise Sophon
chunks for the **base-install versions** of many files that also appear in the
dispatcher's persistent manifests, but those files are byte-identical to the
base install and are placed in `StreamingAssets/` by Collapse's MD5-diff rule —
they are not Persistent hotfixes.

This is a definitive answer for `hk4e_global` at the timestamp below, obtained
by querying the live API directly (not by inferring from Collapse's client-side
gate). The nuance from docs/01 §6 remains true and is now explained: Collapse's
`Repair.cs:167` gate (`string.IsNullOrEmpty(asset.AssetIndex.remoteURL)`) is a
**client-side routing decision**, but it happens to align with the server's
data layout: hotfix bytes exist only on the plain-HTTP dispatcher CDN.

**Confidence: high** for the scoped claim (global region, current dispatcher
revision 49530578, builds 7.0.0/7.1.0/7.2.0, 2026-11-23 UTC). See
[§6 Caveats](#6-caveats-and-confidence) for what cannot be proven.

---

## 1. Methodology

1. Unwrap Collapse's public release-repo metadata to recover the dispatcher URL
   template, `ProtoDispatchKey` seed and RSA key (`fetch_metadata.py`).
2. Query the live `query_cur_region` dispatcher, decrypt the RSA/protobuf
   response with a dependency-free wire parser, dump **all** fields — including
   the ones the local `.proto` omits (`dispatch.py`).
3. Fetch the two full dispatcher-derived persistent manifests, verify their
   MD5/size against the dispatcher's own description, and extract hotfix names
   (`manifests.py`).
4. Query live HYP (`getGameConfigs`, `getGameBranches`,
   `getGameDeprecatedFileConfigs`) and Sophon (`getBuild`, `getPatchBuild`),
   download and parse **every** advertised Sophon manifest (`probe.py`,
   `patch_probe.py`).
5. Cross-match every dispatcher row against every Sophon file by **normalized
   path and MD5**; specifically test whether any hotfix MD5 exists in any
   Sophon manifest (`crossmatch.py`, `hotfix_check.py`).
6. Probe CDN addressing: real chunk vs. guessed chunk/manifest under
   dispatcher-derived paths; hotfix file with Sophon-style query params and an
   HTTP Range request (`cdn_probe.py`).
7. Sweep all four pinned reference projects for code evidence; web-search for
   independent confirmation.

All live calls are unauthenticated read-only GET/POST. No files outside this
directory were modified. No full manifests or ciphertext blobs are stored —
only trimmed samples in `samples/`.

---

## 2. Live facts (exact URLs, statuses, timestamps)

All times UTC, 2026-11-23. Scripts run in order; each sample JSON records its
own timestamp.

### 2.1 Collapse release metadata — `2026-11-23T13:34:53Z`

| URL | Status | Content-Type | Bytes |
|---|---|---|---|
| `https://raw.githubusercontent.com/CollapseLauncher/CollapseLauncher-ReleaseRepo/main/metadata/v3/stable/config_master.json` | 200 | `text/plain; charset=utf-8` | 920 |
| `https://raw.githubusercontent.com/CollapseLauncher/CollapseLauncher-ReleaseRepo/main/metadata/v3/stable/config_GIGlb.json` | 200 | `text/plain; charset=utf-8` | 13211 |

`config_master.json` contains a base64 Collapse-magic blob: signature
`Collapse`, flags `0x1` (Brotli, unencrypted), compressed 610 B, decompressed
606 B — a PKCS#1 DER RSA-1024 private key, `BitSize=128`. This unwraps
(`DataCooker.cs:41-83,94-185`) the base64 blobs in `config_GIGlb.json`
(flags `0x101` = Brotli + RSA-encrypted). Recovered fields:

```
GameDispatchURL = https://{0}.yuanshen.com/query_cur_region?version={1}&lang=1&platform=3&binary=1&time=260&channel_id=1&sub_channel_id=1&account_type=1&dispatchSeed={2}&key_id=5
ProtoDispatchKey = 018bc9560abc707f
DispatcherKeyBitLength = 256
DispatcherKey = <1679-char RSA XML; sha256 30dc57baac2fd34c…>
```

The live `DispatcherKey` XML is **byte-identical** to the copy embedded in
`GenshinSlimmer.ps1:902` (verified by `fetch_metadata.py`,
`live_dispatcher_key_matches_genshinslimmer_copy: true`). Note the PS snippet at
line 918 uses an outdated unwrapping shape (`FromXmlString(base64(Key))`); the
2026 metadata uses the magic blob, which our script handles.

### 2.2 Live dispatcher — `2026-11-23T13:34:55Z`

```
https://osusadispatch.yuanshen.com/query_cur_region?version=OSRELWin7.2.0&lang=1&platform=3&binary=1&time=260&channel_id=1&sub_channel_id=1&account_type=1&dispatchSeed=018bc9560abc707f&key_id=5
```

HTTP **200**, `text/html; charset=UTF-8`, 8221 B. JSON keys exactly
`["content", "sign"]`; `content` 5888 base64 chars (23 × 256-byte RSA blocks),
`sign` 256 bytes. PKCS#1 v1.5 decrypt → **5575-byte protobuf**
(sha256 `188c9bcc25f4d89a…`). Ciphertext is non-deterministic across calls;
a stale seed returns HTTP 200 but an unusable 256-byte payload.

Decoded values (field numbers from `GenshinGateway.proto`):

| Value | Proto path | Value |
|---|---|---|
| Res revision / hash | root 3 → 22 → 1 / 5 | `49530578` / `62f1ec6cdb` |
| Data revision / hash | root 3 → 14 / 26 | `49419538` / `604495f667` |
| Silence revision / hash | root 3 → 18 / 27 | `49291486` / `c25ba650a6` |
| Repo base URLs | root 3 → 8 / 9 | `https://autopatchhk.yuanshen.com/client_game_res/7.2_live` and `.../client_design_data/7.2_live` |
| Game codename / server | root 3 → 36 / 7 | `hk4e_global` / `USA` |

**Unknown fields decoded generically** (`samples/dispatch_fields.txt`):

* Root **11** = 2076-byte opaque binary (starts `45 63 32 62 10 00…`, not UTF-8,
  no URL/`sophon`/`chunk` marker); root **12** = 538-byte opaque binary;
  root **14** = base64 string `hWGBV1ZCxKJ8cJVOjTlKYlh1qnul2XxEnILDo97JFCM=`.
* Properties **38–47** = UGC/customer-support URLs only
  (`osusa-ugc-download.yuanshen.com`, `us-ugc-api.hoyoverse.com`,
  `cs.hoyoverse.com/csc-service-center-fe/...`, act.hoyoverse.com event pages).
  None contains "sophon", "chunk", "persistent" or a manifest id.

→ **No hidden Sophon pointer in the dispatcher response.**

### 2.3 Full persistent manifests — same minute

| URL | Status | Content-Type | Bytes | MD5 vs dispatcher |
|---|---|---|---|---|
| `https://autopatchhk.yuanshen.com/client_game_res/7.2_live/output_49530578_62f1ec6cdb/client/StandaloneWindows64/res_versions_external` | 200 | `application/octet-stream` | 370958 | `9a1b5efc…` ✅ exact |
| `https://autopatchhk.yuanshen.com/client_design_data/7.2_live/output_49419538_604495f667/client/General/AssetBundles/data_versions` | 200 | `application/octet-stream;charset=UTF-8` | 71416 | `09f2c366…` ✅ exact |
| `…/StandaloneWindows64/base_revision` | 200 | `application/octet-stream` | 19 | body `49008628 644b3d946e` |

Parsed: **2631** res rows + **533** data rows = **3164** dispatcher rows. Live
schema keys: `remoteName`, `md5`, `hash` (XXH64 hex), `fileSize`, plus
`category` (496 rows), `isPatch` (10 rows), `localName` (2 rows). Note:
docs/01 §6.4.4 says `isPatch` never comes from the manifest because Collapse's
`PropertyVersioning.cs:29-30` marks it `[JsonIgnore]` — the **server does send
it** (10 rows), Collapse simply discards it. Those 10 server-flagged rows:

```
vulkan_gpu_list_config.txt, blocks/00/08897784.blk, blocks/00/09043028.blk,
blocks/00/09090931.blk, blocks/00/16616174.blk, blocks/00/31049740.blk,
svc_catalog (localName ../svc_catalog),
English(US)/VO_7.2_a_0.pck, Japanese/VO_7.2_a_0.pck, Japanese/VO_7.2_b_0.pck
```

### 2.4 HYP / Sophon API — `2026-11-23T13:36:01Z`

| URL | Status | Content-Type | Bytes |
|---|---|---|---|
| `https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api/getGameConfigs?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3` | 200 | `application/json` | 1529 |
| `…/getGameBranches?…` | 200 | `application/json` | 912 |
| `…/getGameDeprecatedFileConfigs?…&channel=1&sub_channel=0` | 200 | `application/json` | 1019 |
| `https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getBuild?branch=main&package_id=ScSYQBFhu9&password=bDL4JUHL625x` | 200 | `application/json` | 4519 |
| `https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api/getPatchBuild?…` (**POST**) | 200 | `application/json` | 4461 |

Branch: `main`, tag `7.2.0`, diff_tags `7.1.0`, `7.0.0`, build
`msj5r4DqSLOw`. Five Sophon manifests advertised: `game` (2795 files,
checksum `2b4ef596e7440779759717433d1c86d9`, 9,268,781 compressed /
17,314,861 uncompressed bytes) plus `zh-cn`, `en-us`, `ja-jp`, `ko-kr`
(179 files each). All downloaded HTTP 200 `application/octet-stream`.
Manifest URL shape:

```
https://autopatchhk.yuanshen.com/client_app/sophon/manifests/cxhpq4g4rgg0/msj5r4DqSLOw/manifest_92303edbe132c634_2b4ef596e7440779759717433d1c86d9
```

Chunk URL shape (verified with a real chunk `d45934cfd771b6c1_7e8c7dc82783c67143bf5ef153073ea7`, 400,412 B):

```
https://autopatchhk.yuanshen.com/client_app/sophon/chunks/cxhpq4g4rgg0/msj5r4DqSLOw/<chunk_id>   → HTTP 200
```

---

## 3. The decisive cross-match

`crossmatch.py` normalizes Sophon paths the way Collapse maps dispatcher
`remoteName`s (`Fetch.Persistent.cs:239-278`): strip `GenshinImpact_Data/`,
strip `StreamingAssets/AssetBundles/` for `blocks*.blk`, strip
`StreamingAssets/` otherwise. `hotfix_check.py` then compares **MD5s across all
15 advertised manifests** (5 per build × builds 7.2.0, 7.1.0, 7.0.0).

| Category | Count | Meaning |
|---|---:|---|
| `same_path_same_md5` | **1661** | Base-install bytes. Sophon has them; Collapse's MD5-diff rule puts them in `StreamingAssets`, **not** Persistent. |
| `same_path_diff_md5_hotfix_md5_present` | **0** | Genuine hotfix bytes found in a Sophon manifest — **none**. |
| `same_path_diff_md5_hotfix_md5_absent` | **369** | Real hotfixes: base Sophon file exists at that path, but the hotfix MD5 is in no Sophon build. |
| `path_not_in_sophon` | 1134 | Of these, **1127** have their MD5 at a *different* Sophon path (base copies under `AudioAssets/`, `ctable_streaming.dat`, etc.); only **7** are new hotfix-only files (e.g. `English(US)/VO_7.2_a_0.pck`). |
| **Dispatcher MD5 absent from every Sophon build** | **376** | 369 + 7 = the complete hotfix-only byte set. Includes all 10 server `isPatch:true` rows. |

The game manifest contains exactly **one** path with the word `Persistent`:
`GenshinImpact_Data/StreamingAssets/Persistent/plugindelayed.config` — a plugin
config under StreamingAssets, not a game hotfix. No `Persistent/blocks…`, no
`Persistent/*.pck`, no hotfix MD5.

Example hotfix absent from all 15 Sophon manifests:

```
remoteName = blocks/00/08897784.blk
md5        = f8c48706979ae735de2f9d45652fa218   (isPatch: true from dispatcher)
hash       = 07d60c5760780d17
fileSize   = 119052
```

### CDN probes (`cdn_probe.py`)

| Probe | Result |
|---|---|
| Real chunk at advertised `client_app/sophon/chunks/...` prefix | **200**, `application/octet-stream`, `Accept-Ranges: bytes` |
| Same chunk id under `output_49530578_62f1ec6cdb/client/sophon/chunks/...` | **404**, `application/xml` |
| Same chunk id under `…/StandaloneWindows64/sophon/chunks/...` | **404** |
| Same chunk id under `…/client_app/sophon/chunks/...` | **404** |
| Manifest id under dispatcher-derived path / `StandaloneWindows64/` | **404** |
| Hotfix full file at `…/StandaloneWindows64/AssetBundles/blocks/00/08897784.blk` | **200**, 119052 B, MD5 `f8c48706979ae735de2f9d45652fa218` ✅ |
| Same URL + `?chunk=1` / `?sophon=1` | **200**, identical full file — query params ignored |
| Same URL with `Range: bytes=0-15` | **206**, 16 bytes, `Accept-Ranges: bytes` |

→ There is no Sophon-style addressing on the dispatcher CDN, and no dispatcher
bytes on the Sophon CDN. The two delivery systems are disjoint at the byte level
for genuine hotfixes.

### Sophon patch endpoint (`patch_probe.py`)

`getPatchBuild` (POST, tag 7.2.0) advertises 5 patch manifests (`game` +
4 languages). All downloaded HTTP 200. Raw decompressed bytes were searched for
100 known hotfix MD5 hex strings and for `Persistent`/`hotfix`/`external`:

* `game` patch manifest: 901,266 B, `blocks/` ×3953, `persistent` ×1 (the same
  plugin config), `hotfix` ×0, **hotfix MD5 hits 0**.
* All 4 language patch manifests: `external` ×96 (unrelated string), `persistent` ×0,
  **hotfix MD5 hits 0**.

---

## 4. Local source sweep (all four reference projects)

* `reference-projects/Collapse` — Sophon is implemented in `Hi3Helper.Sophon`
  and used by the launcher's install/update/predownload paths. The GI repair
  path explicitly overrides Sophon assets with dispatcher assets:
  `Fetch.Persistent.cs:188-193` assigns `remoteURL`/`remoteURLAlternative` from
  dispatcher URLs; `Repair.cs:167` then routes on `remoteURL` being empty.
  No code fetches a Sophon manifest for Persistent files. Collapse's test
  harness (`Hi3Helper.Sophon/Test/SophonUpdatePreload/launchSettings.json:5`)
  references `getPatchBuild` with `ScatteredFiles` URLs — a version-update
  workflow, not the dispatcher hotfix path.
* `reference-projects/Starward` — Sophon only (`HoYoPlayClient.cs:534-612`,
  `GameInstallService.cs`, `Sophon.proto`). Zero `Persistent`/`hotfix` handling;
  no dispatcher client at all.
* `reference-projects/GenshinSlimmer` — dispatcher-only; its protobuf parser
  (`Parse-ProtoFields`, 860-884) and `query_cur_region` call (901-991) match our
  live findings. No Sophon usage.
* `reference-projects/GI-Live-Hotfix-Downloader` — dispatcher-only; writes
  `GenshinImpact_Data/Persistent` from `res_versions_external`
  (`Tools/Release.cs:45-55`). No Sophon usage.

No counterexample found in any reference project.

## 5. Web research (fact vs speculation)

Searches: `Genshin Impact Sophon chunk manifest Persistent hotfix
res_versions_external`; `github Collapse Launcher Genshin persistent repair
Sophon remoteURL issue`; `"sophon" "persistent" Genshin hk4e chunk downloader
hotfix`; plus GitHub issue search APIs.

* <https://github.com/CollapseLauncher/Hi3Helper.Sophon> — defines Sophon as a
  chunked download method for HoYoverse games; no mention of dispatcher
  persistent/hotfix files. **(Fact, no supporting evidence for the hypothesis.)**
* <https://github.com/CollapseLauncher/Collapse/pull/608> — changelog: "Fixed
  IODenied error in Genshin repair … caused by an asset wrongly marked as
  Persistent while actually being in StreamingAssets." Confirms the
  client-side placement problem, not server-side Sophon. **(Fact, tangential.)**
* <https://github.com/CollapseLauncher/Collapse/pull/505> / <https://github.com/CollapseLauncher/Collapse/pull/504> —
  HSR fixes for "patch-marked files under Persistent Folder" / "file marked
  under Persistent but found under StreamingAssets". Again placement logic.
  **(Fact, tangential.)**
* GitHub issue search `repo:CollapseLauncher/Collapse persistent sophon` → 5
  results, none about Sophon-served persistent files; issue search for
  `GI-Live-Hotfix-Downloader sophon` → 0 results.
  **(Fact: no public evidence found.)**
* CommunitySophon tools (`GesthosNetwork/HK4E-Sophon-Downloader`,
  `yaagl/yet-another-anime-game-launcher` via deepwiki) describe `getBuild`
  chunk install/update only — no Persistent path.

No source found claiming the server provides Sophon chunks for GI hotfixes;
no source found contradicting our live result. This is a **negative web result**,
not the basis of the conclusion — the live byte comparison is.

---

## 6. Caveats and confidence

* **Scope.** One region (`osusadispatch`), one client version (`OSRELWin7.2.0`),
  dispatcher res revision `49530578`, snapshot 2026-11-23 UTC. CN/Bili regions
  use different keys/seeds and were not queried. HoYo can change the layout at
  any time; re-run the scripts to re-check.
* **What is proven.** Across **all 15 Sophon chunk manifests advertised by the
  live API** (3 builds) and **all 5 patch manifests**, none contains any of the
  **376** hotfix-only MD5s of the current dispatcher manifests. The advertised
  manifest list is the server's own enumeration; an entirely unadvertised
  private manifest cannot be disproved, but no endpoint or code references one.
* **Matching key.** Sophon manifests carry MD5 (field 5), not XXH64; we compared
  MD5. MD5 collision is not a practical concern here.
* **The 1661 overlap is not a counterexample.** Those are base-install files
  (`StreamingAssets/AssetBundles/…`, audio `.pck` under `AudioAssets/`) whose
  bytes are identical to dispatcher entries. Collapse's `IsRequireForcePersistent`
  (`Fetch.Persistent.cs:200-211`) forces a dispatcher file into `Persistent/`
  only when the local base path has a **different MD5** — exactly our 369+7
  hotfix set. Sophon covers the base half; HTTP covers the hotfix half.
* **Client-side gate remains.** docs/01 §6 correctly states Collapse always
  uses HTTP for dispatcher assets because `remoteURL` is assigned. The new
  finding explains *why that is safe* and rules out a hidden Sophon fast path.

---

## 7. Implications for girpr

See `FINDINGS.md` for the concise version. Short version:

1. Keep the design in docs/03 §7: dispatcher query → persistent manifests → full-file
   HTTP download with alt URL → MD5 re-verify → markers last. **No Sophon
   chunk-fetch path is needed for Persistent.**
2. girpr's existing Sophon machinery remains correct for base assets; no need
   to extend it to hotfix bytes. But do not assume a dispatcher row with a
   matching Sophon path can be repaired from Sophon *into `Persistent/`* —
   that would silently write the base version instead of the hotfix.
3. The hotfix CDN supports **HTTP Range (206)** and `Accept-Ranges: bytes`,
   though Collapse does not use it. girpr can adopt ranged/resumable full-file
   downloads as an improvement without changing the design.
4. All 376 hotfix-only rows need plain HTTP; at current revision that includes
   ten `[isPatch]` rows and seven new-only files. The alt URL must be kept
   (`Fetch.Persistent.cs:190`).
5. Marker writes, purge keep-set changes, atomic manifest writes, blank-line
   tolerance and post-download MD5 verification from docs/01 §§6.4.2–6.4.4 are
   unchanged and still required.
6. New fact to pin in tests: live manifests carry `isPatch`, `localName` and
   `category`; a parser must tolerate (and ignore, per Collapse) those keys.

---

## 8. Scripts and reproduction

Python 3.14, `uv`, deps in `requirements.txt`. From this directory:

```powershell
uv venv .venv
uv pip install --python .venv/Scripts/python.exe -r requirements.txt
.venv/Scripts/python.exe fetch_metadata.py     # unwraps metadata (redacted summary)
.venv/Scripts/python.exe dispatch.py           # live query_cur_region + protobuf dump
.venv/Scripts/python.exe manifests.py          # full NDJSON, verifies md5/size
.venv/Scripts/python.exe probe.py --download-manifests   # HYP + getBuild + all manifests
.venv/Scripts/python.exe crossmatch.py         # path+md5 cross-match (current build)
.venv/Scripts/python.exe hotfix_check.py       # 3 builds, definitive md5 pool test
.venv/Scripts/python.exe patch_probe.py        # getPatchBuild raw-byte search
.venv/Scripts/python.exe cdn_probe.py          # chunk/manifest/full-file URL probes
```

Order matters: `dispatch.py` reads `samples/metadata_summary.json`;
`manifests.py` reads `samples/dispatch_summary.json`; `probe.py` /
`crossmatch.py` / `hotfix_check.py` / `cdn_probe.py` / `patch_probe.py` read the
samples produced by their predecessors. Every script is re-runnable and
overwrites its own samples only. `.gitignore` excludes `.venv/`, `__pycache__/`
and raw captures.

### Sample index (`samples/`)

* `metadata_summary.json` — redacted metadata fields, key fingerprints.
* `dispatch_summary.json`, `dispatch_fields.txt` — all protobuf fields and NDJSON stats.
* `res_versions_external.head.ndjson`, `data_versions.head.ndjson`, `silence_data_versions.head.ndjson` — first lines from dispatcher protobuf JSON.
* `full_manifests_summary.json`, `*.full.head.ndjson` — full-file manifest stats + first 40 lines.
* `*.names.tsv` — compact `remoteName ⇥ md5 ⇥ hash ⇥ fileSize` tables (no file bodies).
* `probe_summary.json` — HYP/getBuild/manifest enumeration and path samples.
* `crossmatch_summary.json`, `hotfix_check_summary.json` — exact counts and examples.
* `cdn_probe_summary.json`, `patch_probe_summary.json` — status codes and raw-byte hits.

---

## 9. Provenance / licensing notes

* Collapse metadata and release repo are public:
  <https://github.com/CollapseLauncher/CollapseLauncher-ReleaseRepo> (MIT for
  the launcher source; metadata files are public distribution data). The
  dispatcher RSA key is published client/launcher material. Only fingerprints
  are stored here; the master RSA key is never written to disk.
* `dispatch_rsa.xml` is copied verbatim from the pinned public submodule
  `reference-projects/GenshinSlimmer/GenshinSlimmer.ps1:902`
  (<https://github.com/dnullptr/GenshinSlimmer>). It exists only to make the
  probes reproducible.
* The dispatcher/CDN URLs are public endpoints used by the game itself;
  no credentials, account data or personal information appear in any sample.
