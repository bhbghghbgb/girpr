# girpr — Genshin Impact low-disk repair patcher

Brings a possibly-corrupt, possibly-any-older-version Genshin install to the
current live version with minimal extra disk. **Unattended by design:**
args in, exit code + `REPORT`/`SUMMARY` stdout lines out, no UI.

Core = **Starward's repair-in-chunk-mode** (version-agnostic, per-file
chunk repair, verified local-chunk reuse) — now written **in place**, so a
repair costs the file's *growth* rather than a second copy of it
([`docs/07`](docs/07-low-disk-repair-findings-and-plan.md)) —
**+ Collapse's extra-file purge** as a pre/post phase.
**Docs are several series.** The `repair-` prefix is load-bearing:

| series | docs | describes |
|---|---|---|
| **repair spec** (shipped, describes the whole-install patcher) | [`docs/01-repair-findings-and-verdict.md`](docs/01-repair-findings-and-verdict.md) · [`docs/02-repair-chunk-spec.md`](docs/02-repair-chunk-spec.md) · [`docs/03-repair-rust-implementation-plan.md`](docs/03-repair-rust-implementation-plan.md) | findings → spec → Rust module plan for `docs/02` |
| **scope limitations** (shipped) | [`docs/04-scope-limitation-spec.md`](docs/04-scope-limitation-spec.md) | `--include`/`--exclude`, byte/file ceilings, `--print-plan`, endpoint override |
| **artifact bridge** (shipped) | [`docs/06-artifact-bridge-spec.md`](docs/06-artifact-bridge-spec.md) | the `#[ignore]`d end-to-end test against a real bundle |
| **low-disk repair** (shipped) | [`docs/07-low-disk-repair-findings-and-plan.md`](docs/07-low-disk-repair-findings-and-plan.md) | why a repair rewrote every file, the options to stop, and the in-place repair that replaced temp-then-promote |

The repair-spec series describes the whole-install patcher and is unaffected by
the scope-limitations tier: a flag list and a byte ceiling are not a change to
the repair algorithm. `docs/04` names every `girpr` site a path scope has to
reach. `docs/07` **does** change the algorithm — the repair is now written
**in place** — and where the two disagree, `docs/07` wins.

## For developers / AI agents — read this first

`src/main.rs` is a thin shim (parse → tracing → run → `SUMMARY` → exit). All
behavior is in the library; `src/repair/mod.rs::run` is the pipeline
coordinator and holds no logic beyond phase order and the exit-code contract.

**Entry points.**

