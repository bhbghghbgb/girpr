# Repair spec - Rust implementation plan - `girpr` v1 (GI low-disk repair patcher)

Implements `docs/02-repair-chunk-spec.md`. Findings:
`docs/01-repair-findings-and-verdict.md`.

The `repair-` prefix on all three documents is load-bearing: this file, the spec
and the findings are one series describing the whole-install repair patcher.
[`docs/04-scope-limitation-spec.md`](04-scope-limitation-spec.md) is a separate
tier and does not amend any of the three.

## 1. Crate layout (single binary, std + tokio)

The binary is a thin shim; everything else is in the library. `src/repair/` is
one command split into phases — the coordinator (`mod.rs`) owns phase order and
the exit-code contract, the submodules own the work.

```text
Cargo.toml            # runtime: clap, chrono, tokio, reqwest, serde(+json), tracing(+subscriber, appender), anyhow, walkdir, futures-util, tempfile, plus the girpr-* path crates below; prost/zstd and the loopback mock are dev-only; no regex dependency
src/main.rs           # shim: Args::try_parse (arg errors -> exit 1) -> logging::init_tracing -> repair::run -> SUMMARY -> exit
src/lib.rs            # module map + re-exports
src/cli.rs            # clap Args, audio-lang normalize/validate, Args::into_ctx -> RunCtx
src/config.rs         # RunCtx (validated run config) + RunFailure (typed exit code) + RepairMode
src/logging.rs        # console layer (--log-level/RUST_LOG) + file layer (--log-file-level), over the shared policy
crates/log/           # the shared log policy: log file path, log levels, env vars (shared by girpr and the three dev tools)
crates/tempdir/       # the shared scratch-directory policy: a name nothing collides with, a tree removed with the test (also used by tools/)
app/test-fixtures/ # facade-free offline HYP/Sophon mock + fixture builders for the root integration tests, reached through thin shims under tests/common/
app/test-support/  # facade-coupled run glue (RunCtx construction, pipeline driver) shared by the root integration tests, reached through the thin tests/common/ shim
tools/sync/cache/    # girsync's redb hash cache, its postcard wire codec and digest vocabulary; consumed directly by girsync and sled2redb
tools/sync/leaves/    # girsync's acyclic rw limiter, include/exclude filter, link classifier and hardlink facts, consumed directly by girsync so a high-level edit leaves them cached
tools/sync/test-support/ # girsync-coupled option/cache/planner fixtures shared by girsync and sled2redb; keeps crates/tempdir/ girsync-free
app/report/src/    # Summary/Meter/StoppedBy counters + record data plane, REPORT / PROGRESS / SUMMARY emission, phase timer (plan_record lives in the scope reporter)
src/biz.rs            # re-exports girpr_model::region::Region as Biz; ids/channels/base URLs live in crates/model/
app/fetch/src/hyp.rs        # HYP client: getGameConfigs/getGameBranches/getBuild/getDeprecated
app/fetch/src/sophon/      # prost chunk-manifest structs, manifest fetch+verify+parse, the streamed chunk fetch, filtering, local chunk map
app/util-path/        # server-path containment, atomic publish, shared junction/symlink fixtures
app/util-hash/        # md5 helpers and the one-read file scan
app/util-ini/         # config.ini read/write, ignore/blacklist files
app/util-counter/     # the saturating atomic byte counter
src/repair/mod.rs     # phase coordinator + exit-code contract: run()
src/repair/phases/metadata.rs # local config.ini version, ignore/blacklist files, server metadata (step 1-2)
src/repair/phases/manifest.rs # select+fetch latest manifests, build the local chunk-reuse map (step 3)
app/repair-plan/src/plan.rs # RepairPlan / PlannedFile, sorted+deduped+blacklist-filtered work list, chunk layout (step 4)
src/repair/purge.rs          # Collapse files-cleanup: plan_purge, apply_purge, keep_set, classify_purge_path (step 5, 7)
src/repair/scope/     # the path scope and the byte ceilings: filter, limits, report (step 4, ceilings); ledger.rs is a thin alias for girpr-report's Meter/StoppedBy
src/repair/phases/check.rs   # --check-only size+MD5 verification loop (step 6.5)
src/repair/file/mod.rs       # per-file repair: the in-place tiers, the temp mode, reuse vs download (step 6)
src/repair/reuse.rs   # the salvage order: which reuse source survives, in which direction (salvage_order_refs in production; salvage_order is test-only)
src/repair/scan.rs    # the --scan pass: the plan's exact download estimate and damaged-chunk count
app/lock/          # the single-writer lock (outside the install)
app/audio/            # audio scan-file read/write + effective selection (acyclic leaf)
src/repair/phases/post.rs    # deprecated delete, audio cache->res move, purge-after, config.ini bump (step 7)
tests/repair_offline.rs   # offline integration/e2e against a local mock server
tests/cli_contract.rs     # binary-level argument handling and exit codes
tests/artifact_repair_bridge.rs  # the bundle-backed end-to-end suite (docs/06)
tests/common/mod.rs       # thin shims over the shared test libraries below
app/test-fixtures/     # facade-free offline mock, artifact replay server and deterministic fixtures
app/test-support/      # facade-coupled run glue (RunCtx construction, pipeline driver)
```

No build.rs (prost `derive` only, no protoc). No SQLite, no GUI, no speed limiter.

## 2. CLI

