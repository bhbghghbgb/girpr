# Persistent-directory update — implementation plan

**Status: plan. Implements [`docs/08-persistent-update-research.md`](08-persistent-update-research.md).**
**Scope: `girpr` product crates, the offline mock fixtures, `gircap`'s bundle format, the
artifact bridge, and the user-facing docs. The three research scripts in
`tools/persistent-sophon-research/` are extended only to capture the fixtures the
tests need.**

This is a new tier, like [`docs/04`](04-scope-limitation-spec.md) was: it does
not amend the repair-spec series (`docs/01`–`03`), the scope spec, or the artifact
bridge spec. It adds one opt-in phase group to the same pipeline.

Every answer in `docs/08` §5 is an **input** to this plan, not an open question.
§9 maps each C/B item to the section, commit and test that enforces it. The one
research item that is explicitly deferred to implementation time is **B4**
(channel coverage for CN/Bili); §2 is a gated first commit with a fallback to
the global-only answer (B4(a)).

---

## 1. The feature, in one page

### 1.1 The opt-in invariant

One flag, `--with-persistent` (B1(a)), enables the whole dispatcher → manifests →
hotfix files → marker block chain. Without the flag, `girpr` makes **no
dispatcher request, no persistent request, and no marker write**, and its plan,
`SUMMARY` and exit codes stay byte-identical to today's.

Two exceptions are deliberate and run-wide, documented in the README and in
`docs/08` §5 (C1/C2/C12, B5):

1. the purge keep-set gains the marker names and the root `pkg_version` for
   **all** runs, because leaving the default path destructive to an install's
   verification state is not an acceptable trade for a stable byte total;
2. the new `SUMMARY`/`PLAN` keys are conditional on the flag.

### 1.2 What the flag must buy

The game trusts a local install only when **both halves** of the verification
state are present and consistent (`docs/08` §1):

- the *base* marker `Persistent/base_res_version_hash` = lowercase-hex MD5 of
  `StreamingAssets/res_versions_streaming`;
- the *hotfix* database mirrors `Persistent/res_versions_persist`,
  `Persistent/data_versions_persist` and `Persistent/silence_data_versions_persist`,
  byte-for-byte from the dispatcher;
- the *revision block* (`res_revision`, `res_revision_eternal`, `data_revision`,
  `silence_revision`, `audio_revision`, `ChannelName`, `ScriptVersion`,
  `PatchDone`) matching the dispatcher's current numbers.

A run that installs hotfixes but omits any part of this block has made the
install **worse**: the game re-verifies and re-downloads. This is why the flag
is one master switch, why markers are written last, and why a failed file
suppresses the whole block (C8).

### 1.3 What is explicitly out of scope

No Sophon path for hotfix bytes: the live probe (`docs/08` §4.5,
`tools/persistent-sophon-research/README.md` §3) found zero hotfix MD5s in any
of the 15 advertised chunk manifests or 5 patch manifests, and the dispatcher
CDN answers full-file GETs with the manifest MD5 and supports `Range`. The
existing chunk writer is **not** extended; persistent files use a new
full-file HTTP path (C10). `DownloadPref`, `audio_versions*`, `*versions_remote*`
deletions, `VideoAssets/*.tmp` deletions and SDK/WPF/plugin extraction are out
of scope (B6); only the extra *keep* patterns are adopted. The
`silence_data_versions_persist` mirror **is** in scope, following
GenshinSlimmer and GI-Live-Hotfix-Downloader (`docs/08` §2.2). Resumable `Range`
downloads are an optional later improvement, not part of this plan.

---

## 2. Commit 1 — research gate: B4 channel coverage and fixture capture

**Do this before the decoder is finalised.** The live probe covered
`hk4e_global` only; B4's answer is "try all three at launch, fall back to
global-only if CN/Bili cannot be verified". The gate decides which answer ships.

### 2.1 Steps

1. Extend `tools/persistent-sophon-research/fetch_metadata.py` to take a
   `--biz` and select the per-region Collapse metadata file (global is
   `config_GIGlb.json` under `metadata/v3/stable/`; the CN and Bili file names
   are resolved from Collapse's preset/release table during this step, not
   guessed). Record the unwrapped `GameDispatchURL`, `ProtoDispatchKey`,
   `DispatcherKeyBitLength`, `DispatcherKey` fingerprint and region id per biz
   in `samples/metadata_summary.json`.
2. Extend `dispatch.py` with the per-biz subdomain/channel tuple. The global
   shape is pinned at
   `https://osusadispatch.yuanshen.com/query_cur_region?version=OSRELWin7.2.0&lang=1&platform=3&binary=1&time=260&channel_id=1&sub_channel_id=1&account_type=1&dispatchSeed=<seed>&key_id=5`;
   Collapse's subdomains are `osusadispatch` / `oseurodispatch` /
   `osasiadispatch` / `oschtdispatch` (global) and `cngfdispatch` /
   `cnqddispatch` (CN/Bili), with channel `OSRELWin` for global and `CNRELWin` for region id
   ≥ 4. Determine which subdomain/channel each of `hk4e_cn` and
   `hk4e_bilibili` actually uses; do not infer from the HYP channel tuple.
3. For each biz, dump all protobuf fields generically (as the global probe
   already does) and fetch all three persistent manifests, checking their MD5 and
   size against the dispatcher's own description.
4. Commit the **trimmed** samples the tests need: `dispatch_summary.json`,
   `dispatch_fields.txt`, `res_versions_external.full.head.ndjson`,
   `data_versions.full.head.ndjson`, `*.names.tsv`, and one hotfix row per biz.
   Keep raw ciphertext and full manifests out of the tree (the tool's
   `.gitignore` already does).
5. Record the outcome in `tools/persistent-sophon-research/CHANNELS.md`:
   per-biz subdomain, channel, seed, key fingerprint, res/data revision,
   manifest URLs and MD5s, and the go/no-go verdict.

### 2.2 Acceptance criteria / fallback

A biz is **verified** when its ciphertext decrypts with the unwrapped key, the
protobuf carries the four revision fields plus both repo URLs and a version
field equal to the `meta.latest` used in the query, all three manifests MD5-match,
and the NDJSON schema is the same set of recognised keys (extra keys such as
`isPatch`, `category`, `localName` are tolerated).