| File | Owns | Key symbols |
|---|---|---|
| `src/main.rs` | shim: `Args::parse` → `init_tracing` → `repair::run` → `SUMMARY` → `exit` | `main` |
| `src/lib.rs` | module map + re-exports | — |
| `src/cli.rs` | clap `Args`, audio-lang normalization, flag validation | `Args`, `Args::into_ctx`, `parse_audio_selection`, `normalize_audio_lang`, `is_audio_none` |
| `src/config.rs` | validated run config; failure carrying the exit code | `RunCtx`, `RunCtx::readonly`, `RunCtx::api_bases`, `RunFailure` |
| `src/logging.rs` | console layer + always-`TRACE` file layer | `init_tracing`, `log_file_path` |
| `src/report.rs` | run counters + every stdout line of the contract | `Summary`, `SummaryFlags`, `Summary::snapshot`, `emit`, `report_record`, `progress_record`, `emit_progress`, `summary_record`, `spawn_progress_reporter` |
| `src/biz.rs` | biz/channel/launcher/game-id mapping | `Biz::endpoints`, `Biz::channel_tuple` |
| `src/hyp.rs` | HoYoPlay + Sophon JSON APIs | `HypClient::new`, `HypClient::new_with_bases`, `game_config`, `game_branch`, `chunk_build`, `deprecated_files`, `production_bases` |
| `src/sophon.rs` | chunk-manifest protobuf, fetch+verify+parse, manifest filter, local chunk map | `select_manifests`, `fetch_manifest`, `fetch_chunk_bytes`, `build_local_chunk_map`, `join_url` |
| `src/repair/mod.rs` | phase coordinator: plan, then assert, then execute; exit-code contract | `run` |
| `src/repair/metadata.rs` | local state + server metadata (docs/02 1–2) | `fetch`, `Meta` |
| `src/repair/manifest.rs` | manifest fetch + local chunk-reuse map (3) | `fetch_latest`, `local_reuse_map`, `LocalChunkMap` |
| `src/repair/plan/mod.rs` | work list (4) + the plan module root | `build_plan`, `RepairPlan`, `RepairPlan::expected_paths`, `RepairPlan::est_download_bytes`, `est_file_download_bytes`, `PlannedFile`, `PlannedFile::layout` |
| `src/repair/plan/purge.rs` | Collapse files-cleanup: plan (5, 7) + apply | `plan_purge`, `apply_purge`, `PurgePlan`, `PurgeCandidate`, `keep_set`, `classify_purge_path`, `PurgeVerdict` |
| `src/repair/check.rs` | `--check-only` verification (6.5) | `verify_plan` |
| `src/repair/file.rs` | per-file repair: the in-place writer and the temp path (6) | `repair_all`, `ChunkStats`, `Repaired` |
| `src/repair/reuse.rs` | the salvage order: which reuse sources survive which write order | `salvage_order`, `Order`, `find_candidate` |
| `src/repair/scan.rs` | the `--scan` pass: the plan's per-chunk verdicts | `scan_plan`, `FileVerdict`, `ScanReport`, `chunks_to_download` |
| `src/repair/lock.rs` | the single-writer lock (P5) | `acquire`, `Lock`, `LockError` |
| `src/repair/post.rs` | deprecated/audio/`config.ini` + cleanup-after (7) | `run` |
| `src/repair/scope/mod.rs` | scope-of-work facade: path scope + ceilings | `PathScope`, `Limits`, `LimitHit` |
| `src/repair/scope/filter.rs` | `--include`/`--exclude` glob semantics (pure) | `PathScope::new`, `is_visible`, `is_active`, `is_open`, `has_includes`, `compile_patterns` |
| `src/repair/scope/limits.rs` | the pre-flight ceiling predicates (pure) | `Limits::check_preflight`, `Preflight`, `LimitHit::describe` |
| `src/repair/scope/report.rs` | the `PLAN` line + the `--plan-out` JSON document | `PlanTotals`, `plan_record`, `PlanDocument` |
| `src/repair/scope/ledger.rs` | the runtime reservations the hard stops read | `Summary::reserve_download_bytes`, `reserve_write_bytes`, `adjust_download_bytes`, `stop_downloads`, `stop_writes`, `abort` |
| `src/repair/audio.rs` | audio scan-file format (used by 1 and 7) | `resolve_effective_audio`, `read_scan_file`, `write_scan_file` |
| `src/util.rs` | MD5, `config.ini`, ignore/blacklist files, rel-path normalization | `md5_file`, `md5_file_slice`, `file_len`, `read_game_version`, `write_config_ini`, `read_ignore_categories`, `read_blacklist`, `normalize_rel` |
| `tests/repair_offline.rs` | offline integration/e2e (mock API) | one `#[tokio::test]` per behavior |
| `tests/cli_contract.rs` | binary-level arg handling + exit codes | one case per usage error class |
| `tests/common/mod.rs` | mock server + deterministic fixture | `MockServer`, `build_fixture`, `MockOpts` |
| `tests/artifact_bridge.rs` | `#[ignore]`d e2e against a real `gircap` bundle (docs/06) | `drive_capture`, one test per capture |
| `tests/artifact_bridge/server.rs` | the loopback replay server, reached only by the test above | `Bridge`, `Bundle`, `Capture`, `Hit` |

**Action graph** — what one `repair::run(ctx)` does (numbers = `docs/02` steps):