```text
girpr --game-path <DIR> --biz <hk4e_cn|hk4e_global|hk4e_bilibili>
      [--audio <zh-cn,en-us,ja-jp,ko-kr|none>]...   (repeatable; default: keep current else en-us; `none` = explicit game-only, overwrites scan file, cannot mix)
      [--jobs <N>]                            (concurrent FILES; a modest default; chunks sequential/file; `--io-threads` is a deprecated alias)
      [--purge-after] [--purge-before]          (same files-cleanup after / before; timing only)
      [--check-only] [--dry-run]               (verify / print actions, write nothing)
      [--repair-mode <auto|inplace|temp|discard>]  (how each file is written; default auto)
      [--max-ram-bytes <N>]                    (cap the RAM the in-place repair may stage)
      [--scan]                                 (read the install once, for exact plan figures)
      [--log-level error|warn|info|debug|trace] [--log-file-level error|warn|info|debug|trace] [--output text|json] [--log-file <FILE>] [--no-log-file]
```

The path scope, the byte ceilings, `--fail-fast`, `--purge-only`,
`--print-plan`/`--plan-out` and the endpoint overrides are
[`docs/04`](04-scope-limitation-spec.md)'s tier; the README's flag table is
the whole list.

Exit codes: 0 ok (incl. check-only clean), 1 usage/config (bad args — clap-level
included, so the code remaps clap's default 2 — unusable game path, non-chunk
`default_download_mode`), 2 metadata/network, 3 write/verify (counted per-file repair
failures via `Ok(summary, 3)` with `SUMMARY`, plus fatal config.ini / audio-scan write,
files-cleanup, game-dir creation and a live single-writer lock via `Err(RunFailure::write)`
with `FATAL` only), 4 check-only found damage,
5 a requested enforcement ceiling bound the run (`SUMMARY stopped=…`, `limited=1`). `--help`
is the only other 0. Fatal
errors carry their class via `config::RunFailure` at the raise site; per-file verify
failures continue and skip the post-phase instead. Automation: `tracing` diagnostics
on stderr; the begin `REPORT` line (versions + API fields), `PROGRESS` lines, and
the final `SUMMARY key=value` (NDJSON with `--output json`) go to stdout and are mirrored to the log.

## 3. Key flows → code mapping

| Spec step | Code | Provenance |
|---|---|---|
| Local version | `util::read_game_version` (1) | Starward |
| Server metadata (`game_config`/`game_branch`/`chunk_build` latest + optional local) | `repair::phases::metadata::fetch` + `hyp::HypClient` (2) | Starward |
| Effective audio + scan-file write | `girpr_audio::{resolve_effective_audio,write_scan_file}` (1), aliased as `audio` in `src/repair/mod.rs` | Starward |
| Begin `REPORT` | `report::report_record` + `report::emit` (2.5) | Starward |
| Manifest fetch/verify/parse | `repair::phases::manifest::fetch_latest` → `sophon::fetch_manifest` → zstd decode (streamed into a buffer the API's `uncompressed_size` sizes) → md5 check → `prost::Message::decode` (3) | Starward |
| Work list + reuse map | `girpr_repair_plan::build_plan` (4) + `repair::phases::manifest::local_reuse_map` → `sophon::build_local_chunk_map` keyed by path, chunk md5+size (3) | Starward |
| Exact plan figures (`--scan`) | `repair::scan::scan_plan` — one read per planned file, parallel over `jobs`, giving the download estimate and the damaged-chunk count (4) | — |
| Deprecated-file list (fetched in the plan phase, per-launcher `channel`/`sub_channel`) | `hyp::HypClient::deprecated_files` → `hyp_url(api, with_channel)` (2.6) | Starward |
| Files-cleanup before | `repair::purge::plan_purge` → `apply_purge` when `--purge-before` (5) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` parity) |
| Single-writer lock | `girpr_lock::Lock::acquire` — an OS advisory lock on `{temp}/girpr-{key:016x}.lock`, keyed by an FNV-1a hash of the canonical game dir; a live lock refuses the run with exit 3 (5, taken in the execute phase) | — |
| Per-file repair | `repair::file::repair_all` — one scan (whole-file MD5 + per-chunk verdicts) → stage the bytes that must move → write only the wrong chunks: staged copy, else `GET chunk_prefix/id` → zstd decode **streamed straight into the file**, digested on the way past → trim → final md5 (6) | Starward's chunk repair; the in-place write is what the launcher does too |
| Check-only | `repair::phases::check::verify_plan` (6.5) | Starward |
| Post deletes/config | `repair::phases::post::run` — deprecated delete + audio cache→res + `util::write_config_ini` (7) | Collapse-handling (section-preserving `config.ini` bump; `game_biz` force is a Starward carryover) |
| Files-cleanup after | `repair::purge::plan_purge` → `apply_purge` when `--purge-after` (7) → `deleted_extra_bytes` | Collapse-handling (`GetUnusedFileInfoList` v1 scope: expected = Sophon paths + `config.ini` + the legacy in-install `girpr.lock` + the cleaned blacklist) |
| Summary | `report::summary_record` + `report::emit`, emitted by `main` (8) | — |

Concurrency: a bounded spawn/join window over files (`repair::file::join_file_tasks`, at most
`jobs` tasks in flight); a single `reqwest::Client` with a fixed
connection-pool cap (see `app/fetch/src/hyp.rs`; not scaled by `jobs`); per-chunk `GET` sequential inside a file (HDD-safe). MD5 streaming
with fixed-size buffers. Retries: 5× with jittered exponential backoff on manifest/chunk/file ops.

## 4. Testing

`cargo nextest run` runs its test layers, all offline by construction:

- **Unit tests** (`#[cfg(test)] mod tests` next to the code): config.ini parse/bump,
  audio-lang mapping and flag validation, API URL/query building (per-launcher
  `channel`/`sub_channel`, `getBuild` tag encoding), local chunk-map build, manifest
  filter, work-list build/dedup, expected-set/purge classification, audio scan file,
  REPORT/PROGRESS/SUMMARY formatting, HYP wrapper and int-or-string parsing.
- **Integration tests** (`tests/repair_offline.rs`): a dedicated `#[tokio::test]` per
  behavior, each running the full `repair::run` pipeline against a `MockServer`
  on `127.0.0.1:<ephemeral>` (`app/test-fixtures`, reached through the thin
  `tests/common/mod.rs` shim) injected through
  `RunCtx::hyp_base_override` / `sophon_base_override`. Covers repair, check-only
  clean/dirty, dry-run, both purge timings, deprecated deletes, the non-chunk
  `default_download_mode` refusal, the per-launcher channel on the deprecated
  call, the local-slice reuse and the temp mode's resume-by-length (asserting the
  exact chunk download set) and its rollback of a chunk that fails its digest, the
  manifest's checksum gate, the in-place path's two tiers and the mode
  resolution, the single-writer lock, and the audio scan file.
  `--dry-run`/`--check-only` are covered here, so no live-game CI dependency exists.
