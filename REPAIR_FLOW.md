# `girpr` repair process — visual flow atlas

From **process launch** to **process exit**, in 22 diagrams across five zoom levels.
Every node name is a real function/type in this repository; `path:line` anchors point at
the current checkout. The binary is a thin shim — `src/main.rs` — and all behavior lives
in the library, coordinated by `repair::run` in `src/repair/mod.rs`, which owns **only**
phase order and the exit-code contract.

> **How to read this atlas**
> - **L0** — the operating-system view (process, runtime, exit status).
> - **L1** — the seven top-level stages, split into a read-only **plan half** and a
>   write-capable **execute half**.
> - **L2** — one diagram per phase (metadata, manifest, plan, check, repair, post,
>   purge, persistent, ceilings, reporting).
> - **L3** — the per-file algorithm: write-mode resolution, in-place tiers, temp mode,
>   streamed chunk download.
> - **L4** — the supporting value/decision leaves and resume state machines.

Colour legend used in flowcharts:

| Class | Meaning |
|---|---|
| `plan` (blue) | read-only; no install mutation |
| `exec` (orange) | write-capable; may mutate the install |
| `net` (purple) | network / server call |
| `data` (green) | data artifact handed between phases |
| `exit` (red) | a terminal outcome with an exit code |

---

## Index

| # | Level | Diagram | Type |
|---|---|---|---|
| 1 | L0 | Process lifecycle — launch to exit | flowchart |
| 2 | L0 | Exit-code decision map | flowchart |
| 3 | L1 | The whole run in one picture: two halves, seven stages | flowchart |
| 4 | L1 | Data hand-off between phases | flowchart |
| 5 | L2 | `plan_phase` in full | flowchart |
| 6 | L2 | `metadata::fetch` — two HYP waves plus local reads | sequenceDiagram |
| 7 | L2 | `fetch_plan_network` — five-way fan-out | flowchart |
| 8 | L2 | `build_and_assert_plan` — plan, scope, scan, ceilings | flowchart |
| 9 | L2 | `execute_phase` in full | flowchart |
| 10 | L2 | Files-cleanup: `plan_purge` → `apply_purge` | flowchart |
| 11 | L2 | Resource ceilings: refusal vs hard stop | stateDiagram-v2 |
| 12 | L2 | The `--with-persistent` tier | flowchart |
| 13 | L2 | Post-phase and the final `config.ini` bump | flowchart |
| 14 | L2 | Stdout record stream and the log channel | sequenceDiagram |
| 15 | L3 | Per-file repair: complete decision flow | flowchart |
| 16 | L3 | In-place path internals and the two tiers | flowchart |
| 17 | L3 | Temp path internals and resume-by-length | flowchart |
| 18 | L3 | Streamed chunk fetch, decode and digest gate | sequenceDiagram |
| 19 | L4 | Write-mode resolution | flowchart |
| 20 | L4 | Module / crate dependency map | flowchart |
| 21 | L4 | Resume state machine — same-size upgrade per mode | stateDiagram-v2 |
| 22 | L3 | `--check-only` verification detail | flowchart |

---

## 1. L0 — Process lifecycle: launch to exit

`src/main.rs:15` (`main`) is the shim: parse → tracing → config → runtime → `run` → `SUMMARY` → flush → exit code.

```mermaid
flowchart TD
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20

    START(["OS launches girpr.exe"]) --> PARSE["parse_args() — src/main.rs:154<br/>Args::try_parse — clap, one flat struct, no subcommands"]
    PARSE --> PC{"clap result"}
    PC -->|"help or version"| H0["print help or version<br/>girpr_cli::clap_error_code → 0"]
    PC -->|any other parse error| E1A["e.print() + process::exit(1)<br/>NO log file, NO SUMMARY"]
    PC -->|Ok Args| LOG["init_tracing(...) — src/logging.rs:102<br/>LogPlan::resolve → stderr layer + optional file layer<br/>try_init + install_guard"]
    LOG --> ANN["log_file_announcement → eprintln if caller named a file"]
    ANN --> CTX{"args.into_ctx() — src/cli.rs:302<br/>jobs 1..=1024, audio aliases and none, glob compile,<br/>ceilings, repair mode, backoff"}
    CTX -->|Err| E1B["ERROR ... + flush_log() → exit 1"]
    CTX -->|Ok RunCtx| RT["worker_threads(max(2,avail)+1)<br/>tokio multi-thread runtime build"]
    RT -->|build Err| E1C["exit 1"]
    RT -->|Ok| MRUN["runtime.block_on(main::run(ctx)) — src/main.rs:106"]
    MRUN --> PIPE["girpr::run = repair::run(ctx) — src/repair/mod.rs:104<br/>PhaseTimer('run') + Summary::default()"]
    PIPE --> PLAN["plan_phase — read-only<br/>see diagram 5"]
    PLAN --> EXEC["execute_phase — write-capable<br/>see diagram 9"]
    PLAN -.->|Limit or print-plan| EARLY["PlanOutcome::Finished(RunExit)"]
    PIPE --> RES{"repair::run result"}
    EARLY --> RES
    EXEC --> RES
    RES -->|Ok (summary, code)| SUM["emit(summary_record(summary, code.code()))<br/>THE ONLY SUMMARY call site — stdout + log"]
    RES -->|Err RunFailure| FATAL["tracing::error + eprintln FATAL<br/>NO SUMMARY on fatal error"]
    SUM --> FLUSH["flush_log() — drop WorkerGuard, drain queue"]
    FATAL --> FLUSH
    FLUSH --> EXIT["exit_code(code) → ExitCode::from(process_status())"]
    H0 --> FIN(["OS process exit"])
    E1A --> FIN
    E1B --> FIN
    E1C --> FIN
    EXIT --> FIN

    class PARSE,LOG,CTX,RT plan
    class MRUN,PIPE,PLAN,EXEC exec
    class SUM,FLUSH,EXIT data
    class E1A,E1B,E1C,H0,FATAL,EARLY exit
```

**Contracts pinned here**
- Clap errors are remapped: `DisplayHelp`/`DisplayVersion` → `0`, everything else → `1`
  (`crates/cli/src/lib.rs:16`, `clap_error_code`). Parse errors happen *before* `init_tracing`,
  so they create no log file.
- Logging is infallible: an unopenable log file is a `WARN`, never a failed run.
  `RUST_LOG` *adds* to the console filter; it cannot lower `girpr`'s own level.
- `SUMMARY` is emitted exactly once, by `main`, for every non-fatal return. A fatal
  `Err(RunFailure)` prints `FATAL` instead and has **no** `SUMMARY`.

---

## 2. L0 — Exit-code decision map

The table is `define_exit_table!` in `src/config.rs:28` using the shared macro in
`crates/exit/src/lib.rs`. Codes are contiguous from zero and compile-time pinned.