```mermaid
flowchart TD
    A["parse Args<br/>cli.rs: validate io_threads, audio, globs, ceilings → RunCtx"] --> B["metadata::fetch<br/>config.ini game_version (last-match-wins)<br/>ignore + blacklist files<br/>getGameConfigs → getGameBranches<br/>→ getBuild(latest) → getBuild(local, best-effort)"]
    B -->|FILE mode| X1["exit 1 (usage)"]
    B -->|retcode/http fail| X2["exit 2 (metadata)"]
    B --> C["print REPORT to stdout+log<br/>local vs latest + API fields"]
    C --> D["manifest::fetch_latest<br/>drop ignores + unselected audio<br/>fetch + zstd + MD5 + protobuf<br/>+ local_reuse_map"]
    D -->|manifest bad| X2
    D --> E["plan::build_plan: sort, dedup, blacklist-filter, scope-filter<br/>+ purge plan + deprecated list"]
    E --> E2{"any ceiling requested?"}
    E2 --> F{"purge-before?"}
    E2 -->|over a ceiling| X4["exit 5 — nothing written<br/>SUMMARY limited=1"]
    E2 -->|"--print-plan"| P["emit PLAN … (+ --plan-out JSON)<br/>exit 0, writes nothing"]
    F -->|yes, not check-only| G["purge::plan_purge → apply_purge<br/>count → deleted_extra_bytes"]
    F -->|no| H{"check-only?"}
    G --> H
    H -->|yes| I["check::verify_plan<br/>exit 0 clean / 4 damaged<br/>writes nothing"]
    H -->|no| J{"--purge-only?"}
    J -->|yes| MP["post::run, no repair loop<br/>cleanup + post-phase<br/>exit 0, config.ini not bumped"]
    J -->|no| J2["file::repair_all: ≤jobs files in parallel<br/>chunks sequential per file"]
    subgraph perfile ["per file"]
        J1["scan: one read answers everything<br/>(whole MD5, per-chunk verdicts)"] --> J2a["in place: write only the wrong chunks<br/>(staged in RAM, or the salvage order)<br/>else download chunk → zstd → MD5 → write at offset"]
        J2a --> J3["final whole-file MD5 gates<br/>(a mismatch redoes the wrong chunks)"]
    end
    J2 --> K{"any failed?"}
    K -->|yes| L["SUMMARY with failed>0<br/>exit 3 (skip post-phase)"]
    J2 -->|"a byte ceiling hit mid-flight"| X5["SUMMARY stopped=bytes\|write<br/>exit 5"]
    K -->|no| M["post::run (skipped when readonly)<br/>deprecated delete → audio cache→res move<br/>→ purge-after? → config.ini bump"]
    M -->|"delete ceiling hit"| X6["SUMMARY stopped=delete<br/>exit 5, that pass deleted nothing"]
    M -->|write fails| X3["FATAL, exit 3 (no SUMMARY)"]
    M --> N["SUMMARY to stdout+log<br/>exit 0"]
```

**Machine contracts (do not break without updating tests + this table).**

- stdout: `REPORT …` once after metadata; `PROGRESS …` every 10 s;
  `PLAN …` with `--print-plan`; `SUMMARY total=… skipped=… repaired=… failed=… download_bytes=… deleted_extra_bytes=… exit=…`.
  `--output json` emits every one of those lines as an NDJSON object instead —
  same records, same order, each carrying an `event` name.
  A **scoped or limited** run appends conditional keys to that line
  (`scoped=1`, `config_bumped=0`, `plan_files=…`, `limited=1`, …). Every one is
  emitted **only** when its flag is active, so an unflagged run's `SUMMARY` is
  byte-identical to one that never passes them.
  `local_manifest=ok|unavailable` is the one exception to "flag-driven": it is
  emitted when a local `config.ini` version was known and `getBuild(local)` was
  attempted, because that lookup is the one thing that moves both byte counts
  while still exiting `0` — see below. Absent on a fresh install.
  `--repair-mode` adds `repair_mode=…`, `inplace=…`, `temp=…` and
  `kept_bytes=…` (the bytes the run did **not** spend); `--max-ram-bytes` adds
  `ram_bytes=…` (the peak staged payload); `--scan` adds `damaged_chunks=…`.
  `PLAN` carries `est_write_bytes=…` (what the plan would write into the
  install) and `est_extra_bytes=…` (what it would need *beyond* it: the
  files' growth under the in-place path, a whole file each under the temp
  path), plus `scan_bytes=…` under `--scan`.