- **Binary-level tests** (`tests/cli_contract.rs`): run the built binary
  (`CARGO_BIN_EXE_girpr`) to pin the process exit codes — every argument error is
  `1`, never clap's `2`, and `--help` is `0`. No game directory is touched.
- **The bundle-backed suite** (`tests/artifact_repair_bridge.rs`, [`docs/06`](06-artifact-bridge-spec.md)):
  `#[ignore]`d, and it drives the real binary over a captured bundle — a `--print-plan`
  pre-pass plus one real run per capture, the repair captures asking for
  `--repair-mode inplace` explicitly. It asserts the patched bytes, the exact fetched-chunk
  set and the reported download bytes against the bundle's ground truth; the other write
  modes are tested on crafted fixtures, not here.
- **The streaming invariant** (`app/fetch/src/sophon/stream.rs`): an always-on unit test
  (`the_streamed_decode_writes_in_pieces_not_one_buffer`) asserts that a decoded
  chunk reaches the sink in more than one write, every byte arrives, and no single
  write holds the whole body. It fails if streaming is replaced by a buffered
  decode. It replaces a former `#[ignore]`d peak-heap measurement that printed a
  peak-live-heap ratio but asserted nothing.

## 5. Logging & correlation

`tracing` to stderr (`--log-level`, with `RUST_LOG` adding to
it) plus a file log at `<exe-dir>/logs/girpr_<timestamp>.log` (`--log-file-level`).
Both layers are built in `src/logging.rs` over the shared
policy in `crates/log/` — the same policy the three dev tools (`gircap`, `girsync`,
`sled2redb`) follow. The two
levels are separate: `--log-level` only affects stderr, `--log-file-level` only
the file. A log file that cannot be opened degrades to a warning and a sink,
never a failure. Where the file goes, in order: `--no-log-file` (none at all),
then `--log-file`, then `GIRPR_LOG_FILE`, then the timestamped default. The
default itself is a release build's only: a debug build writes no file unless
one was asked for.
- `hyp` (debug): every HYP/Sophon JSON call with full request URL, retcode,
  node/attempt; (trace) full JSON response bodies (small metadata only, never chunk binaries).
- `sophon` (debug): manifest GET, compressed/decompressed byte counts, expected vs actual
  decompressed MD5, parsed file/chunk counts; (trace) per-chunk download byte counts.
- `repair`: every file task runs inside a `file{seq,total,task,path}` span (`task` = tokio
  async-task id, stable across worker-thread hops; first event notes the picking worker thread),
  so `grep 'path=<file>'` groups its whole lifecycle: `check start` (expect size/md5/chunks) →
  `check: missing|size mismatch|md5 mismatch (actual)|md5 match -> skip` → the scan's verdicts
  (`scanned`) → per-chunk trace (`chunk already complete|no local reuse candidate|reuse slice md5
  ok|slice mismatch -> download|chunk decoded`) → `repaired` (chunks total/reused/downloaded/kept,
  bytes, final md5). The temp mode adds `after: tmp ready for promote` before the rename.
  Check-only failures log `expect_*` vs `actual_*` at warn level.

## 6. v1 limits (documented, not bugs)

Chunk-repair only; no hdiff fast-update, no 7z legacy path, no SDK/WPF/plugin, inside-`game_dir`
manifest cache never written, `chunk/` reuse dir not retained. Dispatcher/persistent revisions are no
longer a v1 limit: the persistent tier now ships as the `--with-persistent` opt-in, described in
docs/01 §6 and implemented in §7 of this plan.

**The two write modes, and what each one is for:**

| | `inplace` (`auto` picks this when the target is writable) | `temp` (`--repair-mode temp`) |
|---|---|---|
| the target file | is written where it lies: the wrong bytes are replaced in place | is **never modified**: the repair is assembled in `{path}_tmp` and swapped in with an atomic rename |
| an interrupted run | leaves a file that fails the same check; the next run re-verifies and continues | leaves the target exactly as it was, plus a partial `*_tmp` the next run resumes by length |
| a final-MD5 mismatch | redoes only the chunks the per-chunk verdicts name | deletes the temp and restarts the file |
| the resume artefact | none: the scan finds the chunks that are already right, so a corrupt prefix cannot be inherited | the `*_tmp` prefix, kept and continued |
| peak extra disk | `jobs × largest *growth*` — 0 for a same-size repair | `jobs × largest file` |
| `stopped=--max-download-bytes`/`stopped=--max-write-bytes` | the next run re-verifies and continues | the next run resumes the `*_tmp` prefix |