```mermaid
flowchart TD
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20

    START(["any outcome"]) --> Q0{"fatal Err(RunFailure)?"}
    Q0 -->|yes| Q1{"failure.exit_code"}
    Q1 -->|Usage| C1["1 — usage<br/>bad scope selection, bad local config.ini,<br/>non-CHUNK download mode, scope/audio conflict,<br/>unopenable log is NOT one"]
    Q1 -->|Metadata| C2["2 — metadata/network<br/>HYP or Sophon failure, manifest/biz/tag mismatch,<br/>malformed manifest path or chunk range at plan time,<br/>dispatcher query failure"]
    Q1 -->|Write| C3F["3 — write, FATAL form<br/>mkdir failure, live lock held, plan-out write,<br/>purge or post write failure — NO SUMMARY"]
    C1 --> NF["stdout: FATAL only<br/>log: FATAL only"]
    C2 --> NF
    C3F --> NF

    Q0 -->|no — Ok summary,code| Q2{"which RunExit?"}
    Q2 -->|Success| C0["0 — normal completion<br/>--print-plan early return<br/>clean --check-only<br/>post-phase OK"]
    Q2 -->|CheckFailed| C4["4 — --check-only found damage<br/>presence, length or MD5 mismatch; writes nothing"]
    Q2 -->|Write| C3O["3 — per-file failures counted<br/>SUMMARY failed greater than 0<br/>post-phase SKIPPED, config.ini not bumped"]
    Q2 -->|Limit| C5["5 — a manually requested ceiling bound the run<br/>pre-flight refusal: nothing written<br/>or runtime hard stop: SUMMARY stopped=flag"]
    C0 --> YS["stdout: exactly one SUMMARY with exit=code"]
    C4 --> YS
    C3O --> YS
    C5 --> YS

    NF --> FIN(["ExitCode::from(process_status())"])
    YS --> FIN

    class C0 data
    class C1,C2,C3F,C3O,C4,C5,NF,YS exit
```

**Subtleties**
- `5` is deliberately not `1`: the API has to be fetched first, so whether a limit trips
  depends on the server's answer.
- A failed file **outranks** a limit: `failed_file_outranks_outcome` (`mod.rs:1554`)
  converts `Limit` to `Write` (3) and skips the post phase.
- Pre-flight refusal writes nothing at all; a runtime hard stop leaves the completed
  prefix in place and `SUMMARY stopped=<flag>` says which ceiling bound.

---

## 3. L1 — The whole run in one picture: two halves, seven stages

Authoritative order from `src/repair/mod.rs:32-44`. The plan half decides everything and
touches nothing; every write and delete lives in the execute half.

```mermaid
flowchart TD
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c
    classDef net fill:#f3e5f5,stroke:#6a1b9a,color:#4a148c

    A["1. resolve game_dir root — canonicalize only,<br/>NO create_dir_all yet"] --> B["2. metadata::fetch → Meta<br/>local config.ini + ignore + blacklist + audio<br/>and HYP getGameConfigs/getGameBranches/getBuild"]
    B --> B2["emit REPORT — local vs latest + API fields"]
    B2 --> C["3. manifests + local reuse map + stale lists<br/>tokio::try_join! of five futures"]
    C --> D["4. build work list: build_plan + purge plan<br/>+ optional scan + download estimate"]
    D --> L{"any ceiling exceeded?"}
    L -->|yes| X5["exit 5 — nothing written<br/>SUMMARY limited=1"]
    D --> PP{"--print-plan?"}
    PP -->|yes| X0["emit PLAN + --plan-out JSON<br/>exit 0 — install untouched"]
    PP -->|no| E1["5. create_dir_all + acquire single-writer lock<br/>+ explicit audio scan file"]
    E1 --> E2["stale-file sweep → --purge-before cleanup<br/>frees room for the repair"]
    E2 --> E3{"--check-only?"}
    E3 -->|yes| X4["verify_plan: presence, length, MD5<br/>exit 0 clean or 4 damaged — writes nothing"]
    E3 -->|no| E4{"--purge-only?"}
    E4 -->|yes| X6["post-phase only, no repair loop<br/>exit 0 — config.ini not bumped"]
    E4 -->|no| E5["6a. persistent full-file repair, if --with-persistent"]
    E5 --> E6["6. repair loop — at most jobs files in parallel,<br/>chunks sequential per file"]
    E6 --> F{"any failed file?"}
    F -->|yes| X3["exit 3 — SUMMARY failed greater than 0<br/>post-phase skipped so config.ini cannot advance"]
    F -->|no| E7["7. post-phase: audio move → purge-after<br/>→ persistent markers → config.ini bump LAST"]
    E7 --> X0B["SUMMARY → exit 0<br/>or exit 5 if a delete ceiling tripped"]

    subgraph HALF1["PLAN HALF — read-only except --plan-out"]
        A;B;C;D;L;PP
    end
    subgraph HALF2["EXECUTE HALF — every write and delete"]
        E1;E2;E3;E4;E5;E6;E7
    end

    class A,B,D plan
    class B2,C net
    class E1,E2,E5,E6,E7 exec
    class X5,X0,X4,X6,X3,X0B exit
```

The same seven stages, as a compact linear spine (for a quick mental model):

```
launch ─ parse ─ trace ─ ctx ─ runtime ─┐
                                       ├─ PLAN:  root → metadata → REPORT → manifest ─┐
                                       │                                             ├─ plan → scan → ceilings → PLAN
                                       ├─ EXEC:  mkdir → lock → audio → stale → purge-before ─┐
                                       │                                                     ├─ check | purge-only | persistent
                                       │                                                     ├─ repair loop → failed? → 3
                                       └─ finish ─ post → config.ini → SUMMARY ─ flush ─ exit ─┘
```

---

## 4. L1 — Data hand-off between phases

No `RepairSession` or `RepairContext` type exists. The state holders are `RunCtx`,
`Meta`, `PlanPhase`, `PlanWork`, `Summary` and the `Arc`-shared plan values.

```mermaid
flowchart LR
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100

    CTX["RunCtx<br/>config.rs:124"] --> RUN["repair::run<br/>owns Summary for both halves"]
    RUN --> SUM[("Summary<br/>atomics + two Meters<br/>+ SummaryFlags")]
    RUN --> PP["plan_phase"]
    PP --> META["Meta<br/>hyp, config, branch, latest,<br/>latest_build, local_build, ignore,<br/>blacklist, audio, dispatch"]
    PP --> NET["per_manifest, LocalChunkMap,<br/>stale list, patch deletes,<br/>persistent manifests"]
    PP --> PW["PlanWork<br/>plan, purge_plan, purge_keys,<br/>stale, scan, estimate, disk, persistent"]
    PW --> POP["PlanPhase<br/>mod.rs:135<br/>the immutable hand-off bag"]
    POP --> EX["execute_phase"]
    META --> EX
    SUM --> EX
    EX --> RP[("Arc&lt;RepairPlan&gt;<br/>PlannedFile rel,size,md5,<br/>url_prefix,chunks")]
    EX --> SR[("Arc&lt;ScanReport&gt;<br/>Verdict per file,<br/>plan fingerprint")]
    EX --> PS[("Arc&lt;PersistentState&gt;<br/>dispatch, plan, markers,<br/>suppression, status")]
    EX --> PK[("PurgeKeys + PurgePlan")]
    RP --> FILE["file::repair_all"]
    SR --> FILE
    PS --> PEX["persistent_exec::repair"]
    PK --> POST["post::run"]
    FILE --> SUM
    PEX --> SUM
    POST --> SUM
    SUM --> MAIN["main::run emits exactly one SUMMARY"]

    class CTX,META,NET,PW,POP data
    class PP,RP,SR,PS,PK plan
    class EX,FILE,PEX,POST,MAIN exec
```

**The one shared counter object** (`app/report/src/summary.rs:48`): `files_total`,
`files_skipped`, `files_repaired`, `files_failed`, `download: Meter`,
`deleted_extra_bytes`, `write: Meter`, `flags`, `stopped_by`, fail-fast/abort flags,
RAM reservation, `files_inplace`, `files_temp`, `kept_bytes`, check damage tally.
Every phase writes into the same `Summary`; only `main` renders it.

---

## 5. L2 — `plan_phase` in full