- **The binary cannot report which chunks were requested.** `SUMMARY` is
  byte-level by design. Per-chunk request detail exists only as `trace` lines in
  `<exe-dir>/logs/girpr_<ts>.log`, so asserting "this chunk id was never
  fetched" needs a request log (`--log-level trace`), not the data plane.
- stderr: structured `tracing` logs (`--log-level` / `RUST_LOG`); always mirrored at
  `TRACE` to `<exe-dir>/logs/girpr_<timestamp>.log`.
- path scope: a path outside `--include`/`--exclude` **does not exist** — it is
  dropped from the plan *and* kept by the files-cleanup, so `--purge-*` cannot
  eat it. A scoped run does not bump `config.ini` (the bytes are deliberately
  not in place); `SUMMARY config_bumped=0` says so. `--audio` under a scope is
  exit `1`. So is an `--include` that selected **nothing at all** — but an empty
  *repair* plan on its own is not a typo: a path on disk that the new manifest
  dropped is in no repair plan by construction, so `--purge-only --include
  <that path>` deletes it rather than erroring, which is what `--purge-only`
  exists for.
- resource ceilings are **enforcement bounds, not selectors**: a run that would
  exceed one refuses (exit `5`) *before doing any work* — nothing is written,
  no directory created, no file opened for writing, no file deleted. They never
  trim the work list; under `--include` they are a *check on* the selection,
  which is why the two flags are not redundant. `--max-files N` with exactly N
  planned files passes (`> N`, not `>= N`). One exception to "nothing is
  written": a **runtime** hard stop, which fires mid-flight by definition. It
  leaves the files it was writing half-patched — safe, because the next run
  re-verifies and finishes them (resume-by-verification) — and `SUMMARY
  stopped=` says which ceiling bound and that it happened at all: `stopped=bytes`
  (`--max-download-bytes`), `stopped=write` (`--max-write-bytes`),
  `stopped=delete` (a cleanup pass refused). That key is what distinguishes
  "nothing was written because the plan was too big" from "something was
  written and then we ran out".
- the **single-writer lock**: a run that is going to write takes
  `{game_dir}/girpr.lock` first (created with `create_new`, so the creation is
  the exclusion). A second run is refused with exit `1`, naming the file and
  the holder's pid; a lock older than 24 h is taken over. The lock is only
  taken when the path scope admits it, and the files-cleanup keeps it.
- `SUMMARY download_bytes` means **the bytes of files that were actually
  repaired**. An attempt that fails and is retried hands its
  bytes back, so a run that had to retry does not report what the network cost
  the retries. The counter is nevertheless live during the repair, which is what
  `--max-download-bytes` needs to stop mid-flight. Under the in-place repair it
  is also **smaller than it used to be**: a chunk that is already correct where
  it belongs is kept, not fetched.
- chunk reuse is **best-effort and the degradation is reported**:
  `SUMMARY local_manifest=unavailable` means `getBuild(local)` could not be
  served, so the reuse map was empty. The run is still correct (more bytes,
  never a wrong result) and still exits `0` — but a file whose content
  *shifted* can no longer be salvaged from its own bytes, so it downloads what
  the in-place path would have kept. Check this key before comparing byte
  totals.
- exit codes: `0` ok (check-only: intact) · `1` usage/config (a bad flag, a
  held lock, an unusable path scope) · `2` metadata/network ·
  `3` write/verify (per-file `Ok(summary,3)` with `SUMMARY`, or fatal `Err` with `FATAL` only) ·
  `4` check-only found damage · `5` a manually requested enforcement limit was
  exceeded. A **refusal** writes nothing at all; a **runtime hard stop** writes
  part of the work and leaves the files it was writing half-patched
  (`SUMMARY stopped=…`). Either way
  `SUMMARY` is still emitted, with `limited=1`.  `5` is deliberately not `1`: the API has to be fetched first, so whether a
  limit trips depends on the server's answer — `--max-files 0` is not an error
  against an empty plan and is on a 3000-file one. Every argument-level failure
  (unknown flag, bad enum value, `--io-threads 0`, bad `--audio`, bad
  `--include` glob, non-numeric `--max-files`) is `1` — clap's default `2` is
  remapped so it cannot collide with "metadata/network". `--help` is `0`.
