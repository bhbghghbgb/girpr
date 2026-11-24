# B4 channel gate — `hk4e_global` / `hk4e_cn` / `hk4e_bilibili`

**Verdict: B4(b) — all three channels verified. Ship `--with-persistent` for every
`--biz`.** No unverified channel needs the B4(a) usage-error fallback. Recorded
2026-11-24 UTC by re-running the three scripts with `--biz`; metadata fetched
`10:39:24Z`, dispatchers queried `10:42:39–40Z`, manifests fetched immediately
after. The `hk4e_global` numbers match the original 2026-11-23 probe.

## How the constants were obtained (evidence, not inference)

| Question | Evidence |
|---|---|
| Per-biz metadata file | `metadata/v3/stable/stamp.json` (the release/preset table): `GameName="Genshin Impact"` + `GameRegion` → `MetadataPath` = `config_GIGlb.json` (Global), `config_GICN.json` (Mainland China), `config_GICNBilibili.json` (Bilibili). `fetch_metadata.py` resolves this at runtime. |
| Subdomain / URL params | The unwrapped `GameDispatchURL` in each preset config pins the host outright: `osusadispatch` (global), `cngfdispatch` (CN), `cnqddispatch` (Bili). CN/Bili use `lang=2&time=561&key_id=4` vs global's `lang=1&time=260&key_id=5`; Bili also pins `channel_id=14&sub_channel_id=0&account_type=0`. |
| Region id | Collapse `PresetConfig.cs:815-823`: `(SubChannelID=1, ChannelID=1) → 4` (CN Main), `(0,14) → 5` (Bili). Global falls back to the registry server (USA = 0), pinned to `osusadispatch` by the live probe. Collapse `DispatchHelper.cs:173-194` maps `4 → cngfdispatch`, `5 → cnqddispatch`. |
| Channel prefix | Collapse `DispatchHelper.cs:43-46`: `OSRELWin` below region 4, `CNRELWin` from 4 up. GenshinSlimmer `GenshinSlimmer.ps1:927` uses the same CN host list. |
| Seed / key | Unwrapped `ProtoDispatchKey` and `DispatcherKey` from each preset config with the master key in `config_master.json`. Only fingerprints are committed; the raw keys stay in gitignored `raw/`. |

## Per-biz facts

| | `hk4e_global` | `hk4e_cn` | `hk4e_bilibili` |
|---|---|---|---|
| preset file | `config_GIGlb.json` | `config_GICN.json` | `config_GICNBilibili.json` |
| region id / source | 0 / registry default (USA) | 4 / preset tuple | 5 / preset tuple |
| subdomain | `osusadispatch` | `cngfdispatch` | `cnqddispatch` |
| channel + query version | `OSRELWin7.2.0` | `CNRELWin7.2.0` | `CNRELWin7.2.0` |
| seed (`ProtoDispatchKey`) | `018bc9560abc707f` | `e17f04ce947d859a` | `e17f04ce947d859a` |
| `DispatcherKeyBitLength` | 256 (bytes) | 256 | 256 |
| `DispatcherKey` fingerprint (sha256 of XML) | `30dc57baac2fd34c…` | `e18fbaf3e0e23e61…` | `e18fbaf3e0e23e61…` (same key as CN) |
| dispatcher codename (root 3→36) | `hk4e_global` | `hk4e_cn` | `hk4e_cn` — see caveat below |
| server (root 3→7) | `USA` | `CN` | `CN` |
| `properties_2` | 22101 | 22101 | **22102** |
| protobuf version fields | `7.2_live` (root 3→12, and field 22→6) | `7.2_live` | `7.2_live` |
| res revision / hash | `49530578` / `62f1ec6cdb` | same | same |
| data revision / hash | `49419538` / `604495f667` | same | same |
| silence revision / hash | `49291486` / `c25ba650a6` | same | same |
| `base_revision` body | `49008628 644b3d946e` | same | same |

All three responses decrypted with their **own unwrapped key** (global also
matches the pinned GenshinSlimmer copy). All carry the four revision fields,
both repo base URLs, the three manifest descriptors and a version field. The
query used `OSRELWin/CNRELWin7.2.0` while the response's version field is
`7.2_live` — that is the server's `<minor>_live` spelling, consistent with the
queried minor version and with the original probe; a decoder must compare the
minor version, not demand literal equality with `meta.latest`.

### Caveat for the later decoder commit (not a gate failure)