The rows are two ways to spend disk, not two rungs of a safety ladder. Both gate the file on
the same whole-file MD5, and `config.ini` advances only when every planned file verified.

**The RAM a repair holds.** A downloaded chunk is **streamed**: the compressed body is
decoded as it arrives and the decoded bytes are written into the target as they are
produced, so the peak per active chunk is one read window plus the decoder's buffer,
not the chunk — a scale-free invariant in `app/fetch/src/sophon/stream.rs` fails if a buffered
decode ever writes the whole body in one piece. The
only RAM that scales with the file is the in-place tier's staging, and that is what
`--max-ram-bytes` bounds. A manifest is streamed too, into a buffer the API's
`uncompressed_size` sizes; its *decoded* bytes have to be resident for `prost` to parse
them, which is the one term the decode cannot remove.

**What is deliberately not implemented yet,** with the reason:

| item | why not now |
|---|---|
| the interval-graph optimum for the chunk order | the RAM tier removes the ordering constraint entirely wherever the payload fits, and the two-direction rule already captures a uniform shift — the dominant real shape — for the rest. It is a pure function (`repair::reuse::salvage_order_refs`, with test-only `salvage_order`) to swap, and the writer does not know which tier produced the order |
| a speculative single-pass capture (capture every candidate source range during the scan, instead of a second pass over the bad ones only) | saves a read pass when the plan-time estimate fits the budget; costs RAM for sources that turn out not to be needed. The exact two-pass scheme is simpler to reason about, and the read is mostly served by the page cache |
| cross-file salvage (a chunk from any local file, not just the same path) | mutually exclusive with an in-place repair absent a file-level order: repairing a donor destroys a donee's source. The reuse map is same-file-only, so the conflict is latent — but it must be settled before anyone adds it |
| reclaim-on-demand (delete files mid-run to make room) | the in-place repair removes most of the pressure; it collides with `--max-delete-bytes`' "before anything is deleted" promise and deserves its own flag and its own doc |
| an incremental protobuf parse of the manifest | the manifest's *decoded* bytes (the measured game manifest for a release build) must be resident for `prost` to parse them — the remaining term is protobuf, not zstd. The wire form is a sequence of `0x0A <varint len> <bytes>`, so each entry could be parsed on its own as the bytes arrive, but that needs the *read* side of the decoder (an async→sync bridge) and a small wire reader that must agree with `prost` on field order, unknown fields and framing. A separate piece of work with its own risk profile |
| hard-link dedup of byte-identical manifest paths | permanent space rather than transient, NTFS-only (exFAT, where GI installs often live, has no hard links), and it interacts with the in-place write: rewriting one link rewrites both |
| a compressed chunk cache on disk | strictly more disk, retained on purpose — the opposite of what this tool is for |

---

## 7. Persistent-directory update — opt-in tier

This is the shipped design of the `--with-persistent` feature. It adds one
opt-in phase group to the same repair pipeline; it does not amend the
repair-spec series (`docs/01`–`03`), the scope spec or the artifact bridge spec.
The research findings and the decisions behind the tier live in
[`docs/01-repair-findings-and-verdict.md`](01-repair-findings-and-verdict.md) §6;
how the path-scope and ceiling rules apply to persistent assets is summarised in
[`docs/04-scope-limitation-spec.md`](04-scope-limitation-spec.md) §9.

Every answer in docs/01 §6.5 is an **input** to this design, not an open
question. §7.9 maps each C/B item to the section, commit and test that enforces
it. The channel-coverage question B4 (CN/Bili) was settled by the gated first
commit recorded in §7.2, with the global-only answer (B4(a)) as the documented
fallback.

The commit table in §7.8 is a historical implementation record of the shipped
commits; the two documentation commits were folded into docs/01 §6 (research)
and this §7 (plan).

---

## 7.1. The feature, in one page

### 7.1.1 The opt-in invariant

One flag, `--with-persistent` (B1(a)), enables the whole dispatcher → manifests →
hotfix files → marker block chain. Without the flag, `girpr` makes **no
dispatcher request, no persistent request, and no marker write**, and its plan,
`SUMMARY` and exit codes stay byte-identical to a run without it.

Two exceptions are deliberate and run-wide, documented in the README and in
docs/01 §6.5 (C1/C2/C12, B5):

1. the purge keep-set gains the marker names and the root `pkg_version` for
   **all** runs, because leaving the default path destructive to an install's
   verification state is not an acceptable trade for a stable byte total;
2. the new `SUMMARY`/`PLAN` keys are conditional on the flag.

### 7.1.2 What the flag must buy

The game trusts a local install only when **both halves** of the verification
state are present and consistent (docs/01 §6.1):

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

### 7.1.3 What is explicitly out of scope

No Sophon path for hotfix bytes: the live probe (docs/01 §6.4.5,
`tools/persistent-sophon-research/README.md` §3) found zero hotfix MD5s in any
of the 15 advertised chunk manifests or 5 patch manifests, and the dispatcher
CDN answers full-file GETs with the manifest MD5 and supports `Range`. The
existing chunk writer is **not** extended; persistent files use a new
full-file HTTP path (C10). `DownloadPref`, `audio_versions*`, `*versions_remote*`
deletions, `VideoAssets/*.tmp` deletions and SDK/WPF/plugin extraction are out
of scope (B6); only the extra *keep* patterns are adopted. The
`silence_data_versions_persist` mirror **is** in scope, following
GenshinSlimmer and GI-Live-Hotfix-Downloader (docs/01 §6.2.2). Resumable `Range`
downloads remain an optional later improvement, not part of the shipped tier.