- `getGameConfigs.default_download_mode` must be `DOWNLOAD_MODE_CHUNK`; anything
  else (including `DOWNLOAD_MODE_FILE` / `DOWNLOAD_MODE_LDIFF`, which need the
  7z/hdiff paths v1 lacks) exits `1` instead of failing later at `getBuild`.
- `getGameDeprecatedFileConfigs` is the one API that needs `channel`/`sub_channel`,
  and it uses the per-launcher pair (cn `1/1`, global `1/0`, bili `14/0`) — the
  deprecated list is filtered by them, so cn/bilibili would get the wrong set.
- read-only modes: `--dry-run` and `--check-only` write nothing (no purge, no scan-file,
  no `config.ini`, byte counters stay 0; purge lists are only logged).
  `--purge-only` is read-only *in the repair sense* — it does the cleanup half
  and skips the repair loop. It does **not** bump `config.ini`: a cleanup puts no
  bytes in place, so claiming the newer version would desync the launcher
  (same rule as a scoped run), and on an already-current install it is a no-op
  anyway. `SUMMARY config_bumped=0` says so.
  `--print-plan` is read-only by construction too (it returns before
  `create_dir_all`) but is **not** the same thing as `--check-only`: it answers
  "what would you do", not "what is wrong". It also short-circuits `--dry-run` —
  the run returns at the plan either way, so the combination is just
  `--print-plan`. A ceiling still trips a `--print-plan` run.
- purge: `--purge-before` and `--purge-after` run the **same** cleanup, in two
  calls — `plan::purge::plan_purge` decides, `plan::purge::apply_purge` acts —
  (expected set = live-manifest paths + `config.ini` + `girpr.lock` +
  `audio_lang_*`/`Audio_*_pkg_version`
  patterns; **everything else purges**, incl. temps, exe, logs, `ScreenShot/`).
  The whole plan is built before the first delete.
- audio: omitted flag = keep scan file (default `en-us` if unreadable); explicit list/`none`
  overwrites the scan file. `none` never mixes with langs.
- per-file spans: `file{seq,total,task,path}` — `grep 'path=<file>'` groups a file's lifecycle.

**Deliberate simplifications (do NOT "fix" without reading the linked code).**
S1 whole-blob buffering (`sophon.rs`, `repair/file.rs`) · S2 same-file-only chunk reuse
(`repair/file.rs`, `sophon::build_local_chunk_map`) · S3 resume-by-length + final-MD5 gate
(`repair/file.rs`) — **the temp path only**: the in-place path resumes by
*verification*, and the file itself is the artefact. Purge keep-set is
`config.ini` + `girpr.lock` (`plan::purge::keep_set`).
Version parse is last-match-wins (`util::read_game_version`, Starward parity).

**Weakened invariants (docs/07 §5) — the in-place path's price.** The
"original is never modified" and "a crash leaves the install as it was"
guarantees are deliberately gone: a file that already failed its own
whole-file MD5 is patched where it lies, so a crash leaves a file that
fails the same check and converges on the next run. The invariants that
survived: the plan comes from the latest manifest alone, every byte
written is hash-gated before it lands, a rerun is idempotent, the
ceilings refuse before any mutation, and `config.ini` advances only when
every planned file verified.

## Requirements / build / usage

- Rust 1.85+ (the crate is `edition = "2024"`); network to HoYoPlay + Sophon
  CDNs (production runs only).
- Game dir (read/write); free bytes ≈ `io_threads × largest file` beyond the
  game under `--repair-mode temp`, and ≈ `io_threads × largest growth` under
  the default in-place path — which is **0** for a same-size repair.