The Bilibili dispatcher returns codename `hk4e_cn`, not `hk4e_bilibili`. The
response is nevertheless unmistakably Bilibili: `properties_2 = 22102` (CN =
22101), `cnqd01-ugc-{download,upload,picture-download}` and
`ugc-api-qd(-static).mihoyo.com` URLs, and a `special_region=true` customer
service URL (field 10/44), while CN uses `cngf01-*` / `ugc-api.mihoyo.com`.
The codename check in docs/03 §7.4.4 must therefore accept `hk4e_cn` for
`--biz hk4e_bilibili`; use `properties_2` or the host for the strict channel
assertion. Codename behaviour is a decoder concern; B4's acceptance criteria
do not include it.

## Manifest URLs and MD5 / size checks

Descriptors came from the dispatcher itself: res root 3→22→3, data root 3→19,
silence root 3→20. `manifests.py` fetched each full body and compared MD5 and
byte size. **All 9 checks (3 biz × 3 manifests) matched exactly.**

Res (`output_49530578_62f1ec6cdb/client/StandaloneWindows64/res_versions_external`):

| biz | base | MD5 | size | rows |
|---|---|---|---:|---:|
| global | `autopatchhk.yuanshen.com/client_game_res/7.2_live` | `9a1b5efc2e2b11475e74c88789d0f339` | 370958 | 2631 |
| cn | `autopatchcn.yuanshen.com/client_game_res/7.2_live` | `9a1b5efc2e2b11475e74c88789d0f339` | 370958 | 2631 |
| bili | `autopatchcn.yuanshen.com/client_game_res/7.2_live` | `9a1b5efc2e2b11475e74c88789d0f339` | 370958 | 2631 |

Data (`output_49419538_604495f667/client/General/AssetBundles/data_versions`):

| biz | base | MD5 | size | rows |
|---|---|---|---:|---:|
| global | `autopatchhk.yuanshen.com/client_design_data/7.2_live` | `09f2c36615dc806f67f55e3d2c8a0c8d` | 71416 | 533 |
| cn | `autopatchcn.yuanshen.com/client_design_data/7.2_live` | `09f2c36615dc806f67f55e3d2c8a0c8d` | 71416 | 533 |
| bili | `autopatchcn.yuanshen.com/client_design_data/7.2_live` | `09f2c36615dc806f67f55e3d2c8a0c8d` | 71416 | 533 |

Silence — **newly fetched by this gate** (the 2026-11-23 probe only captured its
descriptor): `output_49291486_c25ba650a6/client_silence/General/AssetBundles/data_versions`:

| biz | base | MD5 | size | rows |
|---|---|---|---:|---:|
| global | `autopatchhk.yuanshen.com/client_design_data/7.2_live` | `2bcf33810856cb0820725300ff2d8666` | 523 | 4 |
| cn | `autopatchcn.yuanshen.com/client_design_data/7.2_live` | `2bcf33810856cb0820725300ff2d8666` | 523 | 4 |
| bili | `autopatchcn.yuanshen.com/client_design_data/7.2_live` | `2bcf33810856cb0820725300ff2d8666` | 523 | 4 |

Full URLs are recorded verbatim in `samples/{,hk4e_cn.,hk4e_bilibili.}full_manifests_summary.json`.

## NDJSON schema (identical across all three biz)

Recognised keys: `remoteName`, `md5`, `hash` (XXH64 hex), `fileSize` on every
row; extra tolerated keys `category` (496 res rows), `isPatch` (10 res + 1 data
row), `localName` (2 res rows). Row counts: 2631 res + 533 data + 4 silence =
**3168 rows per biz** (the silence count is new; the earlier count in docs/03 §7 ("3,164") covered only
res+data). All three biz serve byte-identical manifests at this revision.

## Samples committed for the offline tests

`metadata_summary.json` (per-biz, redacted), `dispatch_summary.json` and
`dispatch_fields.txt` (global plus `hk4e_cn.*` / `hk4e_bilibili.*`),
`res_versions_external.full.head.ndjson`, `data_versions.full.head.ndjson`,
`silence_data_versions.full.head.ndjson`, the global `*.names.tsv`, one
`*.hotfix_row.json` per biz/manifest, and the synthetic `samples/fixtures/`
set (see [`samples/FIXTURES.md`](samples/FIXTURES.md)). Full manifests,
ciphertext and unwrapped production keys remain out of the tree.