`src/repair/mod.rs:199-292`. Steps P0–P5. Timed phases: `metadata`, `manifest`,
`plan`, `scan`, `publish` (`app/report/src/phase.rs:11`).

```mermaid
flowchart TD
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef net fill:#f3e5f5,stroke:#6a1b9a,color:#4a148c
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c

    P0["P0 — canonicalize game_dir once, build PathRoot<br/>NO create_dir_all (mod.rs:209)"] --> P1["P1 fetch_and_report (mod.rs:318)<br/>PhaseTimer metadata"]
    P1 --> P1A["metadata::fetch — see diagram 6"]
    P1A --> P1B{"Err?"}
    P1B -->|usage or metadata error| X1["RunFailure → FATAL exit 1 or 2"]
    P1B -->|Ok| P1C["if --with-persistent: persistent::query dispatcher"]
    P1C --> P1D["emit REPORT — report_record + emit<br/>mod.rs:332 — the first stdout record"]
    P1D --> P2["P2 fetch_plan_network (mod.rs:369)<br/>PhaseTimer manifest"]
    P2 --> FAN["tokio::try_join! of five futures<br/>see diagram 7"]
    FAN --> FANERR{"fatal future failed?"}
    FANERR -->|yes| X2["RunFailure::metadata → exit 2"]
    FANERR -->|no| STALE["build_stale_list: deprecated union patch-deletes<br/>minus live manifest paths"]
    STALE --> P3["P3 build_and_assert_plan (mod.rs:500)<br/>PhaseTimer plan — see diagram 8"]
    P3 --> LIM{"within_limits → check_preflight hits?"}
    LIM -->|hit| X5["PlanOutcome::Finished(Limit)<br/>exit 5, NOTHING written"]
    LIM -->|none| P4["P4 publish_plan (mod.rs:988)<br/>emit PLAN + write --plan-out JSON atomically"]
    P4 --> PO{"ctx.print_plan?"}
    PO -->|yes| X0["PlanOutcome::Finished(Success)<br/>exit 0, install untouched"]
    PO -->|no| READY["PlanOutcome::Ready(Box&lt;PlanPhase&gt;)<br/>mod.rs:135 → execute_phase"]

    class P0,P1,P1D,P3,P4 plan
    class P1A,P1C,P2,FAN net
    class STALE,READY data
    class X1,X2,X5,X0 exit
```

---

## 6. L2 — `metadata::fetch`: two HYP waves plus local reads

`src/repair/phases/metadata.rs:240`. Produces `Meta` (`metadata.rs:31`). Read-only.
The deprecated list is **not** fetched here (it belongs to the plan-phase fan-out).

```mermaid
sequenceDiagram
    autonumber
    participant PP as plan_phase
    participant MD as metadata::fetch
    participant BLK as spawn_blocking_traced (local disk)
    participant H as HypClient (HYP JSON API)

    PP->>MD: fetch(ctx, game_dir)
    MD->>H: hyp_client_for(ctx) — flag > GIRPR_*_BASE > production
    MD->>BLK: util::read_game_version(config.ini)
    Note over BLK: absent = fresh install; unreadable = RunFailure::usage (1)

    rect rgb(243,229,245)
    Note over MD,H: WAVE 1 — settle_primary(config, branch)
    MD->>H: game_config() → get_node launch_configs
    MD->>H: game_branch() → get_node game_branches
    H-->>MD: GameConfig + GameBranchPackage
    MD->>MD: validate biz_mismatch → metadata error (2)
    MD->>MD: validate default_download_mode == DOWNLOAD_MODE_CHUNK → usage (1)
    MD->>MD: clean_config_dir: audio scan/cache/res dirs → '/' form, containment checked
    end

    MD->>BLK: read_ignore_categories + read_blacklist + resolve_effective_audio
    Note over BLK: explicit --audio wins; invalid audio under active scope that hides scan file → usage (1)

    rect rgb(243,229,245)
    Note over MD,H: WAVE 2 — settle_primary(latest build, local build)
    MD->>H: chunk_build(branch, None) → getBuild latest
    MD->>H: chunk_build(branch, Some(local_version)) → getBuild local (best effort)
    H-->>MD: ChunkBuild(latest) + Option&lt;ChunkBuild(local)&gt;
    MD->>MD: latest tag_mismatch → metadata error (2)
    MD->>MD: local tag mismatch or failure → None, warn, "full-fetch fallback"
    end

    MD-->>PP: Meta { hyp, config, branch, latest, latest_build,<br/>local_build, local_version, ignore, blacklist, audio, dispatch: None }
    PP->>PP: emit REPORT record (first stdout line)
```

---

## 7. L2 — `fetch_plan_network`: five-way fan-out

`src/repair/mod.rs:369-447`. One `reqwest::Client` and one `ManifestBudget`
(`crates` semaphore, `app/fetch/src/sophon/mod.rs`), so joined fan-outs cannot double
the peak decoded-manifest memory.

```mermaid
flowchart TD
    classDef net fill:#f3e5f5,stroke:#6a1b9a,color:#4a148c
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c

    START["fetch_plan_network — PhaseTimer manifest"] --> J["tokio::try_join! — one ManifestBudget<br/>fatal future failure cancels the rest"]
    J --> A["1. manifest::fetch_latest<br/>sophon::select_manifests → fetch + zstd + MD5 + protobuf<br/>MAX_MANIFEST_CONCURRENCY = clamp(jobs,1,4)<br/>OrderedResults restores selection order"]
    J --> B["2. manifest::local_reuse_map<br/>local build's manifests, best effort<br/>→ sophon::build_local_chunk_map(path → FileReuse)"]
    J --> C["3. load_deprecated → HypClient::deprecated_files<br/>best effort, per-launcher channel/sub_channel"]
    J --> D["4. sophon::fetch_patch_deletes<br/>best effort, POST patch_build → patch manifests<br/>union ALL delete tags, manifest_download only"]
    J --> E["5. persistent::fetch_manifests<br/>only under --with-persistent<br/>fetch_all_manifests + parse_manifest — fatal on failure"]

    A --> ERR{"first failure in selection order?"}
    ERR -->|yes| X2["RunFailure::metadata → exit 2"]
    ERR -->|no| ST["build_stale_list — deprecated union patch-deletes<br/>minus live manifest paths"]
    B --> LM["LocalChunkMap in Arc — empty means<br/>SUMMARY local_manifest=unavailable<br/>run still correct, all chunks downloaded"]
    C --> ST
    D --> ST
    E --> PS["Option&lt;PersistentState&gt; plan input"]
    ST --> OUT["plan inputs: per_manifest, local_map,<br/>stale, persistent"]
    LM --> OUT
    PS --> OUT

    class A,B,C,D,E net
    class ST,LM,PS,OUT data
    class X2 exit
```