- **All three verified** → ship B4(b): the flag works for every `--biz`.
- **CN and/or Bili fail** → ship B4(a): under the flag, `--biz hk4e_cn` and
  `hk4e_bilibili` are a usage error (exit `1`) with a message naming the
  unverified channel; global works. The decoder is still built channel-generic
  so adding a channel later is a data-table change.

This gate produces the per-biz constants that §5.1.1's `DispatchKeys` provider
and §5.5's subdomain selection consume. It is a commit of scripts, samples and
docs — no product code.

---

## 3. Architecture and crate layout

### 3.1 New crates

| crate | owns | why it is separate |
|---|---|---|
| `app/dispatch/` (`girpr_dispatch`) | unwrap Collapse release metadata into key/seed, dispatcher URL building, RSA PKCS#1 v1.5 decrypt, protobuf decode, typed `DispatchInfo` | new dependency tree (`rsa`, `brotli`, `base64`, `prost`): an edit to repair, report or CLI leaves this cached, same reason `girpr-fetch` exists |
| `app/persistent-plan/` (`girpr_persistent_plan`) | value types and **pure** decisions: row normalisation, suffix→folder placement, force-persistent rule, CDN URL selection, marker rendering, scope suppression | plan-half decisions must be inspectable and unit-testable with no I/O; depends only on path/digest leaves |

### 3.2 Extended crates

| crate/module | addition |
|---|---|
| `app/fetch/src/persistent.rs` | fetch + MD5-verify + NDJSON-parse all three manifests into `PersistentRow`s; full-file HTTP download (primary → alternate on 404), temp file, digest gate, atomic publish |
| `app/repair-plan/` | carry a sibling `PersistentPlan` alongside `RepairPlan`; add persistent bytes to `DownloadEstimate`/`PlanSummaryValues`; feed dispatcher paths into `PurgeKeys` |
| `crates/model/src/region.rs` | const per-biz dispatcher subdomain list and channel prefix (`Region::dispatch_subdomains()`, channel accessor), with the existing constant-pinning tests extended. Values only: URL/request building stays in `girpr_dispatch` (the leaf's "values, not behaviour" rule) |
| `src/repair/phases/metadata.rs` | `Meta` gains `dispatch: Option<DispatchInfo>`; `meta.latest` is only known after `metadata.rs:307`, so the query runs at that point or in `fetch_and_report` |
| `app/report/` | conditional `SummaryFlags` fields and `PLAN` totals for persistent work |
| `src/cli.rs`, `src/config.rs` | `--with-persistent` → `RunCtx.with_persistent` |
| `src/repair/mod.rs`, `src/repair/phases/{metadata,post}.rs`, `src/repair/file/mod.rs` | phase wiring; `src/repair/purge.rs` keep rules |
| `app/test-fixtures/`, `tools/capture/`, `tests/artifact_bridge.rs` | mock routes, bundle records, bridge cases |

### 3.3 Dependency direction (no cycles)

```
girpr_dispatch ──┐
girpr_persistent_plan ──┐
girpr_fetch (+ persistent.rs) ──┤
girpr_repair_plan ─────────────┤──> girpr (facade) ──> binary
girpr_purge (facade) ──────────┘
```

`app/repair-plan` already depends on `girpr-fetch`; therefore persistent value
types go in a **new leaf** (`app/persistent-plan/`) rather than in either
crate — putting them in `girpr-fetch` would make fetch depend on repair-plan or
vice versa. `girpr_dispatch` depends on `girpr-model` for the `--biz` values and
on `girpr-digest` for hex/digest spelling, but **not** on `girpr-fetch` (its
transport is its own: metadata calls are plain GETs, the dispatcher response is
an encrypted binary body).

### 3.4 New dependencies (pin deliberately)

- `rsa` — PKCS#1 v1.5 private-key decryption (metadata unwrap and dispatcher
  response). No product `unsafe`; the workspace lint stays `deny`. Two key
  sizes are in play: the metadata master key is RSA-1024 (`BitSize = 128`-
  byte blocks) and the dispatcher key is RSA-2048 (`DispatcherKeyBitLength =
  256` **bytes**, despite the name). The .NET XML key must be loaded as
  `(n, e, d, p, q)` — `InverseQ` is `q^-1 mod p` and must not be passed as a
  seventh component.
- `brotli` — Collapse metadata blob decompression. The "Collapse-magic" blob
  is a 32-byte little-endian header: signature `7310310183885631299`, flags
  (`compression = flags & 0xFF`, `encrypted = (flags >> 8) & 0xFF`),
  compressed size, decompressed size, then the payload. `config_master.json`
  holds a flags-`0x1` (Brotli only) blob whose payload is the DER RSA-1024
  private key; `config_GIGlb.json` (and the per-region files from §2) hold
  flags-`0x101` blobs (Brotli + RSA in `BitSize` blocks). Verify both sizes
  after decrypt/decompress.
- `base64` — the dispatcher `content` field (23 × 256-byte RSA blocks → 5575-
  byte protobuf) and the metadata blobs.
- `prost` — already in `girpr-fetch`'s tree; used for the gateway protobuf in
  `app/dispatch/`. A small generic field walk is needed for the fields the
  checked-in `.proto` omits (root 14, properties 38–47); those are logged as
  opaque, never used to place files. The fields we consume are root `3` →
  properties `22` → `1`/`5` (res revision/hash), properties `14`/`26` (data),
  `18`/`27` (silence), `8`/`9` (repo base URLs), `22` → `3` (res manifest
  descriptor NDJSON), `19` (data descriptor NDJSON), `20` (silence descriptor
  NDJSON), `12` (version), `7` (server) and `36` (codename).
- `twox-hash` — XXH64 verification when a row carries `hash` (Collapse prefers
  it over `md5`); MD5 stays the fallback via `girpr-util-hash`.

`Cargo.toml` workspace members and `Cargo.lock` are updated in the commit that
introduces each crate.

---

## 4. Data model and pure decisions

### 4.1 Types (in `app/persistent-plan/`)

