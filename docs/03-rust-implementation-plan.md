# Rust implementation plan — `girpr` v1 (Genshin low-disk repair patcher)

Spec: `docs/02-repair-chunk-spec.md`. Findings: `docs/01-findings-and-verdict.md`.

## 1. Crate layout (single binary, std + tokio)

```text
Cargo.toml            # clap, tokio, reqwest, serde, prost, md5, zstd, tracing, anyhow, walkdir, regex
src/main.rs           # CLI (clap), logging init, exit codes, orchestration
src/biz.rs            # biz/channel/launcher/game-id mapping + config.ini channel values
src/hyp.rs            # HoYoPlay client: getGameConfigs/getGameBranches/getBuild/getDeprecated
src/sophon.rs         # prost chunk-manifest structs, manifest fetch+verify+parse, filtering
src/repair.rs         # work-list build, per-file repair, cleanup, purge-extra, config.ini write
src/util.rs           # md5 helpers, retry, atomic rename, pre/post temp sweep
```

No build.rs (prost `derive` only, no protoc). No SQLite, no GUI, no speed limiter.

## 2. CLI

```text
girpr --game-path <DIR> --biz <hk4e_cn|hk4e_global|hk4e_bilibili>
      [--audio <zh-cn,en-us,ja-jp,ko-kr>]...   (repeatable; default: keep current selection)
      [--io-threads <N, default 4>]            (concurrent FILES; chunks sequential/file)
      [--purge-extra] [--purge-before]         (extra-file purge after / also before)
      [--check-only] [--dry-run]               (verify / print actions, write nothing)
      [--log-level info|debug] [--json-summary]
```

Exit codes: 0 ok (incl. check-only clean), 1 usage, 2 metadata/network, 3 verify/write,
4 check-only found damage. Automation: all progress on stderr (tracing), final `SUMMARY key=value`
(or JSON) on stdout.

## 3. Key flows → code mapping

| Spec step | Code |
|---|---|
| Pre-clean temps | `util::sweep_temps(game_dir)` (0) |
| Local version + audio | `repair::read_local_version`, audio scan file read/write (1) |
| 4 metadata calls | `hyp::get_game_config/branches/build/deprecated` (2) |
| Manifest fetch/verify/parse | `sophon::fetch_manifest(manifest_dl, meta)` → zstd decode → md5 check → `prost::Message::decode` (3) |
| Work list + reuse map | `repair::build_plan(latest, local)` keyed by path, chunk md5+size (4) |
| Per-file repair | `repair::repair_file()` — skip check → open `_tmp` → sequential chunks: slice-reuse (md5-gated) else `GET chunk_prefix/id` → zstd stream → write at offset → final md5 → rename (5) |
| Post deletes/config/purge | `repair::post_phase()` (6), summary (7) |

Concurrency: `tokio::Semaphore(io_threads)` over files; one `reqwest::Client` with
~`io_threads*4` pool; per-chunk `GET` sequential inside a file (HDD-safe). MD5 streaming
(512 KiB bufs). Retries: 5× linear backoff on manifest/chunk/file ops.

## 4. Testing

`cargo test`: pure unit tests — config.ini parse/bump, audio-lang mapping, manifest filter,
reuse-map build, expected-set/purge classification, temp-sweep matcher. Network paths covered by
`--dry-run`/`--check-only` against a fixture manifest (no live-game CI dependency).

## 5. v1 limits (documented, not bugs)

Chunk-repair only; no hdiff fast-update, no 7z legacy path, no SDK/WPF/plugin, no dispatcher
persistent revisions, inside-`game_dir` manifest cache never written, `chunk/` reuse dir not retained.