**Manifest decode gate** (`app/fetch/src/sophon/stream.rs:66`): resolve `url_prefix`,
validate blob id (reject `:`, `/`, `\`, `..`, `?`, `#`), GET with 5 attempts and jittered
backoff, check decoded size (64 MiB cap) and MD5 (case-insensitive), then
`prost::Message::decode`. The upstream field is misspelled `chuncks` (tag 1) — byte-critical,
never "fix" it (`crates/proto/src/lib.rs`).

---

## 8. L2 — `build_and_assert_plan`: plan, scope, scan, ceilings

`src/repair/mod.rs:500-698`. Nothing is written here.

```mermaid
flowchart TD
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c

    IN["PlanInputs: per_manifest, meta, blacklist, scope,<br/>purge flags, limits, scan, persistent"] --> PS["persistent::build + base_md5_map + read_base_hash_source<br/>scope_decision → Proceed or Suppressed<br/>collect persistent expected_rels"]
    PS --> PW["plan_work_and_purge (mod.rs:698)<br/>PhaseTimer plan"]
    PW --> BP["girpr_repair_plan::build_plan<br/>drop folders, clean_rel, blacklist, scope filter,<br/>dedup first-manifest-wins, sort chunks by offset,<br/>validate_chunk_ranges (exit 2), drop empty chunks,<br/>warn on gaps/overlaps, sort by rel, fingerprint"]
    BP --> SCOPE1["check_scope_selection for unpurged runs<br/>an --include matching nothing → usage exit 1"]
    SCOPE1 --> WALK["one spawn_blocking:<br/>PurgeKeys::new(plan, keep_set, blacklist)<br/>keys.extend_expected(persistent expected)<br/>purge::plan_purge — see diagram 10"]
    WALK --> SCOPE2["post-walk check_scope_selection for purge-enabled runs"]
    SCOPE2 --> DISK["if --print-plan or --plan-out:<br/>spawn DiskPlan::new (one symlink_metadata per file)<br/>est_extra_bytes = peak × jobs"]
    DISK --> SCAN{"--scan?"}
    SCAN -->|yes| SC["scan::scan_plan — one read per planned file<br/>buffer_unordered(jobs), whole-file MD5 + dest_ok<br/>+ source_ok, damaged_chunks, plan fingerprint<br/>see diagram 15 step 1"]
    SCAN -->|no| EST["estimate_download: conservative all-chunks upper bound"]
    SC --> EST["DownloadEstimate { by_file, bytes }<br/>+ PlanSummaryValues"]
    EST --> LIM["within_limits → Limits::check_preflight<br/>see diagram 11"]
    LIM --> HIT{"LimitHit?"}
    HIT -->|Exceeded or IncompletePurge| X5["exit 5 — nothing written"]
    HIT -->|none| AWAIT["await disk_handle, return PlanWork"]

    class IN,BP,PS,WALK plan
    class DISK,SC,EST,AWAIT data
    class SCOPE1,SCOPE2,X5 exit
```

`--scan` is the only way to make the estimate exact: it resolves the write path
(temp / discard / in-place) and the salvage direction, so chunks that direction
overwrites are not counted as downloads. Without it the figure is an upper bound.

---

## 9. L2 — `execute_phase` in full

`src/repair/mod.rs:1069-1218`. Exact internal numbering 5 → 7. Everything below may write.

```mermaid
flowchart TD
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20

    START["execute_phase(ctx, PlanPhase, Summary)"] --> PREP["5. prepare_install (mod.rs:1229)<br/>create_dir_all unless readonly, re-canonicalize, rebuild PathRoot<br/>failure → RunFailure::write, FATAL exit 3, NO SUMMARY"]
    PREP --> LOCK["acquire_lock (mod.rs:1263)<br/>girpr_lock::takes_lock = !readonly and !check_only and !print_plan<br/>OS advisory lock on {temp}/girpr-{key:016x}.lock<br/>held by another run → FATAL exit 3"]
    LOCK --> AUDIO["5.1 write_explicit_audio — explicit --audio only, non-readonly"]
    AUDIO --> STALE["5.15 run_stale_sweep → post::delete_stale<br/>best effort, skipped by --check-only"]
    STALE --> REL["5.2a persistent_exec::relocate_stranded_av<br/>stranded .pck/.usm/.cuepoint back to StreamingAssets<br/>BEFORE purge-before"]
    REL --> PURGE["5.2 run_purge_before → purge::apply_purge<br/>PhaseTimer purge-before, skipped by --check-only"]
    PURGE --> CHK{"--check-only?"}
    CHK -->|yes| CHKOUT["5.5 run_check_only → check::verify_plan<br/>+ verify_persistent if flag<br/>0 clean / 4 damaged — writes nothing"]
    CHK -->|no| PO{"--purge-only?"}
    PO -->|yes| POOUT["5.6 run_purge_only → post::run then finish<br/>no repair loop, no config.ini bump"]
    PO -->|no| PEX["6a install_run_limits + persistent_exec::repair<br/>full-file hotfix downloads if --with-persistent"]
    PEX --> REP["6. run_repair → file::repair_all<br/>PhaseTimer repair, jobs file window<br/>ProgressGuard ticker"]
    REP --> FLAGS["apply_run_flags — record SUMMARY stopped=flag<br/>failed_file_outranks_limit: failed greater than 0 → exit 3"]
    FLAGS --> FAILED{"failed file?"}
    FAILED -->|yes| X3["skip post-phase so config.ini cannot advance"]
    FAILED -->|no| POST["7. run_post → post::run → finish<br/>PostOutcome Ok → 0, DeleteCeiling → 5"]
    POST --> END["_lock dropped here; execute_phase returns (Summary, RunExit)"]

    class PREP,LOCK,AUDIO,STALE,REL,PURGE,PEX,REP,POST exec
    class CHKOUT,POOUT plan
    class X3,CHKOUT exit
    class FLAGS,END data
```

---

## 10. L2 — Files-cleanup: `plan_purge` → `apply_purge`

`src/repair/purge.rs`, values from `app/repair-plan/src/purge_values.rs`. One cleanup
function called twice (before and after repair), planned fully before the first delete.

```mermaid
flowchart TD
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20

    START["plan_purge(game_dir, PurgeKeys, PathScope)<br/>purge.rs:132"] --> WALK["walk_and_classify — WalkDir follow_links(false)<br/>parallel stat of candidates, at most 8 scoped threads<br/>collect files, all_dirs, kept_dirs, hidden, errors"]
    WALK --> RULES{"classify_purge_path_visible — ordered rules"}
    RULES -->|"0. out of path scope"| KEEP["KEEP — out-of-scope paths do not exist"]
    RULES -->|"1. in live manifest expected set"| KEEP
    RULES -->|"2. server keep: config.ini, girpr.lock, pkg_version"| KEEP
    RULES -->|"3. launcher blacklist"| KEEP
    RULES -->|"4. audio_lang_* or Audio_*_pkg_version"| KEEP
    RULES -->|"5. verification-state markers<br/>*_persist, *_revision, ChannelName, ScriptVersion,<br/>PatchDone, base_res_version_hash, cache_versions*, ctable*"| KEEP
    RULES -->|"everything else"| DEL["PURGE candidate<br/>temps, ScreenShot, logs, exe, versions_remote,<br/>VideoAssets/*.tmp, unused audio"]
    KEEP --> FIN["finish_plan: sort by path, dirs = walked minus kept ancestors,<br/>deepest-first, sum total_bytes"]
    DEL --> FIN
    FIN --> PP[("PurgePlan { files, dirs, total_bytes, hidden, errors }")]
    PP --> CEIL{"--max-delete-bytes active?"}
    CEIL -->|incomplete plan: errors greater than 0| REFUSE["exit 5 — refuse to delete against a lower bound"]
    CEIL -->|"greater than N"| REFUSE
    CEIL -->|ok| APPLY["apply_purge(plan, dry_run)<br/>dry run: log 'would purge extra ...', return 0<br/>real: remove_file on scoped threads,<br/>then remove_emptied_dirs deepest-first"]
    APPLY --> BYTES["summary.add_deleted_bytes — shared with stale sweep"]

    class WALK,RULES,FIN plan
    class PP,BYTES data
    class APPLY exec
    class REFUSE exec
```

The `purge-after` pass re-plans so repair-created `*_tmp` files are counted, and in a dry
run it lists only candidates the before pass did not name.

---

## 11. L2 — Resource ceilings: refusal vs hard stop

`src/repair/scope/{mod,limits,ledger}.rs`. Two separate tiers: **path scope** answers
*which paths exist*; **resource ceilings** answer *how much work*.

```mermaid
stateDiagram-v2
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c
    [*] --> Parsed: --max-* flags parsed into Limits
    Parsed --> Filtered: build_plan applies --include / --exclude first
    Filtered --> Preflight: Limits::check_preflight(Preflight)
    Preflight --> Refused: Exceeded or IncompletePurge
    note right of Refused
        exit 5, SUMMARY limited=1
        nothing written: no dir created,
        no _tmp opened, no file deleted
    end note
    Refused --> [*]

    Preflight --> Armed: no hit -> install_run_limits
    Armed --> Live: reserve_* charges meter BEFORE the write
    Live --> Settled: settle_* validates; live and settled figures
    Live --> Stopped: live or settled crosses a ceiling
    Stopped --> Reported: apply_run_flags -> flags.stopped = flag
    note right of Reported
        exit 5, SUMMARY stopped=--max-write-bytes etc.
        work already done stays in place;
        next run re-verifies and continues
    end note
    Reported --> [*]
    Settled --> Done: post-phase delete ceiling re-checked by check_delete
    Done --> [*]
```

**The meters** (`app/report/src/ledger.rs:128`): a byte is *live* the moment it is
committed and *settled* once validated. `--max-download-bytes` and `--max-write-bytes`
bound live traffic/disk; `--max-verified-*` bound the settled figures. Each pair is
reported in `SUMMARY` (`download_bytes` / `download_bytes_spent`, `write_bytes` /
`write_bytes_spent`) so the difference is visible waste.

---

## 12. L2 — The `--with-persistent` tier

`src/repair/persistent.rs` (plan half: query, verify, plan, guard) and
`src/repair/persistent_exec.rs` (execute half: prerequisites and full-file bytes).
Without the flag, no dispatcher or persistent request is made and all outputs stay byte-identical.

```mermaid
flowchart TD
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef net fill:#f3e5f5,stroke:#6a1b9a,color:#4a148c
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c

    FLAG{"--with-persistent?"}
    FLAG -->|no| INERT["no dispatcher request, no markers,<br/>PLAN and SUMMARY byte-identical"]
    FLAG -->|yes| Q["persistent::query — dispatcher query with latest,<br/>unwrap key and seed from release metadata,<br/>select host pinned per biz — failure → exit 2"]
    Q --> FM["fetch_all_manifests + parse_manifest<br/>NDJSON res/data/silence, size + MD5 gate<br/>failure → exit 2"]
    FM --> BUILD["persistent::build → girpr_persistent_plan::build_plan<br/>placement rule, force-persistent rule, dedup, sort,<br/>render_markers, scope_decision"]
    BUILD --> SCOPE{"scope decision"}
    SCOPE -->|marker or mirror out of scope| U1["usage error exit 1"]
    SCOPE -->|hotfix out of scope| SUP["suppress whole marker block<br/>SUMMARY persistent=suppressed"]
    SCOPE -->|proceed| STATE[("PersistentState<br/>dispatch, plan, markers,<br/>suppression, status")]
    STATE --> KEYS["extend PurgeKeys with persistent expected_rels"]
    STATE --> EXEC

    EXEC["execute: relocate stranded AV before purge-before"] --> PREREQ["prerequisites: create Persistent and StreamingAssets,<br/>remove stale ctable.dat and DXSETUP,<br/>copy ctable.dat → ctable_streaming.dat if different"]
    PREREQ --> DL["persistent_exec::repair — per asset, buffer_unordered(jobs)<br/>hotfix_is_intact → skip;<br/>else download_hotfix: primary URL, alternate only on 404,<br/>stream to NamedTempFile, XXH64 or MD5 gate,<br/>atomic publish, re-read to verify"]
    DL --> FAIL{"asset failed?"}
    FAIL -->|yes| X3["files_failed → exit 3, skip post,<br/>publish NO marker block"]
    FAIL -->|no| POST["post::publish_persistent: mirrors FIRST,<br/>markers LAST, atomic writes"]

    class Q,FM,BUILD plan
    class EXEC,PREREQ,DL,POST exec
    class STATE,KEYS data
    class U1,X3 exit
```

**Post order under the flag**: audio move → cleanup-after → mirrors → full marker
block → `config.ini` bump last. A failed persistent file, a delete ceiling, or a
suppressed run withholds the whole marker block and therefore the version claim.
`--check-only` under the flag also verifies every persistent file, every mirror body
and every marker value; any mismatch is damage (exit 4).

---

## 13. L2 — Post-phase and the final `config.ini` bump

`src/repair/phases/post.rs:55-154`. Load-bearing order: audio move → purge-after →
publish persistent → bump `config.ini` **last**.

```mermaid
flowchart TD
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c

    START["post::run — PhaseTimer post<br/>skipped by --check-only and --print-plan"] --> AUDIO["1. move_audio_cache: cache/**/* → res/RELATIVE_PATH<br/>rename, then NamedTempFile copy fallback,<br/>remove source only after success — best effort"]
    AUDIO --> PURGE{"--purge-after or --purge-only?"}
    PURGE -->|no| PUB
    PURGE -->|yes| PLAN["2. purge_after_pass: re-plan with PurgeKeys<br/>walk sees repair-created *_tmp files"]
    PLAN --> DELCEIL{"Limits::check_delete?"}
    DELCEIL -->|tripped| X5["PostOutcome::DeleteCeiling<br/>exit 5, SUMMARY stopped=--max-delete-bytes<br/>that pass deleted nothing"]
    DELCEIL -->|ok| DRY{"dry run?"}
    DRY -->|yes| LIST["dedup against before_purge, log only"]
    DRY -->|no| APPLY["purge::apply_purge → deleted_extra_bytes"]
    LIST --> PUB
    APPLY --> PUB
    PUB["3. publish_persistent — only under flag,<br/>not purge-only, not suppressed<br/>refresh_markers_after_repair re-reads base hash,<br/>then mirrors first and markers last"]
    PUB --> PUBERR{"publish error?"}
    PUBERR -->|yes| X3["RunFailure::write → FATAL exit 3<br/>version bump never reached"]
    PUBERR -->|no| DRY2{"--dry-run?"}
    DRY2 -->|yes| NOBUMP1["log 'not bumping config.ini'<br/>PostOutcome::DeletedNothing"]
    DRY2 -->|no| SCOPE{"path scope active?"}
    SCOPE -->|yes| NOBUMP2["warn — partial install, config_bumped=0"]
    SCOPE -->|no| PO{"--purge-only?"}
    PO -->|yes| NOBUMP3["warn — no bytes repaired, config_bumped=0"]
    PO -->|no| BUMP["4. util::write_config_ini(game_dir, latest, biz,<br/>channel_tuple) — section-preserving surgical update<br/>THE LAST WRITE of the run"]
    BUMP --> OK["PostOutcome::Ok → exit 0"]
    X5 --> FIN(["finish(outcome) → RunExit"])
    NOBUMP1 --> FIN
    NOBUMP2 --> FIN
    NOBUMP3 --> FIN
    X3 --> FIN
    OK --> FIN

    class AUDIO,PLAN,APPLY,PUB,BUMP exec
    class LIST data
    class X5,X3 exit
```

The `config.ini` update forces `game_version`, `game_biz`, `channel`, `sub_channel`, `cps`;
preserves `sdk_version`; drops comments; sorts keys; creates `[General]` if missing.

---

## 14. L2 — Stdout record stream and the log channel

`app/report/src/{record,emit,summary}.rs`. Stdout is a record stream with a fixed order;
tracing diagnostics go to stderr and the log file, never to stdout.

```mermaid
sequenceDiagram
    autonumber
    participant MAIN as main::run
    participant PP as plan_phase
    participant EX as execute_phase
    participant FILE as file::repair_all / check::verify_plan
    participant EM as report::emit
    participant OUT as stdout
    participant LOG as log file + stderr

    PP->>EM: report_record(local_version, latest_version, biz, exe,<br/>download_mode, branch, package_id, build_id, audio_langs, diff_tags)
    EM->>LOG: tracing::info!(rendered line) FIRST
    EM->>OUT: REPORT ...
    opt --print-plan or --plan-out
        PP->>EM: plan_record(files, est_download_bytes, est_write_bytes,<br/>est_extra_bytes, [scan_bytes], purge_files, purge_bytes,<br/>max_chunk_bytes, scoped, [persistent_*])
        EM->>LOG: tracing::info!(line)
        EM->>OUT: PLAN ...
    end
    EX->>FILE: start ProgressGuard (single ticker, every 10 s)
    loop while files complete
        FILE->>EM: progress_record(done, total, skipped, repaired, failed, download_bytes)
        EM->>OUT: PROGRESS ...
    end
    FILE->>FILE: Drop ProgressGuard — abort ticker
    EX-->>MAIN: Ok((Summary, RunExit)) or Err(RunFailure)
    alt non-fatal
        MAIN->>EM: summary_record(summary, code)
        EM->>LOG: tracing::info!(line)
        EM->>OUT: SUMMARY total=... skipped=... repaired=... failed=...<br/>download_bytes=... deleted_extra_bytes=... exit=... [conditional keys]
    else fatal
        MAIN->>LOG: tracing::error!(fatal) + eprintln!("FATAL ...")
        Note over OUT: no SUMMARY line at all
    end
    MAIN->>LOG: flush_log() — drop WorkerGuard, drain queue
```

Conditional keys appear **only** when their flag is active, in a fixed tail order:
`limited, scoped, config_bumped, plan_files, plan_download_bytes, download_bytes_spent,
write_bytes, write_bytes_spent, repair_mode, inplace, temp, kept_bytes, ram_bytes,
damaged_chunks, purge_errors, purge_hidden, stopped, local_manifest, persistent,
persistent_files, persistent_repaired, persistent_failed, persistent_base_hash`.

---

## 15. L3 — Per-file repair: complete decision flow

`src/repair/file/mod.rs:344` (`repair_all`) → `join_file_tasks` → `file_task` →
`repair_one_file` (`mod.rs:641`). At most `jobs` files in flight; chunks strictly
sequential per file. Each whole task runs with `Handle::block_on` on Tokio's blocking pool.

```mermaid
flowchart TD
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c

    START["repair_one_file(idx)"] --> STOP{"stopped_by set?"}
    STOP -->|yes| RSTOP["Repaired::Stopped"]
    STOP -->|no| VALID["validate_chunk_ranges — defensive second gate"]
    VALID --> PATH["root.join_validated or contained_join<br/>create_dir_all(parent), build {path}_tmp as OsString"]
    PATH --> MODE["resolve_mode(requested, path) → Mode Inplace or Temp<br/>see diagram 19"]
    MODE --> TRIES{"retry_state — MAX_ATTEMPTS=5<br/>or 1 under --fail-fast"}
    TRIES --> ATTEMPT{"state.inplace?"}
    ATTEMPT -->|yes| INP["inplace::attempt_inplace<br/>see diagram 16"]
    ATTEMPT -->|no| TMP["temp::attempt_temp<br/>see diagram 17"]
    INP --> OUT{"Inplace outcome"}
    OUT -->|Skipped| RSKIP["count as verified, no download"]
    OUT -->|DryRun| RDRY["Repaired::DryRun"]
    OUT -->|Repaired ChunkStats| ROK["fold counters into Summary"]
    OUT -->|Unavailable reason| FLIP["permanently flip to Mode::Temp<br/>continue WITHOUT spending an attempt"]
    OUT -->|FinalDigestMismatch| FATALTRY["Fatal — no backoff,<br/>deterministic plan inconsistency"]
    OUT -->|other error| RETRY["retry with jittered backoff"]
    TMP --> TOUT{"Temp outcome"}
    TOUT -->|Skipped or DryRun| RSKIP
    TOUT -->|Repaired ChunkStats| ROK
    TOUT -->|FinalDigestMismatch| FATALTRY
    TOUT -->|other error| RETRY
    FLIP --> ATTEMPT
    RETRY --> TRIES
    RSTOP --> FOLD["file_task folds Repaired into Summary<br/>JoinError or panic also increments files_failed"]
    RSKIP --> FOLD
    RDRY --> FOLD
    ROK --> FOLD
    FATALTRY --> FOLD
    FOLD --> NEXT["next file — buffer_unordered window"]

    class VALID,PATH,MODE plan
    class INP,TMP,RETRY,FLIP exec
    class RSKIP,ROK,RDRY,RSTOP,FOLD,NEXT data
    class FATALTRY exit
```

**The invariant**: every byte written lands in a region that was **already wrong at the
moment of the write**, and the whole-file MD5 decides whether the file is good. A
downloaded chunk cannot be hash-gated before it lands, so the gate is structural, not hopeful.

---

## 16. L3 — In-place path internals and the two tiers

`src/repair/file/inplace.rs:90`. No `_tmp` is created; peak extra disk is the file's
**growth**. Two tiers produce the same file with different reuse/direction.

```mermaid
flowchart TD
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20

    START["attempt_inplace_inner"] --> SRC["reuse::sources_for → Source offsets in the SAME file<br/>scan_ranges → dest_ranges, source_ranges, source_owner"]
    SRC --> OPEN["mode::open_inplace — open_rw(read+write, no truncate),<br/>clear_readonly on failure; still failing → Unavailable"]
    OPEN --> SCAN["open_and_scan — ONE read via util::scan_reader<br/>whole-file MD5 + dest_ok + source_ok"]
    SCAN --> SKIP{"len == size and md5 == planned md5?"}
    SKIP -->|yes| DONE["Inplace::Skipped — count as verified"]
    SKIP -->|no| DISCARD{"--repair-mode discard?"}
    DISCARD -->|yes| DIS["discard_target: delete and recreate the target,<br/>use Scan::absent — AFTER the skip check"]
    DISCARD -->|no| BAD["plan_bad_chunks: chunks where !dest_ok[i]"]
    DIS --> BAD
    BAD --> TRIM["trim_dead_tail — eager set_len only when no plan source<br/>reaches past target size; else deferred to the end"]
    TRIM --> TIER{"choose_tier: moving payload vs --max-ram-bytes"}
    TIER -->|"fits (or size 0: corruption repair)"| STAGED["Tier::Staged<br/>stage_sources re-hashes each captured slice (TOCTOU guard)<br/>RAM payload, extra disk 0, write ASCENDING"]
    TIER -->|"does not fit, or budget 0"| ORDERED["Tier::Ordered<br/>salvage_order_refs: source survives iff<br/>ascending r >= o, descending r <= o<br/>ties go ascending; extra disk 0, RAM 0"]
    STAGED --> PATCH["patch_chunks<br/>staged → write_at<br/>disjoint downloads → window_download_chunks (window 4)<br/>otherwise write_downloaded_chunk<br/>copy_within for ordered salvage<br/>zero_fill gaps, then set_len(file.size)"]
    ORDERED --> PATCH
    PATCH --> VERIFY["verify_and_retry → girpr_chunk_verify::verify_in_place<br/>fresh full read"]
    VERIFY --> VM{"md5 matches?"}
    VM -->|yes| REPAIRED["repaired_inplace — log chunks_total, kept, reused,<br/>downloaded, download_bytes, final_md5"]
    VM -->|no| REDO["run_still_bad_chunks: re-read to name culprit,<br/>redo ONLY those chunks, then read_after_redo"]
    REDO --> RM{"still mismatched?"}
    RM -->|yes| FAIL["FinalDigestMismatch — counted as failed, continue run"]
    RM -->|no| REPAIRED

    class OPEN,SCAN,TRIM plan
    class DIS,BAD,STAGED,ORDERED,PATCH,VERIFY,REDO exec
    class DONE,REPAIRED data
    class FAIL exit
```

**Resume**: in-place resumes by **verification**. There is no artefact — the scan finds
the chunks already correct where they belong, so the next run rewrites only the rest and
cannot inherit a corrupt prefix. No rollback is needed.

---

## 17. L3 — Temp path internals and resume-by-length

`src/repair/file/temp.rs:28` (`attempt_temp`). The target is **never modified** until the
final atomic rename. Peak extra disk is the whole file.

```mermaid
flowchart TD
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c

    START["attempt_temp"] --> SKIP{"reusable_verdict from plan scan (length, mtime,<br/>2 s age guard) or fresh md5_file_digest matches?"}
    SKIP -->|yes| DONE["Skipped"]
    SKIP -->|no| OPEN["open {path}_tmp with create(true), truncate(FALSE)<br/>have = current length; clamp to file.size if longer"]
    OPEN --> LOOP{"for each chunk in offset order"}
    LOOP --> CASE{"tmp_len >= chunk end?"}
    CASE -->|yes| RESUMED["ChunkOutcome::Resumed — keep the completed prefix"]
    CASE -->|no| REUSE{"local same-file reuse slice intact?<br/>capture_slice → md5_digest matches uncompressed_md5"}
    REUSE -->|yes| REUSED["ChunkOutcome::Reused — copy_reuse_pieces<br/>charging reserve_write per piece"]
    REUSE -->|no| DOWN["ChunkOutcome::Download — write_downloaded_chunk<br/>→ sophon::stream_chunk_to, see diagram 18"]
    DOWN --> FAIL{"chunk failed its digest?"}
    FAIL -->|yes| ROLL["rollback_failed_chunk: set_len(chunk.offset)<br/>reset tmp_len — keep the resume point honest"]
    FAIL -->|no| NEXTCH["advance tmp_len by written bytes"]
    RESUMED --> LOOP
    REUSED --> LOOP
    NEXTCH --> LOOP
    ROLL --> ABORT["abandon this attempt, retry whole file"]
    LOOP -->|all chunks done| PROMOTE["promote_tmp: stat len == file.size,<br/>md5_file_digest must match,<br/>else delete _tmp and bail"]
    PROMOTE --> RENAME["std::fs::rename(tmp → final) — ATOMIC overwrite"]
    RENAME --> OK["Repaired — whole-file MD5 gate passed"]

    class OPEN plan
    class DOWN,ROLL,ABORT,PROMOTE,RENAME exec
    class RESUMED,REUSED,NEXTCH,OK data
    class FAIL exit
```

**Resume**: temp resumes by **length**. A chunk that fails its digest is rolled back to
its offset, so every retained byte was digest-checked when written and the resume point
is honest. A target that cannot be opened in place is written this way, loudly
(`Inplace::Unavailable` → permanent `Mode::Temp`, no attempt spent).

---

## 18. L3 — Streamed chunk fetch, decode and digest gate

`app/fetch/src/sophon/stream.rs:443` (`stream_chunk_to`). Only the network receive is
async; zstd decode and sink writes are synchronous, so the caller polls from a blocking
context. The chunk is never resident as a whole.

```mermaid
sequenceDiagram
    autonumber
    participant W as writer (inplace or temp)
    participant SF as stream_chunk_to
    participant T as try_stream_chunk (5 attempts)
    participant H as HashingWriter
    participant Z as zstd Decoder (streaming)
    participant N as CDN URL prefix / chunk id

    W->>SF: ChunkFetch { url_prefix, id, expected size, consumed, sink }
    SF->>SF: validate id, join_url(prefix, id), reserve compressed_size
    loop retry on 408 / 429 / 5xx only; 404 is final
        T->>N: GET {chunk_prefix}/{id}
        N-->>T: response body (zstd-compressed chunk)
        T->>H: wrap sink
        loop each received piece
            H->>H: enforce decoded-size ceiling BEFORE forwarding
            H->>Z: write compressed piece
            Z-->>H: decoded pieces
            H->>W: sink.write(decoded piece) — writes at chunk offset immediately
            W->>W: reserve_write per piece
            H->>H: hash only accepted bytes
        end
        T->>Z: dec.finish() — require a complete zstd frame
        T->>T: consumed += compressed bytes across ALL attempts
        alt digest.bytes == expected and digest.md5 matches uncompressed_md5
            T-->>SF: ChunkDigest ok
        else content mismatch
            T-->>SF: StreamError::Content → Fatal
        end
    end
    SF->>W: settle_reservation(real consumed) — not the reserved upper bound
    W->>W: note_write — settle the settled counter
    Note over W: digest known only after the LAST byte landed;<br/>the whole-file MD5 is the real gate
```

**Retry policy** (`app/fetch/src/retry.rs`): jittered exponential backoff, 5 attempts.
Only `408`/`429`/`5xx` are retried; a 404 on a chunk is asked once because blob ids come
from `getBuild` and are published atomically.

---

## 19. L4 — Write-mode resolution

`app/repair-policy/src/lib.rs:81` (`resolve_mode`) — one rule, shared by the writer, the
disk estimate and the download estimate so they cannot disagree.

```mermaid
flowchart TD
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef exec fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20

    START["--repair-mode = auto | inplace | temp | discard<br/>default auto"] --> Q{"requested == Temp?"}
    Q -->|yes| T["Mode::Temp — assemble in {path}_tmp,<br/>atomic rename, target never modified"]
    Q -->|no| META{"symlink_metadata(path)"}
    META -->|Ok and is_file| I["Mode::Inplace — write only wrong bytes,<br/>no _tmp, peak extra disk = file growth"]
    META -->|Err (missing)| I
    META -->|Ok but dir or symlink| T
    I --> D{"requested == Discard?"}
    D -->|yes| DISC["resolve to Inplace, but discard_if_requested<br/>deletes and recreates the target first:<br/>0 extra disk when already full size, full download"]
    D -->|no| I2["normal in-place: corruption repair writes 0 bytes moved;<br/>version upgrade uses Staged or Ordered tier"]
    T --> OPEN{"open_inplace actually succeeds?"}
    OPEN -->|no| T2["Inplace::Unavailable → permanent Mode::Temp<br/>loudly, no attempt spent"]
    OPEN -->|yes| I

    class START,Q,META,D plan
    class T,DISC,I2,T2 exec
    class I data
```

`src/repair/file/mode.rs` holds only the **open helpers** (`open_inplace`, `open_rw`,
`clear_readonly`); the *policy* lives in `app/repair-policy`. `SUMMARY` carries a
conditional `repair_mode=`, `inplace=`, `temp=`, `kept_bytes=` group only when
`--repair-mode` was passed.

---

## 20. L4 — Module / crate dependency map

The binary depends on acyclic leaves; no report key or exit code is shared between
programs. `girpr` is the only default workspace member; `gircap`, `girsync`,
`sled2redb`, `cache-json` are dev tools.

```mermaid
flowchart TB
    classDef bin fill:#fff3e0,stroke:#ef6c00,color:#e65100
    classDef facade fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef leaf fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef shared fill:#f3e5f5,stroke:#6a1b9a,color:#4a148c

    MAIN["src/main.rs — thin shim"] --> LIB["src/lib.rs — facade"]
    LIB --> CLI["src/cli.rs + config.rs + logging.rs + biz.rs"]
    LIB --> REPAIR["src/repair/mod.rs — coordinator only"]
    REPAIR --> PH["phases: metadata, manifest, check, post"]
    REPAIR --> FILE["file: mod, inplace, temp, mode"]
    REPAIR --> SCAN["scan.rs + reuse.rs"]
    REPAIR --> PURGE["purge.rs"]
    REPAIR --> PERS["persistent.rs + persistent_exec.rs"]
    REPAIR --> SCOPE["scope: limits, ledger, report"]

    CLI --> POL["app/repair-policy"]
    FILE --> POL
    SCAN --> POL
    PH --> PLAN["app/repair-plan"]
    SCAN --> PLAN
    PURGE --> PLAN
    PERS --> PPLAN["app/persistent-plan"]
    FILE --> VERIFY["app/chunk-verify"]
    SCAN --> VERIFY
    PH --> FETCH["app/fetch (hyp + sophon)"]
    PERS --> FETCH
    PH --> AUDIO["app/audio"]
    REPAIR --> AUDIO
    FILE --> UPATH["app/util-path"]
    PERS --> UPATH
    PH --> INI["app/util-ini"]
    REPAIR --> LOCK["app/lock"]
    SCOPE --> REPORT["app/report"]

    PLAN --> ID["crates/chunk-identity + chunk-cost"]
    FETCH --> PROTO["crates/proto"]
    FETCH --> DIGEST["crates/digest + retry"]
    PLAN --> FILTER["crates/filter + path-shape"]
    CLI --> CCLI["crates/cli + exit + log + time"]
    REPORT --> RCORE["crates/report-core"]

    class MAIN,LIB bin
    class REPAIR,PH,FILE,SCAN,PURGE,PERS,SCOPE facade
    class CLI,POL,PLAN,PPLAN,VERIFY,FETCH,AUDIO,UPATH,INI,LOCK,REPORT leaf
    class ID,PROTO,DIGEST,FILTER,CCLI,RCORE shared
```

---

## 21. L4 — Resume state machine — same install, next run

The two write modes resume differently, each in the way that suits its write direction.
Every resume is safe because the whole-file MD5 gates the version claim.

```mermaid
stateDiagram-v2
    [*] --> Interrupted: a run stops mid-flight (ceiling, failure, kill)

    Interrupted --> ReScanI: mode auto or inplace — no artefact
    ReScanI --> IntactI: next run re-reads each planned file once
    IntactI --> RewriteI: chunks correct where they belong stay
    RewriteI --> GateI: rewrite only the wrong chunks
    GateI --> ConfigBump: final whole-file MD5 passes

    Interrupted --> KeepPrefixT: mode temp — open {path}_tmp without truncation
    KeepPrefixT --> ResumeT: tmp_len >= chunk end means already done
    ResumeT --> DigestFailT: a chunk fails its decoded MD5
    DigestFailT --> RollbackT: set_len(chunk.offset), reset tmp_len
    RollbackT --> ResumeT
    ResumeT --> PromoteT: len == size and md5(tmp) == planned md5
    PromoteT --> AtomicRenameT: std::fs::rename tmp to final
    AtomicRenameT --> ConfigBump

    ConfigBump --> [*]: config.ini advanced only if EVERY planned file verified

    note right of ReScanI
        in-place: no rollback, a corrupt prefix
        cannot be inherited; next scan finds it
    end note
    note right of RollbackT
        temp: every retained byte was digest-checked
        when written; the resume point stays honest
    end note
```

**Rerun is safe** overall: intact files are skipped, and the rest are re-verified.
`--check-only` performs only step 1 for every file (presence, length, MD5) and writes
nothing; a malformed manifest chunk range is refused at plan time (exit 2) so it cannot
report such a file intact.

---

## 22. L3 — `--check-only` verification detail

`src/repair/phases/check.rs:60` (`verify_plan`) and `check.rs:278` (`verify_persistent`).
The check phase is an early return in the execute half, before any download or write.

```mermaid
flowchart TD
    classDef plan fill:#e3f2fd,stroke:#1565c0,color:#0d47a1
    classDef data fill:#e8f5e9,stroke:#2e7d32,color:#1b5e20
    classDef exit fill:#ffebee,stroke:#c62828,color:#b71c1c

    START["run_check_only (mod.rs:1411)"] --> TIMER["PhaseTimer check + ProgressGuard ticker"]
    TIMER --> LOOP["for each planned file, spawn_blocking,<br/>buffer_unordered(jobs) — resolve root.join_validated"]
    LOOP --> SRC{"--scan verdict available<br/>and plan fingerprint matches?"}
    SRC -->|yes| USEV["DigestSource::Scanned — reuse the plan-phase MD5,<br/>but re-stat presence and length"]
    SRC -->|no| USER["DigestSource::Read — util::md5::md5_file_digest on current bytes"]
    USEV --> GATE{"check_one: presence → length → MD5"}
    USER --> GATE
    BAD["CheckFailure: PathEscaped, Unreadable,<br/>Absent, Size, Md5 or Read"]
    GATE -->|"absent or unreadable"| BAD
    GATE -->|"size differs"| BAD
    GATE -->|"MD5 differs"| BAD
    GATE -->|all match| GOOD["CheckOutcome::clean"]
    BAD --> FOLD["record_check: files_failed += 1"]
    GOOD --> FOLD2["record_check: files_skipped += 1"]
    FOLD --> WARN["one single warn site emits every failure shape"]
    FOLD2 --> PERS{"--with-persistent?"}
    WARN --> PERS
    PERS -->|yes| PV["verify_persistent: each asset via hotfix_is_intact,<br/>each mirror via verify_manifest_bytes,<br/>each marker byte-for-byte against refresh_markers_after_repair"]
    PERS -->|no| TALLY
    PV --> TALLY["publish_check_damaged_chunks — measured zero differs from never ran"]
    TALLY --> Q{"any bad item?"}
    Q -->|no| X0["RunExit::Success — exit 0, writes nothing"]
    Q -->|yes| X4["RunExit::CheckFailed — exit 4, writes nothing"]

    class TIMER,LOOP,SRC,USEV,USER,GATE plan
    class FOLD,FOLD2,WARN,PERS,PV,TALLY data
    class X0,X4 exit
```

**Note**: with `--scan`, the plan-phase scan already hashed each file, so a same-length
content change *after* the scan can be reported intact. A fresh check of current bytes
must omit `--scan`.

---

## One-line summary of the whole atlas

```
main shim → run() → [PLAN: metadata → REPORT → 5-way manifest fan-out → build_plan
→ scan → ceilings → PLAN] → [EXECUTE: mkdir → lock → audio → stale sweep → purge-before
→ check | purge-only | persistent → per-file repair (in-place tiers or temp resume)
→ post: audio → purge-after → mirrors → markers → config.ini LAST] → SUMMARY → flush → exit code
```

Sources: `README.md` (action graph and machine contracts), `docs/02-repair-chunk-spec.md`,
`docs/03-repair-rust-implementation-plan.md`, and the Rust sources under `src/`,
`app/`, and `crates/` at the paths and line numbers named in each diagram.