A follow-up live check (`tools/sophon-minimal-research/`) closes the
launchability question for the base half: the 7.2.0 Sophon set with one voice
language selected (2,974 files) is exactly the known-runnable torrent copy minus
the out-of-scope `BeyondAssets/` UGC editor, and **all** the `*_streaming` /
`*_revision` / `svc_catalog` / `ctable_streaming.dat` / root `pkg_version` files
are byte-identical Sophon-tracked base files. No extra fetch path is needed for
them, and `base_res_version_hash` is always recomputable from the Sophon-restored
`res_versions_streaming` (MD5 `f5ceb98f…`). The only non-Sophon state a
Sophon-only install lacks is the `Persistent/` marker block this tier adds; that
is why the flag remains opt-in and the base repair stays unchanged.

---

## 7.2. Commit 1 — research gate: B4 channel coverage and fixture capture

**This gate was run before the decoder was finalised.** The live probe covered
`hk4e_global` only; B4's answer is "try all three at launch, fall back to
global-only if CN/Bili cannot be verified". The gate decided which answer shipped.

### 7.2.1 Steps

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

### 7.2.2 Acceptance criteria / fallback

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

This gate produces the per-biz constants that §7.5.1.1's `DispatchKeys` provider
and §7.5.5's subdomain selection consume. It was a commit of scripts, samples and
docs — no product code.

---

## 7.3. Architecture and crate layout

### 7.3.1 New crates

| crate | owns | why it is separate |
|---|---|---|
| `app/dispatch/` (`girpr_dispatch`) | unwrap Collapse release metadata into key/seed, dispatcher URL building, RSA PKCS#1 v1.5 decrypt, protobuf decode, typed `DispatchInfo` | new dependency tree (`rsa`, `brotli`, `base64`, `prost`): an edit to repair, report or CLI leaves this cached, same reason `girpr-fetch` exists |
| `app/persistent-plan/` (`girpr_persistent_plan`) | value types and **pure** decisions: row normalisation, suffix→folder placement, force-persistent rule, CDN URL selection, marker rendering, scope suppression | plan-half decisions must be inspectable and unit-testable with no I/O; depends only on path/digest leaves |

### 7.3.2 Extended crates