```powershell
cargo build --release
# Preview (writes nothing)
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --purge-after --dry-run
# Verify only; exit 4 if damaged
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --check-only
# Repair + purge extras
.\target\release\girpr --game-path "D:\Genshin Impact game" --biz hk4e_global --purge-after --io-threads 4
```

| Flag | Default | Meaning |
|---|---|---|
| `--game-path <DIR>` | (required) | Install dir (`config.ini`, `*_Data`, …) |
| `--biz <BIZ>` | (required) | `hk4e_cn` \| `hk4e_global` \| `hk4e_bilibili` |
| `--audio <LANG>`… | keep current, else `en-us` | Repeatable: `zh-cn`, `en-us`, `ja-jp`, `ko-kr` (case-insensitive, `_` accepted for `-`; display names `chinese`/`english`/`english(us)`/`japanese`/`korean` also work); `none` = game-only (overwrites scan file) |
| `--io-threads <N>` | `4` | Concurrent **files** (chunks per file sequential → HDD-friendly). SSD 4–8, HDD 1–2 |
| `--purge-after` / `--purge-before` | off | Same files-cleanup, different timing (before frees repair space) |
| `--include <GLOB>` | (every path) | Repeatable. Only act on paths matching this glob (`/`-separated, relative to `--game-path`, case-insensitive). `*` also crosses `/`. Non-matching paths do not exist to the run: not repaired, **not deleted**, and `config.ini` is not bumped |
| `--exclude <GLOB>` | (nothing) | Repeatable. Treat matching paths as nonexistent; beats `--include` |
| `--max-files <N>` | (no limit) | Refuse (exit `5`) if the plan touches more than N files. `0` is a valid ceiling. Checked **after** `--include`, so it means "the files the scope selected" |
| `--max-delete-bytes <N>` | (no limit) | Refuse (exit `5`) if the files-cleanup would delete more than N bytes. Checked against the cleanup candidates computed before any work starts, and again against the post-repair pass's own candidates before it deletes anything (`stopped=delete`); out-of-scope paths are never candidates |
| `--max-download-bytes <N>` | (no limit) | Refuse (exit `5`) if the plan's **estimated** chunk downloads exceed N, and hard-stop mid-flight (exit `5`, `SUMMARY stopped=bytes`) if reuse falls back to more than the estimate predicted. Does **not** bound metadata or manifest traffic |
| `--max-write-bytes <N>` | (no limit) | Hard-stop mid-flight (exit `5`, `SUMMARY stopped=write`) once N bytes have been written into the install. This is the **disk** cost: reuse-copies and chunk writes are counted, neither of which `download_bytes` sees |
| `--repair-mode <MODE>` | `auto` | `auto` repairs **in place** wherever the target can be opened read+write (so a repair costs the file's growth, not a second copy of it); `inplace` forces that; `temp` is the shipped behaviour (write `{path}_tmp`, then rename it over the target — the original is never modified, and the extra space is one whole file per file in flight); `discard` deletes the target first and writes it fresh. The `SUMMARY` keys `repair_mode=…`, `inplace=…`, `temp=…`, `kept_bytes=…` appear only when the flag is passed |
| `--max-ram-bytes <N>` | `64 MiB` | Cap the RAM the in-place repair may use to stage the bytes that have to *move* inside a file. It is a **budget**, not a refusal: a file whose payload does not fit falls to the ordered path (which writes the same file without staging anything). `0` forces that path for every file. `SUMMARY ram_bytes=…` appears only when the flag is passed |
| `--scan` | off | Read every planned file before the repair starts, so the download estimate becomes **exact** (a chunk that is already correct where it belongs is not fetched), `--print-plan` reports `scan_bytes`, and `--check-only` reports `damaged_chunks`. Costs a full read pass before the repair |
| `--check-only` | off | Verify size+MD5; writes nothing |
| `--purge-only` | off | Make the install match the manifest: files-cleanup + post-phase, **no repair loop** (it *implies* the cleanup, so no purge flag is needed). The inverse of `--check-only`. Does not bump `config.ini` |
| `--fail-fast` | off | Stop the run once one file exhausts its retries; no further files are started. Only worth setting for `5xx`-shaped failures: a 4xx costs nothing on its own, because only `408`/`429`/`5xx` are retried (a 404 on a chunk is asked once, not five times) |
| `--dry-run` | off | Log actions; writes nothing |
| `--print-plan` | off | Print the whole plan (`PLAN files=… est_download_bytes=… purge_files=… purge_bytes=… max_chunk_bytes=… scoped=…`) and exit 0 without repairing, purging or bumping anything. **Not** an offline mode — the plan *is* the manifest, so it still fetches |
| `--plan-out <FILE>` | (none) | Write the per-file plan as JSON `{repair:[…],purge:[…]}` |
| `--output <text\|json>` | `text` | One human `key=value` line per record, or the same records as NDJSON (`report` / `progress` / `plan` / `summary` events). Stdout only; diagnostics stay on stderr |
| `--log-level <LVL>` | `info` | `error`/`warn`/`info`/`debug`/`trace` (`RUST_LOG` also honored) |
| `--log-file <FILE>` | `logs/girpr_<timestamp>.log` beside the executable | Write the trace log here instead. Missing parents are created. The file layer captures `TRACE` and above **regardless of `--log-level`**, so `--log-level trace` is only needed to also see it on stderr. An unopenable file is a stderr warning, never a failure |
| `--hyp-base <URL>` | production | Point the HoYoPlay API at another base (overrides `GIRPR_HYP_BASE`) |
| `--sophon-base <URL>` | production | Point Sophon `getBuild` at another base (overrides `GIRPR_SOPHON_BASE`) |

`--hyp-base` / `--sophon-base` redirect the **metadata calls only**. Manifest
and chunk URLs come from the `url_prefix` inside the response *bodies*, not from
the configured base, so a local mock must return bodies pointing at itself.

## Testing (offline by construction)

```powershell
cargo test            # 123 unit + 67 integration, all offline
cargo test --test repair_offline
cargo clippy --all-targets

# the artifact bridge: #[ignore]d, needs a bundle (docs/06)
cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
```

- No test touches the internet: every case builds a `MockServer` on
  `127.0.0.1:<ephemeral>` and injects it via `RunCtx::hyp_base_override` /
  `sophon_base_override` (`src/config.rs`; resolved in `src/repair/metadata.rs`;
  `HypClient::new_with_bases` in `src/hyp.rs`).
- Binary-level e2e without code changes: set `GIRPR_HYP_BASE` / `GIRPR_SOPHON_BASE`
  env vars, or pass `--hyp-base` / `--sophon-base` (explicit > env > production;
  resolved in `RunCtx::api_bases`).
- **What the tests actually drive.** `tests/repair_offline.rs` is 67 cases, and
  most of them call `repair::run(ctx)` **in-process** with a hand-built `RunCtx`
  — that exercises the library, not the flag surface. Five of them (the
  endpoint-override cases and the `--output` format case) spawn the
  real binary through
  `CARGO_BIN_EXE_girpr`, which is how the `--hyp-base` / `--sophon-base`
  precedence is pinned; they still run against the mock, not against real bytes.
  `tests/cli_contract.rs` is the other file that spawns the binary, and it never
  reaches the API at all — it is argument handling and exit codes only.
- **Real bytes, real flags — and expensive.** `tests/artifact_bridge.rs` runs
  the built binary against a `gircap` artifact bundle replayed over loopback,
  which is what proves `--include` and `--hyp-base` / `--sophon-base` reach
  `HypClient` through the real flag-parsing path. It is **`#[ignore]`d because
  it costs ~2 minutes** of real binary runs over ~20 MB of bundle bytes per run,
  and it is meant to be started **by hand, only when a repair-spec change is
  suspected**:
  ```powershell
  cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
  ```
  It still *compiles* under `cargo test`, so a broken bridge fails to build
  rather than failing silently. And because it only ever runs deliberately, a
  **missing prerequisite is an error**: no bundle under `artifacts/` and no
  `GIRPR_ARTIFACT_BUNDLE` fails the run with the command that would produce
  one, rather than passing quietly having checked nothing.
  - The spec is [`docs/06-artifact-bridge-spec.md`](docs/06-artifact-bridge-spec.md).
  - The server rewrites only the **scheme and authority** of the `url_prefix`
    inside each `getBuild` body, because manifest and chunk URLs are built from
    that body rather than from the flags. It never writes to the bundle; the
    working tree is a scratch copy, and `gircap verify` must still pass
    afterwards.
  - Per capture it asserts: `download_bytes` is **at most** the patchmap's
    total (the in-place repair keeps the chunks that are already correct where
    they belong, so it fetches *fewer* bytes than the patchmap counts), the
    patched file's MD5 equals `expectations.result_md5`, no id in
    `must_not_fetch` was requested, exit `0` with `config_bumped=0` and nothing
    outside the scope touched on disk, and `local_manifest` did not come back
    `unavailable` (a run with no local manifest still exits `0`, downloads
    everything, and is the failure mode byte counts alone will not catch).
- Mock shapes mirror live `hk4e_global` responses (Sept 2026): same wrappers,
  node names, int-or-string sizes, `url_prefix + "/" + id` joining. `getBuild?tag=<unknown>`
  returns `retcode -202` to exercise the local-build fallback.
- Fixture: 2 files / 3 chunks with fixed bytes (`build_fixture`); chunk fetches are
  logged (`mock.chunk_hits()`) so reuse/resume tests assert exact download sets.
  `MockOpts::local_manifest` adds a **local** build whose chunks sit at
  *different* offsets — the shape a version upgrade leaves behind, and the only
  way to exercise the salvage order.
- Unit tests sit next to the code they cover, one `mod tests` per module: pure
  phases only (plan, purge classification, audio scan file, `config.ini`, URL and
  query building, REPORT/SUMMARY formatting, flag validation, manifest filter,
  local chunk map).
- `tests/cli_contract.rs` runs the built binary to pin the process-level contract
  (every arg error → `1`, `--help` → `0`); it never touches a game directory.
- To add a case: extend `MockOpts` (deprecated list, audio dirs) or the fixture,
  then write one `#[tokio::test]` calling `repair::run` and asserting
  `(summary, exit code)` + on-disk state. Use `jobs=1` when asserting chunk order.

## How it keeps disk usage low

1. Files-cleanup (`--purge-before`/`--purge-after`): delete everything not in the live manifest.
2. Skip intact files (size + full MD5): 0 bytes for healthy files.
3. **In-place repair (the default)**: the target is patched *where it lies*, so
   a repair costs the file's **growth** — **0** for a same-size repair — and
   only the chunks that are wrong are written. A file that is already correct
   where it belongs is *kept*, not rewritten; a file whose content shifted is
   salvaged from its own bytes where the write order allows it.
4. Chunk-level repair: only missing/corrupt chunks fetched; unchanged chunks copied from
   verified local slices (hash-gated, network fallback).
5. `--repair-mode temp` is the escape hatch: the shipped per-file temp + atomic
   move, whose transient cost is one `_tmp` beside the file — no game duplicate,
   no blob store, no zip staging.
6. Post-phase: deprecated files, audio cache→res move, optional purge, `config.ini` bump.

The disk arithmetic, in one line: the peak extra free space a repair needs is
`io_threads × largest file` under `--repair-mode temp`, and
`io_threads × largest growth` under the default in-place path — which is zero
for the common case (a corrupt file of the right size). `PLAN est_extra_bytes`
reports the figure for the plan in front of you, and `SUMMARY kept_bytes`
reports what the run did *not* spend.

v1 gap: expected set = latest Sophon manifest paths only (no dispatcher union, no
SDK/WPF/plugin zips — game launches without them). Chunk-repair only: no hdiff,
no 7z legacy path, no predownload, no speed limiter. Reruns are idempotent
(intact skipped, a half-patched file finished by re-verifying it).