```rust
struct DispatchInfo {
    game_version: String,        // root 3 -> 12; == meta.latest (B9), else exit 2
    channel_name: String,        // OSRELWin (region < 4) | CNRELWin (region >= 4)
    server_name: String, codename: String,   // root 3 -> 7 / 36; subdomain check
    res_revision: u64, data_revision: u64,
    silence_revision: u64, audio_revision: u64,
    res_hash: String, data_hash: String,
    silence_hash: String, audio_hash: String, // audio from base_revision GET
    client_game_res_url: String, client_design_data_url: String,
    client_silence_data_url: String,
    client_audio_assets_url: String,
    manifest_digests: [ManifestDescriptor; 3],
}

struct ManifestDescriptor {          // one per mirrored manifest
    kind: Res | Data | Silence,
    url: String,
    expected_md5: String,            // from the descriptor NDJSON, not hardcoded
    expected_size: u64,
    mirror_rel: String,              // <exe>_Data/Persistent/{res,data,silence_data}_versions_persist
}

struct PersistentRow {               // one NDJSON line
    remote_name: String,
    md5: String,
    hash: Option<u64>,               // XXH64 from the `hash` hex field, when present
    file_size: u64,
    // `isPatch`, `category`, `localName` are parsed-but-ignored keys.
}

struct PersistentAsset {
    rel: String,                     // cleaned, relative to --game-path
    placement: Persistent | StreamingAssets,
    size: u64, md5: String, hash: Option<u64>,
    primary_url: String, alternate_url: String,
    source: Res | Data | Silence,
}

struct PersistentPlan {
    assets: Vec<PersistentAsset>,    // sorted, deduped by rel
    mirrors: Vec<(String, Vec<u8>)>, // raw manifest bytes + destination rel
    marker_paths: Vec<String>,       // cleaned rels of every file the post phase writes
    expected_rels: Vec<String>,      // assets + mirrors + marker paths, for the purge
    total_bytes: u64,
}

struct MarkerSet { path: String, value: String }  // plain text, no trailing newline
```

### 4.2 Placement (port of Collapse `Fetch.Persistent.cs`)

For each parsed row, map `remoteName` to a folder by suffix, then choose the
base tree:

| `remoteName` suffix | folder |
|---|---|
| `*.pck` | `AudioAssets/` |
| `remoteName` **contains** `blocks` **and** ends with `.blk`, or ends with `svc_catalog` | `AssetBundles/` |
| `*.usm`, `*.cuepoint` | `VideoAssets/` |
| anything else | `""` (install root within the base tree) |

These are Collapse's exact tests (`Fetch.Persistent.cs:239–278`, case-insensitive):
note `Contains("blocks")`, **not** `StartsWith` — a different mapper elsewhere in
the same file uses `StartsWith`; do not conflate them. Collapse's second mapper
(`Fetch.cs:214–248`) also treats `StartsWith("blocks") || EndsWith(".blk")` as
`isPersistentAsStreaming`; that mapper is not the one that places dispatcher
rows, so it is not the rule to port here.

`rel = <exe>_Data/<base>/<folder>/<remoteName>` where `<base>` is
`StreamingAssets` by default and `Persistent` when the row is a hotfix.

**Force-persistent rule (the semantic core).** Build a map of the *unfiltered*
Sophon base manifest: `clean_rel → md5`. If a dispatcher row's `rel` already
appears there with a **different MD5** (case-insensitive compare), it is a
hotfix: `placement = Persistent`. If the MD5 is equal, the base plan already
covers those bytes — omit the row from the persistent download set (girpr
saves the HTTP traffic Collapse spends there). If the `rel` is absent from the
base map, keep the row with its suffix-derived placement; these are the
"new path" rows (audio copies at a new location, etc.) and the game needs them
even though no Sophon entry names them. Server-sent `isPatch` is parsed and
**ignored** for placement (Collapse parity, research §4.5).

**URL assembly** uses the *original* `remoteName` before any rel rewriting:

| source | non-hotfix (base) primary/alternate | hotfix primary/alternate |
|---|---|---|
| `res_versions_external` | `audio-assets/StandaloneWindows64`, `game-res/StandaloneWindows64` | `game-res/StandaloneWindows64`, `audio-assets/StandaloneWindows64` |
| `data_versions` | `design-data/…`, `design-data/…` | `design-data/…`, `design-data/…` |
| `data_versions` (silence) | `silence-data/…/AssetBundles`, `silence-data/…/AssetBundles` | `silence-data/…/AssetBundles`, `silence-data/…/AssetBundles` |

where the four prefixes are `client_audio_assets_url`, `client_game_res_url`,
`client_design_data_url` and `client_silence_data_url` from `DispatchInfo`, and
the final URL is `{prefix}/{folder}/{remoteName}`. Silence rows carry the same
`*.blk` / `svc_catalog` placement rule; the CDN prefix differs only in
`client_silence/General` instead of `client/General`. The alternate URL is
required (research §4.5); every download attempts primary then alternate on a
404 only, never on a digest failure.

**Digest gate.** If `hash` is present, verify XXH64 of the downloaded bytes
against it; otherwise MD5. Re-verify after the atomic publish (a second read)
before the file counts as done — Collapse's HTTP path has no post-download
check, which is a known weakness we do not copy.

### 4.3 Marker bytes (port of Collapse `Fetch.cs:252–293`)

All values are plain text with **no trailing newline**, published with
`app/util-path::write_atomic`:

| File (under `<exe>_Data/Persistent/`) | Value |
|---|---|
| `base_res_version_hash` | lowercase-hex MD5 of `<exe>_Data/StreamingAssets/res_versions_streaming`; **omit + report** if the source is absent (C16) |
| `res_revision` | `res_revision` |
| `res_revision_eternal` | `res_revision` (HoYo's own typo for "external") |
| `data_revision` | `data_revision` |
| `silence_revision` | `silence_revision` |
| `audio_revision` | `audio_revision` |
| `PatchDone` | same value as `audio_revision` |
| `ChannelName` | `channel_name` |
| `ScriptVersion` | `game_version` (= `meta.latest`, B9) |

`DownloadPref` is not reproducible from public data and is not a verification
flag: skip it. The three mirror files are the raw, MD5-verified manifest bytes.

**Confidence tiers — why every line stays in the block.** The game's own string
table names `res_revision`, `data_revision`, `silence_revision`, `ChannelName`,
`ScriptVersion`, `base_res_version_hash`, the `*_versions_persist` manifests and
the `*versions_remote*` / `cache_versions*` families; a startup `CreateFileW` log
shows it opens `base_res_version_hash`, and the game log
`IsBaseResVersionHashChanged => … / Package has changed!!!!! Clear state files in
persist dir!!!!!` proves a mismatch **wipes** the Persistent state. GenshinSlimmer
issue #19 shows the client checks `res_revision_eternal` *alongside* `res_revision`,
so writing both from one dispatcher value is load-bearing, not redundant, even
though the older string-table dump does not name the "eternal" twin.
`audio_revision` and `PatchDone` are Collapse parity with no documented reader;
`DownloadPref` is launcher metadata. None of those three is treated as a
verification flag, but writing the two harmless revision files is pinned by the
byte tests.

### 4.4 Version authority (B9(a))

Query `query_cur_region` with `meta.latest`, **not** the pre-repair local
version. The query's `version` parameter is `{channel}{meta.latest}` — e.g.
`OSRELWin7.2.0` — with the fixed parameters
`lang=1&platform=3&binary=1&time=260&channel_id=1&sub_channel_id=1&account_type=1&key_id=5`
and the versioned `dispatchSeed`. If the response's own version field
(protobuf root `3` → `12`) disagrees with `meta.latest`, raise
`RunFailure::metadata` (exit `2`) with a distinct log line — never publish a
revision that describes a different byte set. Also check the codename (root
`3` → `36`) against the selected biz. This also means a fresh install (no
local version) works under the flag. The key/seed provider must cover
`meta.latest`; a key/seed missing for that version is the same exit-2 metadata
failure, not a silent fallback to an older seed (the GenshinSlimmer table has
version-specific seeds such as `018bc9560abc707f` for `7.2.0` and
`d9f572e5464f160f` for `7.0.0`).

---

## 5. Phase integration

All sites below are in the current `src/repair/mod.rs` pipeline:
`plan_phase` (`184–274`) → `fetch_and_report` (`298–318`) →
`fetch_plan_network` (`332–394`) → `build_and_assert_plan` (`421–530`) →
`publish_plan` (`768–813`) → `execute_phase` (`822–915`) →
`run_repair` (`1140–1160`) / `run_post` (`1167–1188`).

### 5.1 Plan half — decide, touch nothing

1. **Dispatcher query, in `fetch_and_report` after `metadata::fetch`.**
   Only when `ctx.with_persistent && !ctx.purge_only` (C20: a cleanup-only run
   does no persistent work at all; `--print-plan` and `--check-only` do need the
   query so they can list/check the persistent set). Load `DispatchKeys` (B2(a): fetch and unwrap
   Collapse's release metadata from
   `metadata/v3/stable/config_master.json` + the per-region config file, pinned
   URLs + digest check on the key blob; B2(b) embedded versioned table only as
   an explicit, non-user-facing fallback), choose the subdomain (§5.5), build
   the URL with `meta.latest` (B9), decrypt, decode, and check the version and
   codename fields. Then derive the four CDN prefixes:
   `{resBase}/output_{resNum}_{resHash}/client`,
   `{silenceBase}/output_{silNum}_{silHash}/client_silence/General` (the
   dispatcher's `ClientDesignDataSilURL`, built as
   `{RepoDesignDataURL}/output_{RepoDesignDataSilenceNumber}_{RepoDesignDataSilenceHash}/client_silence/General`),
   `{dataBase}/output_{dataNum}_{dataHash}/client/General`, and — after a
   second GET of `{gameResPrefix}/StandaloneWindows64/base_revision` whose
   body is `"<revision> <hash>"` — `{resBase}/output_{audioRev}_{audioHash}/client`.
   On any key/seed, decrypt, decode, `base_revision`, version or codename
   failure: `RunFailure::metadata` with the log line `dispatcher query failed: …`
   and exit `2` (C6, B3). An unflagged run never calls this provider. Cache
   nothing (Collapse's 5-minute cache is not needed for a single-shot run).

2. **Persistent manifests, in `fetch_plan_network`** (same `with_persistent &&
   !purge_only` gate). The dispatcher's embedded descriptor NDJSON (root `3` → `22` → `3`, root `3` → `19` and root `3` → `20`)
   supplies each full manifest's `remoteName`, `md5` and `fileSize`; derive the
   three URLs from those names, not from hardcoded paths. Fetch all three
   concurrently with the existing manifest/reuse/deprecated trio under the
   same client and `ManifestBudget`, but as a **required** branch: a failed
   manifest is exit `2`, not a degraded report. Verify each body's MD5 and
   size against its descriptor before parsing (the live probe did exactly
   this for 3,164 rows); parse NDJSON skipping blank lines and tolerating
   `isPatch`/`category`/`localName` keys (the research notes a blank line
   aborts Collapse's parser). Keep the raw bytes in memory only (C3: the plan
   half stays read-only). The silence descriptor (root `3` → `20`) is a
   **required third branch**, not a shape check: fetch its full manifest from
   `{silenceBase}/…/client_silence/General/AssetBundles/data_versions`, verify
   MD5 + size against the descriptor, parse it into `PersistentRow`s and mirror
   it to `Persistent/silence_data_versions_persist` (`docs/08` §2.2). A failure
   is exit `2`, exactly like the other two.

3. **Build `PersistentPlan` in `build_and_assert_plan`.** Pure functions in
   `app/persistent-plan`: normalise rows, place them, apply the force rule
   against the *pre-scope* Sophon map, assemble URLs, dedupe by cleaned rel,
   render the marker paths and expected rels. Apply `ctx.path_scope` to the
   asset list (the same "filtered-out paths do not exist" rule as the base
   plan). Dispatcher hotfix paths are added to `PurgeKeys`' expected set
   alongside `plan.expected_rels()`.

4. **Scope guards (C7(d)+(f)).** With the flag on and an active scope:
   - if any *marker file* the run would write is outside the scope →
     `RunFailure::usage` (exit `1`), with the same wording style as the
     `--audio` scan-file guard (`metadata.rs:465–479`); this is the only new
     flag-combination error;
   - otherwise, if any dispatcher hotfix path in the current persistent set is
     out of scope → still repair the in-scope files, but **suppress the marker
     block and all three mirrors** and report `persistent=suppressed`. The revision
     value would otherwise assert that a complete revision is installed.
     This mirrors the scoped `config.ini` rule (`config_bumped=0`).
   - Neither guard applies to unflagged runs.

5. **Ceilings (C11, B7).** Add persistent sizes to the pre-flight byte totals
   (`DownloadEstimate`, `Limits::check_preflight`) and to the live counters.
   `--max-download-bytes`, `--max-write-bytes`, and the two `--max-verified-*`
   ceilings bound persistent bytes exactly as they bound base bytes; a
   `--max-download-bytes 0` run under the flag refuses before any write.
   `--max-files` also counts persistent files (a file-count ceiling that a new
   code path can bypass is broken). `--scan` still reads only base files: the
   persistent digest gate already reads each one in full and the manifest
   gives exact sizes (C13). Make that exclusion visible in `PLAN`/`--plan-out`.

6. **Publish (C13, C4).** `PLAN` gains flag-only `persistent_files=` and
   `persistent_bytes=`; `--plan-out` gains a `persistent` section. In
   `--check-only`, `--dry-run` and `--print-plan`, no mirror or marker is
   written — the persistent work is listed only.

### 5.2 Execute half — put the bytes in place

Insert a new persistent repair path next to `file::repair_all` in
`execute_phase`, before the post phase. Order:

1. **Directories and cheap prerequisites (gap rows 9–11).** After
   `prepare_install` but before repair: create `<exe>_Data/Persistent` and
   `<exe>_Data/StreamingAssets`; delete stale `Persistent/ctable.dat`; delete
   `DXSETUP/` (false-positive integrity errors); copy
   `StreamingAssets/ctable.dat` → `ctable_streaming.dat`. All under the flag,
   never in read-only modes. The exe stem comes from
   `meta.config.exe_file_name`, not a hardcoded string.
2. **Move non-patch `.pck`/`.usm`/`.cuepoint` from `Persistent/` back to
   `StreamingAssets/`** (Collapse `Check.cs:91–145`), so a hotfix that ended
   does not leave a stranded duplicate. Never in `--check-only`.
3. **Read-only flags (C17(b)).** Clear the read-only attribute on a persistent
   target immediately before overwriting it — tree-wide stripping would
   mutate files the run does not otherwise touch. If the file still cannot be
   opened, fall back to the same temp path any other repair uses; do not fail
   the run for that case.
4. **Full-file download (C15, C10, gap row 7).** For each persistent asset:
   verify size + digest first and skip if intact; otherwise stream the body
   from `primary_url`, retry `alternate_url` on 404 only, write to a temp
   sibling, run the digest gate, then publish atomically with
   `write_atomic`. Reuse the shared reqwest client exposed by
   `HypClient::http()` and `girpr_retry`'s backoff; only `sophon::retryable`
   statuses (408/429/5xx) are retried on one URL, with the alternate tried on
   a 404, matching the launcher. Re-read and re-verify after publish. Every byte and every
   success/failure feeds the shared `Summary` counters and the live ceilings.
   There is no Sophon/chunk fallback for these paths.
5. **All-or-nothing gate (C8).** Any failed persistent file behaves like any
   failed base file: the run returns exit `3` with `SUMMARY` and **skips the
   post phase**, so no marker block or mirror is published. No partial
   revision claims (C19 still writes the *full* block on a clean run even when
   no file was broken).

### 5.3 Post phase — publish markers last

In `src/repair/phases/post.rs::run_inner`, insert one step after the
`purge_after_pass` block (`~118`) and **before** the `config.ini` bump (`~123`):

1. If a delete ceiling ended the phase (`PostOutcome::DeleteCeiling`), return
   first — no markers.
2. If `ctx.purge_only` (C20), skip all persistent work: no dispatcher query in
   the plan half, no mirror, no marker. The already-widened keep-set still
   protects existing markers.
3. If read-only (`--dry-run`), log one `would write marker …` line per file and
   publish nothing (C4).
4. If the C7(f) scope suppression fired, log the suppression and publish
   nothing, reporting `persistent=suppressed`.
5. Otherwise write the **full** marker block and all three mirrors atomically
   (C19), after every persistent file has passed its gate. Failures are fatal
   `RunFailure::write` (exit `3`), like the `config.ini` write.
6. Then the existing `config.ini` bump runs last (C9): markers first, version
   advance last, both skipped on failure. A scoped or purge-only run still
   shows `config_bumped=0`.

### 5.4 Check-only

With the flag, `phases::check::verify_plan` also verifies (a) every persistent
file's size + digest against its manifest row, (b) all three mirrors' bytes against
the dispatcher MD5, and (c) every marker file's presence and exact value from
`MarkerSet`. Any mismatch is damage → exit `4`. Without the flag, check-only
is unchanged. The help text and README must say that the flag narrows what
"intact" means (C5).

### 5.5 Subdomain selection

Collapse takes the region id from its preset and maps it
`0 → osusadispatch`, `1 → oseurodispatch`, `2 → osasiadispatch`,
`3 → oschtdispatch`, `4 → cngfdispatch`, `5 → cnqddispatch` (Bilibili),
with `OSRELWin` below 4 and `CNRELWin` from 4 up. `girpr` has no such preset:
use the ordered per-biz host list from the §2 gate, try each in order, and
accept the first response that decrypts to the expected game version and
codename for the biz. Log the chosen subdomain. If none works, exit `2` with
the list of attempted hosts. This is unattended and needs no local
`info.txt` read (GenshinSlimmer's fallback at
`%USERPROFILE%\AppData\LocalLow\miHoYo\Genshin Impact\info.txt`); that file
remains a last-resort diagnostic if the gate shows a biz where the ordered
list is insufficient.

---

## 6. User-facing contracts

### 6.1 CLI and exit codes

| Flag | Default | Meaning |
|---|---|---|
| `--with-persistent` | off | Query the dispatcher, update `Persistent/`, and publish the verification markers. Requires `--biz` coverage per §2. No effect on the default repair. |

Exit codes stay the six already pinned in `src/config.rs` (`define_exit_table!`);
no new variant:

- `1` — usage: `--with-persistent` with an unverified biz (B4(a) fallback), or
  a marker file outside an active scope (C7(d)).
- `2` — metadata/network: metadata unwrap, dispatcher query/decrypt/decode,
  version disagreement (B9), or a persistent manifest fetch/MD5 failure (C6, B3).
- `3` — write: any failed persistent file (`SUMMARY` with `failed>0`, post
  skipped), an atomic marker publish failure, or a lock/mkdir failure.
- `4` — check-only found persistent file, mirror or marker damage (C5).
- `5` — a ceiling bound, including the new persistent bytes (C11).

### 6.2 Record stream

Every new key is emitted **only** when `--with-persistent` was passed, so an
unflagged line is byte-identical. `SummaryFlags` gains:

| key | type | value |
|---|---|---|
| `persistent` | word | `published` · `suppressed` · `listed` (read-only) · `skipped` (`--purge-only`) · `failed` |
| `persistent_files` | number | planned persistent assets |
| `persistent_repaired` | number | assets that passed their digest gate this run |
| `persistent_failed` | number | assets that failed |
| `persistent_base_hash` | word | `omitted` only when `res_versions_streaming` is absent (C16) |

`download_bytes`, `write_bytes` and their `*_spent`/verified variants keep
their meaning but now include persistent bytes under the flag (C11, B7). The
new keys are appended in `app/report/src/summary.rs::conditional_extras` next
to `local_manifest` (last), and the golden key vectors in
`app/report/src/tests.rs` and `tests/cli_contract.rs` are updated in the same
commit. The `PLAN` line and `--plan-out` document gain flag-only persistent
totals (C13).

### 6.3 Docs

README: flag table row, the opt-in contract paragraph, check-only's narrowed
meaning, the ceiling broadening, and the one-time keep-set-driven
`deleted_extra_bytes` change (C12/B5). `docs/08` status line is updated to
"implementation planned in `docs/09`" only after the feature ships (final
commit).

---

## 7. Testing strategy

All new tests are offline by construction; no test scans a directory it does
not own (`README.md` "For developers / AI agents"). Scratch trees go under
`<temp>/girpr-test/`.

| Layer | Location | What it pins |
|---|---|---|
| Unit — decoder | `app/dispatch/` tests | metadata blob unwrap against recorded samples; proto field decode against `samples/dispatch_fields.txt`; version-match rule; key/seed version coverage; URL/subdomain builders; per-biz channel constants |
| Unit — pure plan | `app/persistent-plan/` tests | suffix→folder table; force-persistent MD5 diff (same/different/missing base); URL primary/alternate swap; digest preference hash-vs-md5; blank-line and extra-key tolerance; marker byte rendering (no trailing newline, `PatchDone` = audio revision, hash omission); scope suppression predicate |
| Unit — purge | `src/repair/purge.rs` table test `cached_keys_classify_every_keep_rule` | new keep patterns for `*_persist`, `*_revision`, `ChannelName`, `ScriptVersion`, `PatchDone`, `base_res_version_hash`, `cache_versions_*`, `ctable*`, root `pkg_version`; negative rows proving `*versions_remote*` and `VideoAssets/*.tmp` still purge |
| Integration | `app/test-fixtures` mock + `tests/repair_offline.rs` (pulled into the `tests/repair.rs` root) | dispatcher failure → 2; version disagreement → 2; manifest MD5 failure → 2; three-mirror publish (res/data/silence), including silence CDN fallback; full hotfix download with primary 404 → alternate; corrupt body → retry/gate failure → 3 with no markers; read-only flags; `ctable`/`DXSETUP` prerequisites; read-only modes list-only; `--purge-only` skip; scoped run with marker path out of scope → 1; scoped run with hotfix path out of scope → suppressed; pre-flight and runtime ceilings including persistent bytes; check-only marker damage → 4 |
| Binary contract | `tests/cli_contract.rs` (in the `tests/contracts.rs` root) | flag parse, help text, exit codes, conditional `SUMMARY` keys absent unflagged and present in order under the flag |
| Bridge (`#[ignore]`d) | `tests/artifact_bridge.rs` + `app/test-fixtures/src/artifact_server.rs` | a captured dispatcher response + all three manifests + a few full-file blobs are replayed; assert hotfix bytes, mirror bytes and marker bytes; assert no marker writes under `--dry-run` and `--purge-only` |
| Manual, real install | recorded in `docs/08` §6 step 5 | update Persistent with the flag, copy the game to a second machine, launch once, confirm no verification/re-download |

**Fixtures — offline mock.** The mock in `app/test-fixtures` gains a dispatcher
route whose body is a deterministically encrypted sample (fixed test RSA key +
seed committed as test data — never the production key), a key/seed provider
seam so tests do not touch Collapse's release repo, three manifest routes with
known MD5s, and a hotfix CDN route that can be scripted for 404-fallback and
corrupt-body cases. In `src/repair/phases/metadata.rs`'s own tests the new
dispatcher route follows the existing `Reply`/`TestServer` seam
(`src/test_origin.rs`), alongside the `getGameConfigs`/`getGameBranches`/
`getBuild` switch.

**Fixtures — gircap bundle and bridge.** Extend the bundle schema
(`tools/capture/docs/01-artifact-schema.md`, currently `schema_version: 3`)
with a `dispatch/` record (ciphertext + key fingerprint + seed + version) and
a `persistent/` directory (raw manifests + a handful of full-file blobs). The
new writers go through `emit::write_bytes`/`write_doc` so `MANIFEST.json →
files[]` and `verify/checksums.json` pick them up automatically
(`tools/capture/src/emit/{mod,tree,index}.rs`); extend the captured-body arrays
in `tools/capture/src/hyp/mod.rs` (`ValidatedCapture::bodies()`) and add the
records to `tools/capture/src/verify/{checks,list}.rs` so `gircap verify` and
`gircap list` stay honest. Add a `persistent` matrix class in
`tools/capture/src/select/classes.rs` (`Class::ALL`, `REQUIRED_ORDER`, `id()`
— all length-pinned), and a bridge case through the existing `capture_test!`
macro / `drive_named` in `tests/artifact_bridge.rs` with the record reader in
`tests/artifact_bridge/bundle.rs`.

On the replay side (`app/test-fixtures/src/artifact_server.rs`) add
`HitKind`/`Route` variants (`Dispatch`, `PersistentManifest`, `Hotfix`),
`Bundle` accessors for the new directories, and new entries in `Origins`;
serve the new endpoints from `hyp_response`-style handlers and rewrite their
origins in memory at `Bridge::start` bind time with `swap_authority`, exactly
as Sophon prefixes are rewritten today — never writing the bundle, so
`gircap verify` still passes afterwards. The existing real fixture under
`artifacts/hk4e_global/7.1.0__7.2.0/` is the model to extend; a new bundle is
captured by hand, as `docs/06` §3 already requires.

---

## 8. Commit sequence

Each commit builds and passes its own tests. The feature is not user-visible
until C8; no release should be cut between C6 and C8.

| # | Commit title | Touch | Verification |
|---|---|---|---|
| 1 | `docs: add persistent update implementation plan` | this file | doc only |
| 2 | `test(persistent): capture dispatcher + manifest fixtures for all channels` | `tools/persistent-sophon-research/`, `CHANNELS.md`, trimmed samples | scripts re-run; MD5 checks logged; B4 verdict recorded |
| 3 | `fix(purge): keep verification markers and root pkg_version` | `src/repair/purge.rs`, purge tests, README note | `cargo nextest run`; unflagged `SUMMARY` change documented (C12) |
| 4 | `feat(dispatch): add dispatcher metadata/query decoder crate` | `app/dispatch/`, workspace, `Cargo.lock` | unit tests against committed samples; no product wiring |
| 5 | `feat(persistent): add persistent plan values and full-file fetch` | `app/persistent-plan/`, `app/fetch/src/persistent.rs`, workspace | unit tests for every pure rule and the download gate; no product wiring |
| 6 | `feat(persistent): wire the plan half behind --with-persistent` | `src/cli.rs`, `src/config.rs`, `app/report/`, `app/repair-plan/`, `src/repair/mod.rs`, `src/repair/phases/metadata.rs`, tests | dispatcher/manifest/plan/scope/ceiling cases; `PLAN`/`SUMMARY` keys; flag-off byte identity |
| 7 | `feat(persistent): repair persistent files in the execute half` | `src/repair/mod.rs`, new persistent repair module, mock routes, tests | download/fallback/gate/prereq/read-only cases; failure → 3 skips post |
| 8 | `feat(persistent): publish markers last and check them` | `src/repair/phases/post.rs`, `src/repair/phases/check.rs`, tests | marker byte pins (incl. `res_revision_eternal`), three-mirror publish (res/data/silence), C19/C20/C16/C5 cases, `config.ini` ordering |
| 9 | `test(capture): extend gircap bundle and bridge for persistent` | `tools/capture/`, `app/test-fixtures/`, `tests/artifact_bridge.rs`, `docs/06` note | `gircap verify`; bridge cases green against a captured bundle |
| 10 | `docs(persistent): update README and research status; record real-install verification` | `README.md`, `docs/08` | doc review; recorded manual verification result |

Where a commit's tests need a fixture the earlier fixture commit did not
provide, add that fixture in the same commit as its consumer (the bridge's
synthetic mini-bundle follows this rule today, `docs/06` §1.2).

---

## 9. Traceability

### 9.1 `docs/08` §4 gap rows

| row | implemented by |
|---|---|
| 1 dispatcher query | §5.1.1, commit 4/6 |
| 2 persistent manifests | §5.1.2, commit 5/6 |
| 3 mirror to `*_persist` | §5.3, commit 8 |
| 4 revision block | §4.3, §5.3, commit 8 |
| 5 `base_res_version_hash` | §4.3 (C16), commit 8 |
| 6 placement + force rule | §4.2, commit 5 |
| 7 full-file downloader | §5.2.4, commit 5/7 |
| 8 read-only stripping | §5.2.3 (C17(b)), commit 7 |
| 9 create both trees | §5.2.1, commit 7 |
| 10 ctable / DXSETUP | §5.2.1, commit 7 |
| 11 move non-patch AV back | §5.2.2, commit 7 |
| 12 audio selection | reuse `meta.audio` + scan-file read; verify path equality in commit 6 (C14) |
| 13 config.ini bump | unchanged, ordering pinned in §5.3 (C9) |
| 14 extra-file purge | commit 3 + dispatcher paths in commit 6 (C1/C2) |
| 15 SDK/WPF/plugin keep | out of scope (B6); revisit only when widening keep inputs |
| 16 `pkg_version` rebuild | out of scope; root `pkg_version` kept by commit 3 (C2) |

### 9.2 `docs/08` §5 decisions

| id | answer | where enforced |
|---|---|---|
| C1 keep-set for all runs | (a) | commit 3, purge table test |
| C2 root `pkg_version` keep | (a) | commit 3, `keep_set()` + test |
| C3 mirror in execute/post | (a) | §5.1.2, §5.3 |
| C4 read-only modes write nothing | (a) | §5.1.6, §5.3, integration tests |
| C5 check-only includes markers | (a) | §5.4 |
| C6 dispatcher failure → 2 | (a) | §5.1.1, §6.1 |
| C7 marker scope (d) + path scope (f) | (d)+(f) | §5.1.4 |
| C8 all-or-nothing markers | (a) | §5.2.5 |
| C9 markers before bump | (a) | §5.3 |
| C10 mode group excludes persistent | (a) | §1.3, §5.2.4; documented in README |
| C11 fold bytes into ceilings | (a) | §5.1.5 |
| C12 keep-set fix changes unflagged total | (a) | commit 3 + README note |
| C13 scan excludes persistent, visible in plan | (b) | §5.1.5–6 |
| C14 reuse audio selection | (a) | §9.1 row 12 |
| C15 atomic publish + digest gate | (a) | §5.2.4 |
| C16 omit `base_res_version_hash` if source missing | (a) | §4.3, `persistent_base_hash=omitted` |
| C17 strip read-only on persistent paths only | (b) | §5.2.3 |
| C19 always write full block under flag | (a) | §5.3 |
| C20 purge-only skips persistent | (a) | §5.3, integration test |
| B1 one master flag | (a) | §1.1 |
| B2 key/seed from metadata, embedded fallback | (a)/(b) | §5.1.1 |
| B3 failure while opted in → 2 | (a) | §6.1 |
| B4 all three channels, fallback global-only | (b)→(a) | §2 gate |
| B5 ship keep-set fix independently | (a) | commit 3 |
| B6 Collapse parity + silence mirror + extra keep inputs | (a), extended to maintain `silence_data_versions_persist` from GenshinSlimmer / GI-Live-Hotfix-Downloader | §1.3, `docs/08` §2.2, §5.1.2, commits 3/6/8 |
| B7 fold bytes into existing ceilings | (a) | §5.1.5 |
| B8 legal/ToS posture accepted | (a) | §1.1, §5.1.1; project-owner call already recorded |
| B9 `meta.latest` authoritative | (a) | §4.4 |

---

## 10. Risks, implementation-time checks and non-goals

- **Third-party metadata dependency (B2/B8).** Runtime unwrapping of
  Collapse's release metadata is the chosen strategy, with a digest pin on the
  key blob so a silent upstream change is a loud exit `2`. The embedded
  versioned table is the fallback if that dependency is rejected; it must
  still fail closed on an uncovered `meta.latest`.
- **Seed churn.** The dispatcher seed is versioned and changes with game
  releases. The metadata provider must treat "seed/key not covering this
  version" as a metadata failure, not try a stale seed. The B4 gate's samples
  are a canary for format changes, as are the two pinned reference projects.
- **Exe stem and paths.** `<exe>_Data` comes from
  `meta.config.exe_file_name` (global `GenshinImpact`, CN/Bili differ); never
  hardcode a stem. Verify the two base trees exist before the plan is built.
- **Disk peak.** Full-file downloads use a temp sibling plus an atomic publish;
  the manifest gives exact sizes, so ceilings are exact and the peak is one
  file per concurrent task. `--repair-mode` is documented as not governing
  these bytes (C10).
- **Partial manifest writes.** Mirror the Collapse caveat: write to a temp
  file, verify, then publish; gate all marker writes on a successfully parsed
  manifest. Blank NDJSON lines are skipped, not fatal.
- **Post-download validation.** Re-read and re-verify after publish; do not
  copy Collapse's unchecked HTTP path.
- **Non-goals:** no live dispatcher query or manifest write in an unflagged
  run; no Sophon repair of hotfix bytes; no GenshinSlimmer deletions; no
  `DownloadPref`; no Range/resume in this release; no new exit code; no change
  to the base chunk-repair algorithm or to `docs/01`–`04`.

---

## 11. Appendix — exact constants to port

These are the strings and algorithms the reference projects and the live probe
agree on. Keep them in the decoder's unit tests so a format change fails a
test, not an install.

```text
Dispatcher URL template:
  https://{0}.yuanshen.com/query_cur_region?version={1}&lang=1&platform=3&binary=1
  &time=260&channel_id=1&sub_channel_id=1&account_type=1&dispatchSeed={2}&key_id=5
Subdomains (region id): 0 osusadispatch, 1 oseurodispatch, 2 osasiadispatch,
  3 oschtdispatch, 4 cngfdispatch, 5 cnqddispatch (Bilibili)
Channel prefix: OSRELWin (region < 4), CNRELWin (region >= 4)
Query `version` = channel prefix + meta.latest, e.g. OSRELWin7.2.0
Seeds seen: 018bc9560abc707f (7.2.0), d9f572e5464f160f (7.0.0)
Key/response: RSA-2048 PKCS#1 v1.5, 256-byte blocks, `content` = base64 of
  23 blocks; plaintext = protobuf. Metadata master key: RSA-1024, 128-byte blocks.
Metadata: .../metadata/v3/stable/config_master.json + config_GIGlb.json
  (per-region files resolved in the §2 gate)
Magic blob: signature 7310310183885631299, 32-byte LE header
  (signature, flags, compressed size, decompressed size), flags 0x1 / 0x101
Proto paths: 3→22→1/5 res revision/hash; 3→14/26 data; 3→18/27 silence;
  3→8/9 repo URLs; 3→22→3 / 3→19 / 3→20 descriptor NDJSON; 3→12 version;
  3→7 server; 3→36 codename
CDN URL shapes:
  {resBase}/output_{resNum}_{resHash}/client/StandaloneWindows64/res_versions_external
  {resBase}/output_{resNum}_{resHash}/client/StandaloneWindows64/base_revision
     (body: "<revision> <hash>", split on one space)
  {dataBase}/output_{dataNum}_{dataHash}/client/General/AssetBundles/data_versions
  {dataBase}/output_{silNum}_{silHash}/client_silence/General/AssetBundles/data_versions
  {resBase}/output_{audioRev}_{audioHash}/client/StandaloneWindows64/<folder>/<remoteName>
Mirrors: Persistent/res_versions_persist, Persistent/data_versions_persist,
  Persistent/silence_data_versions_persist
Placement: .pck → AudioAssets; contains("blocks") && .blk OR .svc_catalog →
  AssetBundles; .usm / .cuepoint → VideoAssets; else root
Force-persistent: base-path MD5 (case-insensitive) != dispatcher md5 → Persistent
Gate: XXH64 from `hash` (8 bytes) when present, else MD5 (16 bytes); size first
Marker block: base_res_version_hash, res_revision, res_revision_eternal,
  data_revision, silence_revision, audio_revision, ChannelName, ScriptVersion,
  PatchDone (= audio_revision); DownloadPref skipped; no trailing newline
  Confidence: res/data/silence_revision + ChannelName + ScriptVersion +
  base_res_version_hash + the three mirrors are game-read; res_revision_eternal
  is checked alongside res_revision (GenshinSlimmer issue #19); PatchDone has no
  known reader; DownloadPref is launcher metadata.
Prerequisites: create Persistent + StreamingAssets; delete Persistent/ctable.dat;
  delete DXSETUP/; copy StreamingAssets/ctable.dat → ctable_streaming.dat
```

The biggest dataset to pin in tests is the live probe's shape: 3,164 rows
(2,631 `res_versions_external` + 533 `data_versions`), 1,661 base copies that
the MD5 rule keeps out of `Persistent/`, and 376 hotfix-only MD5s — including
all 10 server-flagged `isPatch` rows. The live probe did **not** fetch the
silence manifest; its descriptor (root `3` → `20`) and full NDJSON must be
captured in the fixture commit (§2 step 3, commit 2) and its row count pinned
there. The trimmed samples under `tools/persistent-sophon-research/samples/`
are the fixture source; the full manifests and ciphertext stay out of the tree.