| crate/module | addition |
|---|---|
| `app/fetch/src/persistent.rs` | fetch + MD5-verify + NDJSON-parse all three manifests into `PersistentRow`s; full-file HTTP download (primary → alternate on 404), temp file, digest gate, atomic publish |
| `app/repair-plan/` | carry a sibling `PersistentPlan` alongside `RepairPlan`; add persistent bytes to `DownloadEstimate`/`PlanSummaryValues`; feed dispatcher paths into `PurgeKeys` |
| `crates/model/src/region.rs` | const per-biz dispatcher subdomain list and channel prefix (`Region::dispatch_subdomains()`, channel accessor), with the existing constant-pinning tests extended. Values only: URL/request building stays in `girpr_dispatch` (the leaf's "values, not behaviour" rule) |
| `src/repair/phases/metadata.rs` | `Meta` gains `dispatch: Option<DispatchInfo>`; `meta.latest` is only known after `metadata.rs:307`, so the query runs at that point or in `fetch_and_report` |
| `app/report/` | conditional `SummaryFlags` fields and `PLAN` totals for persistent work |
| `src/cli.rs`, `src/config.rs` | `--with-persistent` → `RunCtx.with_persistent` |
| `src/repair/mod.rs`, `src/repair/phases/{metadata,post}.rs`, `src/repair/file/mod.rs` | phase wiring; `src/repair/purge.rs` keep rules |
| `app/test-fixtures/`, `tools/capture/`, `tests/artifact_repair_bridge.rs` | mock routes, bundle records, bridge cases |

### 7.3.3 Dependency direction (no cycles)

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

### 7.3.4 New dependencies (pin deliberately)

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
  private key; `config_GIGlb.json` (and the per-region files from §7.2) hold
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

Cargo.toml workspace members and `Cargo.lock` were updated in the commits
that introduced each crate.

---

## 7.4. Data model and pure decisions

### 7.4.1 Types (in `app/persistent-plan/`)

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

### 7.4.2 Placement (port of Collapse `Fetch.Persistent.cs`)

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
**ignored** for placement (Collapse parity, docs/01 §6.4.5).

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
required (docs/01 §6.4.5); every download attempts primary then alternate on a
404 only, never on a digest failure.

**Digest gate.** If `hash` is present, verify XXH64 of the downloaded bytes
against it; otherwise MD5. Re-verify after the atomic publish (a second read)
before the file counts as done — Collapse's HTTP path has no post-download
check, which is a known weakness we do not copy.

### 7.4.3 Marker bytes (port of Collapse `Fetch.cs:252–293`)

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

### 7.4.4 Version authority (B9(a))

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

## 7.5. Phase integration

All sites below are in the shipped `src/repair/mod.rs` pipeline:
`plan_phase` (`184–274`) → `fetch_and_report` (`298–318`) →
`fetch_plan_network` (`332–394`) → `build_and_assert_plan` (`421–530`) →
`publish_plan` (`768–813`) → `execute_phase` (`822–915`) →
`run_repair` (`1140–1160`) / `run_post` (`1167–1188`).

### 7.5.1 Plan half — decide, touch nothing

1. **Dispatcher query, in `fetch_and_report` after `metadata::fetch`.**
   Whenever `ctx.with_persistent` — `--purge-only` included, where the query and
   manifests are a keep-set input only (C20 revised: no marker, mirror or version
   is published in that mode); `--print-plan` and `--check-only` need the query
   so they can list/check the persistent set. Load `DispatchKeys` (B2(a): fetch and unwrap
   Collapse's release metadata from
   `metadata/v3/stable/config_master.json` + the per-region config file, pinned
   URLs + digest check on the key blob; B2(b) embedded versioned table only as
   an explicit, non-user-facing fallback), choose the subdomain (§7.5.5), build
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

2. **Persistent manifests, in `fetch_plan_network`** (same `with_persistent`
   gate, `--purge-only` included). The dispatcher's embedded descriptor NDJSON (root `3` → `22` → `3`, root `3` → `19` and root `3` → `20`)
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
   it to `Persistent/silence_data_versions_persist` (docs/01 §6.2.2). A failure
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

### 7.5.2 Execute half — put the bytes in place

The shipped `execute_phase` inserts a persistent repair path next to
`file::repair_all`, before the post phase, in this order:

1. **Directories and cheap prerequisites (gap rows 9–11).** After
   `prepare_install` but before repair: create `<exe>_Data/Persistent` and
   `<exe>_Data/StreamingAssets`; delete stale `Persistent/ctable.dat`; delete
   `DXSETUP/` (false-positive integrity errors). Then copy
   `StreamingAssets/ctable.dat` → `ctable_streaming.dat` **only if the source
   exists and the target is missing or differs**: on a torrent/Sophon-only
   install there is no `StreamingAssets/ctable.dat` (it is a launcher artifact
   present on E: but not on F:), while `ctable_streaming.dat` already ships in
   the Sophon base manifest and is MD5-verified by the base repair
   (`cd5f73407c6c4733f3c86a145e9f94c0`, live 7.2.0). Treat a missing source as
   a no-op, never as an error, and never overwrite an intact target with a stale
   source (prior research `notes/00` and `notes/01`: the repair tool only needs
   to keep/produce the `ctable_streaming.dat` source). All under the flag,
   never in read-only modes. The exe stem comes from
   `meta.config.exe_file_name`, not a hardcoded string.
2. **Move non-patch `.pck`/`.usm`/`.cuepoint` from `Persistent/` back to
   `StreamingAssets/`** (Collapse `Check.cs:91–145`), so a hotfix that ended
   does not leave a stranded duplicate. Never in `--check-only`. This is a
   standalone execute step (`persistent_exec::relocate_stranded_av`) placed
   **before `--purge-before`**: the purge candidates were walked in the plan
   half, before the move, so a stranded file is named as an orphan there. Moving
   first lands it at its manifest-expected `StreamingAssets/` counterpart, which
   the purge keeps; the repair pass then treats it as an ordinary candidate.
   Without the hoist the purge deletes the source first and no destination is
   ever created.
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

### 7.5.3 Post phase — publish markers last

In the shipped `src/repair/phases/post.rs::run_inner`, one step sits after the
`purge_after_pass` block (`~118`) and **before** the `config.ini` bump (`~123`):

1. If a delete ceiling ended the phase (`PostOutcome::DeleteCeiling`), return
   first — no markers.
2. If `ctx.purge_only` (C20 revised), skip the *publication*: no mirror, no
   marker, no version bump. The dispatcher and manifests are still queried in
   the plan half, but only to widen the purge keep set, so existing hotfix bytes
   under `Persistent/` that the manifests expect survive the cleanup. The
   run-wide keep-set (C1/C2) still protects existing markers.
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

### 7.5.4 Check-only

With the flag, `phases::check::verify_plan` also verifies (a) every persistent
file's size + digest against its manifest row, (b) all three mirrors' bytes against
the dispatcher MD5, and (c) every marker file's presence and exact value from
`MarkerSet`. Any mismatch is damage → exit `4`. Without the flag, check-only
is unchanged. The help text and README say that the flag narrows what
"intact" means (C5).

### 7.5.5 Subdomain selection

Collapse takes the region id from its preset and maps it
`0 → osusadispatch`, `1 → oseurodispatch`, `2 → osasiadispatch`,
`3 → oschtdispatch`, `4 → cngfdispatch`, `5 → cnqddispatch` (Bilibili),
with `OSRELWin` below 4 and `CNRELWin` from 4 up. `girpr` has no such preset:
use the ordered per-biz host list from the §7.2 gate, try each in order, and
accept the first response that decrypts to the expected game version and
codename for the biz. Log the chosen subdomain. If none works, exit `2` with
the list of attempted hosts. This is unattended and needs no local
`info.txt` read (GenshinSlimmer's fallback at
`%USERPROFILE%\AppData\LocalLow\miHoYo\Genshin Impact\info.txt`); that file
remains a last-resort diagnostic if the gate shows a biz where the ordered
list is insufficient.

---

## 7.6. User-facing contracts

### 7.6.1 CLI and exit codes

| Flag | Default | Meaning |
|---|---|---|
| `--with-persistent` | off | Query the dispatcher, update `Persistent/`, and publish the verification markers. Requires `--biz` coverage per §7.2. No effect on the default repair. |

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

### 7.6.2 Record stream

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

### 7.6.3 Docs

The README carries the flag table row, the opt-in contract paragraph,
check-only's narrowed meaning, the ceiling broadening, and the one-time
keep-set-driven `deleted_extra_bytes` change (C12/B5). The findings and
decisions are in docs/01 §6; this §7 is the implementation design.

---

## 7.7. Testing strategy

All tier tests are offline by construction; no test scans a directory it does
not own (`README.md` "For developers / AI agents"). Scratch trees go under
`<temp>/girpr-test/`.

| Layer | Location | What it pins |
|---|---|---|
| Unit — decoder | `app/dispatch/` tests | metadata blob unwrap against recorded samples; proto field decode against `samples/dispatch_fields.txt`; version-match rule; key/seed version coverage; URL/subdomain builders; per-biz channel constants |
| Unit — pure plan | `app/persistent-plan/` tests | suffix→folder table; force-persistent MD5 diff (same/different/missing base); URL primary/alternate swap; digest preference hash-vs-md5; blank-line and extra-key tolerance; marker byte rendering (no trailing newline, `PatchDone` = audio revision, hash omission); scope suppression predicate |
| Unit — purge | `src/repair/purge.rs` table test `cached_keys_classify_every_keep_rule` | new keep patterns for `*_persist`, `*_revision`, `ChannelName`, `ScriptVersion`, `PatchDone`, `base_res_version_hash`, `cache_versions_*`, `ctable*`, root `pkg_version`; negative rows proving `*versions_remote*` and `VideoAssets/*.tmp` still purge |
| Integration | `app/test-fixtures` mock + `tests/repair_offline.rs` (pulled into the `tests/repair.rs` root) | dispatcher failure → 2; version disagreement → 2; manifest MD5 failure → 2; three-mirror publish (res/data/silence), including silence CDN fallback; full hotfix download with primary 404 → alternate; corrupt body → retry/gate failure → 3 with no markers; read-only flags; `ctable`/`DXSETUP` prerequisites; read-only modes list-only; `--purge-only` queries the dispatcher but publishes no markers and keeps manifest bytes; stranded AV file with a manifest destination survives `--purge-before`; scoped run with marker path out of scope → 1; scoped run with hotfix path out of scope → suppressed; pre-flight and runtime ceilings including persistent bytes; check-only marker damage → 4 |
| Binary contract | `tests/cli_contract.rs` (in the `tests/contracts.rs` root) | flag parse, help text, exit codes, conditional `SUMMARY` keys absent unflagged and present in order under the flag |
| Bridge (`#[ignore]`d, run by hand) | `tests/artifact_repair_bridge.rs` + `app/test-fixtures/src/artifact_server.rs` | six repair-only cross-tool cases (identical, no-reuse, reuse-in-place, reuse-moved, added, removed) against captured bundles; each asserts the patched file's bytes, the exact set of chunks fetched and the patch's byte total |
| Manual, real install | recorded in docs/01 §6.6 step 5 | update Persistent with the flag, copy the game to a second machine, launch once, confirm no verification/re-download |

**Fixtures — offline mock.** The mock in `app/test-fixtures` carries a dispatcher
route whose body is a deterministically encrypted sample (fixed test RSA key +
seed committed as test data — never the production key), a key/seed provider
seam so tests do not touch Collapse's release repo, three manifest routes with
known MD5s, and a hotfix CDN route that can be scripted for 404-fallback and
corrupt-body cases. In `src/repair/phases/metadata.rs`'s own tests the
dispatcher route follows the existing `Reply`/`TestServer` seam
(`src/test_origin.rs`), alongside the `getGameConfigs`/`getGameBranches`/
`getBuild` switch.

---

## 7.8. Commit sequence (historical implementation record)

Each commit built and passed its own tests. The feature became user-visible
at C8; C6–C8 landed together before any release.

| # | Commit title | Touch | Verification |
|---|---|---|---|
| 1 | `docs: add persistent update implementation plan` | the plan document (this §7) | doc only |
| 2 | `test(persistent): capture dispatcher + manifest fixtures for all channels` | `tools/persistent-sophon-research/`, `CHANNELS.md`, trimmed samples | scripts re-run; MD5 checks logged; B4 verdict recorded |
| 3 | `fix(purge): keep verification markers and root pkg_version` | `src/repair/purge.rs`, purge tests, README note | `cargo nextest run`; unflagged `SUMMARY` change documented (C12) |
| 4 | `feat(dispatch): add dispatcher metadata/query decoder crate` | `app/dispatch/`, workspace, `Cargo.lock` | unit tests against committed samples; no product wiring |
| 5 | `feat(persistent): add persistent plan values and full-file fetch` | `app/persistent-plan/`, `app/fetch/src/persistent.rs`, workspace | unit tests for every pure rule and the download gate; no product wiring |
| 6 | `feat(persistent): wire the plan half behind --with-persistent` | `src/cli.rs`, `src/config.rs`, `app/report/`, `app/repair-plan/`, `src/repair/mod.rs`, `src/repair/phases/metadata.rs`, tests | dispatcher/manifest/plan/scope/ceiling cases; `PLAN`/`SUMMARY` keys; flag-off byte identity |
| 7 | `feat(persistent): repair persistent files in the execute half` | `src/repair/mod.rs`, new persistent repair module, mock routes, tests | download/fallback/gate/prereq/read-only cases; failure → 3 skips post |
| 8 | `feat(persistent): publish markers last and check them` | `src/repair/phases/post.rs`, `src/repair/phases/check.rs`, tests | marker byte pins (incl. `res_revision_eternal`), three-mirror publish (res/data/silence), C19/C20/C16/C5 cases, `config.ini` ordering |
| 9 | `test(capture): extend gircap bundle and bridge for persistent` (later reverted; repair-only bridge restored) | `tools/capture/`, `app/test-fixtures/`, `tests/artifact_repair_bridge.rs`, `docs/06` note | `gircap verify`; bridge cases green against a captured bundle |
| 10 | `docs(persistent): fold research into docs/01 and document the flag` (c6e8e763); `docs(persistent): fold the plan into docs/03 and retire the two separate follow-up docs` (8934ffac) | `README.md`, docs/01 §6, docs/03 §7 | doc review; real-install launch remains an operator step |

Where a commit's tests need a fixture the earlier fixture commit did not
provide, add that fixture in the same commit as its consumer (the bridge's
synthetic mini-bundle follows this rule today, `docs/06` §1.2).

---

## 7.9. Traceability

### 7.9.1 docs/01 §6.4 gap rows

| row | implemented by |
|---|---|
| 1 dispatcher query | §7.5.1.1, commit 4/6 |
| 2 persistent manifests | §7.5.1.2, commit 5/6 |
| 3 mirror to `*_persist` | §7.5.3, commit 8 |
| 4 revision block | §7.4.3, §7.5.3, commit 8 |
| 5 `base_res_version_hash` | §7.4.3 (C16), commit 8 |
| 6 placement + force rule | §7.4.2, commit 5 |
| 7 full-file downloader | §7.5.2.4, commit 5/7 |
| 8 read-only stripping | §7.5.2.3 (C17(b)), commit 7 |
| 9 create both trees | §7.5.2.1, commit 7 |
| 10 ctable / DXSETUP | §7.5.2.1, commit 7 |
| 11 move non-patch AV back | §7.5.2.2, commit 7 |
| 12 audio selection | reuse `meta.audio` + scan-file read; verify path equality in commit 6 (C14) |
| 13 config.ini bump | unchanged, ordering pinned in §7.5.3 (C9) |
| 14 extra-file purge | commit 3 + dispatcher paths in commit 6 (C1/C2) |
| 15 SDK/WPF/plugin keep | out of scope (B6); revisit only when widening keep inputs |
| 16 `pkg_version` rebuild | out of scope; root `pkg_version` kept by commit 3 (C2) |

### 7.9.2 docs/01 §6.5 decisions

| id | answer | where enforced |
|---|---|---|
| C1 keep-set for all runs | (a) | commit 3, purge table test |
| C2 root `pkg_version` keep | (a) | commit 3, `keep_set()` + test |
| C3 mirror in execute/post | (a) | §7.5.1.2, §7.5.3 |
| C4 read-only modes write nothing | (a) | §7.5.1.6, §7.5.3, integration tests |
| C5 check-only includes markers | (a) | §7.5.4 |
| C6 dispatcher failure → 2 | (a) | §7.5.1.1, §7.6.1 |
| C7 marker scope (d) + path scope (f) | (d)+(f) | §7.5.1.4 |
| C8 all-or-nothing markers | (a) | §7.5.2.5 |
| C9 markers before bump | (a) | §7.5.3 |
| C10 mode group excludes persistent | (a) | §7.1.3, §7.5.2.4; documented in README |
| C11 fold bytes into ceilings | (a) | §7.5.1.5 |
| C12 keep-set fix changes unflagged total | (a) | commit 3 + README note |
| C13 scan excludes persistent, visible in plan | (b) | §7.5.1 items 5–6 |
| C14 reuse audio selection | (a) | §7.9.1 row 12 |
| C15 atomic publish + digest gate | (a) | §7.5.2.4 |
| C16 omit `base_res_version_hash` if source missing | (a) | §7.4.3, `persistent_base_hash=omitted` |
| C17 strip read-only on persistent paths only | (b) | §7.5.2.3 |
| C19 always write full block under flag | (a) | §7.5.3 |
| C20 purge-only queries for keep set, publishes nothing | (d), superseding (a) | §7.5.1.1, §7.5.3, integration test |
| B1 one master flag | (a) | §7.1.1 |
| B2 key/seed from metadata, embedded fallback | (a)/(b) | §7.5.1.1 |
| B3 failure while opted in → 2 | (a) | §7.6.1 |
| B4 all three channels, fallback global-only | (b)→(a) | §7.2 gate |
| B5 ship keep-set fix independently | (a) | commit 3 |
| B6 Collapse parity + silence mirror + extra keep inputs | (a), extended to maintain `silence_data_versions_persist` from GenshinSlimmer / GI-Live-Hotfix-Downloader | §7.1.3, docs/01 §6.2.2, §7.5.1.2, commits 3/6/8 |
| B7 fold bytes into existing ceilings | (a) | §7.5.1.5 |
| B8 legal/ToS posture accepted | (a) | §7.1.1, §7.5.1.1; project-owner call already recorded |
| B9 `meta.latest` authoritative | (a) | §7.4.4 |

---

## 7.10. Risks, implementation-time checks and non-goals

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

## 7.11. Appendix — exact constants to port

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
  (per-region files resolved in the §7.2 gate)
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
  delete DXSETUP/; copy StreamingAssets/ctable.dat → ctable_streaming.dat only
  if the source exists and the target differs; a missing source is a no-op on
  Sophon/torrent installs (the target itself is a verified Sophon file)
```

The biggest dataset to pin in tests is the live probe's shape: 3,164 rows
(2,631 `res_versions_external` + 533 `data_versions`), 1,661 base copies that
the MD5 rule keeps out of `Persistent/`, and 376 hotfix-only MD5s — including
all 10 server-flagged `isPatch` rows. The live probe did **not** fetch the
silence manifest; its descriptor (root `3` → `20`) and full NDJSON were
captured in the fixture commit (§7.2 step 3, commit 2) and its row count pinned
there. The trimmed samples under `tools/persistent-sophon-research/samples/`
are the fixture source; the full manifests and ciphertext stay out of the tree.
