# TO-FIX — prerequisites for the artifact bridge

---

## How to use this file

You are implementing this file. It is a **scratch work order**, not a document
in the project.

**Three process rules. Follow them literally.**

1. **Do not reference this file, its path, or its existence anywhere.** Not in a
   code comment, not in a doc comment, not in a README row, not in a commit
   message, not in a `docs/` cross-reference, not in a test name, not in a
   "see also". **This file is deleted once the work lands.** Anything you write
   that points at it becomes a dangling reference. If you catch yourself wanting
   to explain *why* a change was made by pointing at this file, explain the
   reason in the code's own terms instead — that is what the surrounding code
   does, and it is why the explanation survives after this file is gone.

2. **Turn around after every commit.** Implement exactly one commit, then
   **stop and hand back to the user** with a short report: the commit hash,
   what changed and why, the test/verification output, and anything that
   surprised you or that you had to decide yourself. **Do not start the next
   commit** until the user has responded. An auditor reviews each commit in
   isolation; chaining them defeats the point and makes a revert impossible.

3. **If something here is wrong, stop and say so** rather than improvising
   around it. This file was written from a review, not from the code, and the
   line numbers below will drift as you edit.

---

## What you are working in

One Cargo workspace, two crates:

| crate | path | what it is |
|---|---|---|
| `girpr` | repo root | The product. A Genshin Impact low-disk repair patcher. Downloads chunks from HoYoPlay/Sophon CDNs and patches an install forward, reusing unchanged byte ranges from the file already on disk. |
| `gircap` | `tools/capture/` | A **dev-only** tool. Fetches from the live HoYoPlay + Sophon APIs and writes a self-contained, gitignored **artifact bundle**: a real `game/` fixture, every API body and manifest, every chunk blob, and ground truth about what a correct patch costs. |
| `girsync` | `tools/sync/` | A sibling tool, unrelated except as a **style reference** — its `--output`/`--log-file` flags and its report/record model are what commits 2, 3 and 7 copy. |

**The thing being unblocked.** `girpr` currently has no end-to-end test against
real bytes. `tests/repair_offline.rs` builds a synthetic 2-file fixture and calls
`repair::run(ctx)` **in-process**; `tests/cli_contract.rs` is the only file that
spawns the real binary, and it never reaches the API. So nothing proves that
`--include`, `--hyp-base` etc. actually reach the HTTP client through the real
flag-parsing path. `docs/06-artifact-bridge-spec.md` designs the missing test: a
loopback server that replays one `gircap` bundle, plus a driver that runs the
real `girpr` binary against it and asserts it patches each file to the new
version. **Writing that test is not this work order's job.** These nine commits
are its prerequisites.

**Two bundles already exist** under `artifacts/` (gitignored), both for
`hk4e_global` 7.0.0 → 7.1.0:

```
artifacts/hk4e_global/7.0.0__7.1.0/20261015T092615Z
artifacts/hk4e_global/7.0.0__7.1.0/20261015T111345Z
```

Both pass `gircap verify --artifact <dir>` with 90/90 and exit `0`. They predate
commits 5 and 6, so they carry neither `build_id` on chunk references nor
`api/error_shapes.json`. Both facts are back-compatible and both bundles still
verify — but they are no longer what the tool writes today, so **re-capturing is
a legitimate step in commits 5 and 6 if you want the acceptance to be against
current output.** See the capture note under *Ground rules*.

---

## Evidence (measured — do not re-measure, do not hit the network)

Taken from the live `hk4e_global` endpoints on 2026-10-16. Commits 4 and 6 rest
on this, so it is recorded here rather than re-derived.

**The two route families answer failure differently.** This is the single most
important fact in this file.

**Blob routes** (`autopatchhk.yuanshen.com`, `Server: AliyunOSS`) answer
**`404`, never 200-with-a-code**:

| request | status | content-type | body |
|---|---|---|---|
| valid manifest / chunk | 200 | `application/octet-stream` | bytes |
| bogus id | **404** | `application/xml` | `<Error><Code>NoSuchKey</Code>…` |
| garbage / short id | **404** | `application/xml` | same |
| unknown `build_id` in the path | **404** | `application/xml` | same |
| valid id under the **other** build's prefix | **404** | `application/xml` | same |
| completely unknown path | **404** | `application/xml` | same |

**JSON routes** (`sg-hyp-api.hoyoverse.com`, `sg-downloader-api.hoyoverse.com`)
answer **`200`**, with the failure in `retcode`:

| request | body |
|---|---|
| valid | `{"retcode":0,"message":"OK",…}` |
| `getBuild?tag=<unknown>` | `{"data":null,"message":"not found","retcode":-202}` |
| `getBuild` bad `package_id` | `{"data":null,"message":"invalid params","retcode":-201}` |
| `getGameConfigs` bad `game_ids[]` | `{"data":null,"message":"invalid params","retcode":-1001}` |
| `getGameBranches` bad `game_ids[]` | `{"retcode":0,…,"data":{"game_branches":[]}}` — **not an error** |

Two consequences:

- **`RequestId` in the OSS error body is volatile.** Three probes, three
  different ids, and the body length varies because it echoes `Key`. A replayer
  cannot be byte-exact there and does not need to be: `girpr` calls
  `.error_for_status()` and never reads the body, so **status and content-type
  are the only two things it can observe.**
- `tools/capture/docs/00-spec.md` §3.7 already records "same chunk id under the
  other build's prefix → 404". It is now confirmed, and it holds for manifests
  too. Commit 9 promotes it.

**Measured timings**, from running the real binary against a bundle over
loopback, with one chunk 404'd:

| | elapsed | server-side requests | exit |
|---|---|---|---|
| with `--fail-fast` | 17.6 s | 5 | 3 |
| without | **88.5 s** | 25 | 3 |

Both bundles record **49 requests, every one `status: 200`** — a bundle contains
no not-found evidence at all today. That is what commit 6 fixes.

---

## Ground rules

- **Workspace layout is fixed.** `Cargo.toml` has
  `members = [".", "tools/sync", "tools/capture"]`. Do not add a member, do not
  add a `src/bin/`, do not add a dependency in either direction. `gircap` must
  never depend on or invoke `girpr`.
- **`gircap capture` is available to you**, and commits 5 and 6 change the
  artifact, so capturing fresh output is a reasonable way to verify them — it is
  the only command that uses the network, and it is not forbidden here. What
  matters is not *whether* but *when and where*, and the full policy lives in
  `tools/capture/README.md` under **Artifact bundles**. In short:
  - **scratch by default** — `--out <a temp dir>` writes the identical tree
    somewhere disposable, so the repo stays clean while you iterate. Prefer this;
  - **promote to `artifacts/` only when the bundle is good and you expect to
    re-use it**, i.e. while `gircap` is not still changing shape;
  - **`verify --artifact <dir>` must exit `0`** before anything depends on a
    bundle, new or old;
  - **do not re-capture for the sake of it.** A capture pulls tens of megabytes
    of blobs. If a unit test can answer the question, the test answers the
    question — the capture is for when you genuinely need the bundle the tool
    writes *today*.
  If you cannot run a capture at all (no network), say so in your hand-back
  rather than skipping the verification silently.
- **Current test baseline** (`cargo test -p girpr`): **76** lib unit tests,
  **50** in `tests/repair_offline.rs`, **4** in `tests/cli_contract.rs`. Commits
  2–4 change these numbers; record your own baseline first rather than trusting
  these.
- **Match the surrounding code.** Doc comments carry real explanations in this
  codebase — write the same kind, not a restatement of the code. Preserve the
  existing `NOTE(...)` / `V1-SIMPLIFICATION(...)` conventions.
- **Do not "fix" unrelated things.** In particular: `tools/capture/src/report.rs`
  hand-writes JSON key/value pairs and documents why —
  `serde_json`'s `preserve_order` feature is **workspace-unified**, so enabling
  it in one crate silently reorders the JSON another tool emits. Leave that
  alone. `girpr` sends no `Range` headers and does not need to.
- Tests are expected in every code commit, in the same style as the neighbours:
  unit tests next to the code, `#[tokio::test]` for async, `tests/cli_contract.rs`
  for anything process-level.

**Commit order is 1 → 9. Do not reorder.**

---

# Commit 1 — `girpr: point the doc link at the method that exists`

**Why.** `RepairPlan::est_download_bytes_with` does not exist. The name is
stale, and it is referenced in three places, one of them an intra-doc link a
reader would try to click.

**Files**

- `src/repair/scope/limits.rs:35` — the doc comment links to
  `RepairPlan::est_download_bytes_with`. The real method is
  `RepairPlan::est_download_bytes` (`src/repair/plan/mod.rs:55`).
- `README.md:43` — the entry-point table's key-symbol column for
  `src/repair/plan/mod.rs` lists `RepairPlan::est_download_bytes_with`.
- `docs/05-scope-limitation-implementation-plan.md:167` — references it too.

**Verify**

```
cargo doc --no-deps -p girpr
```

No warning naming `est_download_bytes_with`. (The crate has 21 other doc
warnings — private-module links and bare-URL notes. Leave them; only this one
names a symbol that does not exist.)

```
cargo test -p girpr
cargo clippy --workspace --all-targets
```

---

# Commit 2 — `girpr: add --log-file`

**Why.** `logging::init_tracing` computes the log path and `src/main.rs:14`
throws it away. The log file lands in `<exe-dir>/logs/girpr_<timestamp>.log` —
i.e. `target/debug/logs/` — opened **append**, shared by every run that has ever
existed and never cleaned. No external process can tell which file belongs to a
given run. Both sibling tools already ship this flag; the asymmetry is
gratuitous.

**Files and specifics**

- `src/logging.rs` — change the signature to
  `init_tracing(level: &str, log_file: Option<&Path>) -> PathBuf`. With `None`,
  behaviour is unchanged (`<exe-dir>/logs/…`). Keep `init_tracing` infallible:
  a log file that cannot be opened is already a stderr warning, and that must
  stay true.
- `src/cli.rs` — add `--log-file <FILE>`, beside `--log-level`. In the help text,
  say that the file layer captures `TRACE` and above **regardless of
  `--log-level`**, so the reader does not think they also need `--log-level
  trace` to get chunk-level lines.
- `src/config.rs` — `RunCtx.log_file: Option<PathBuf>`, populated in
  `Args::into_ctx` (`src/cli.rs:158`).
- `src/main.rs` — pass it through. Keep the existing
  `tracing::info!("log file: …")` line, and **additionally** `eprintln!` the path
  when the flag was given explicitly, so a wrapper that forgot to capture the log
  still learns where it went.
- `tests/cli_contract.rs` — add `"--log-file"` to the array in
  `help_text_lists_every_documented_flag`. Add a case proving a bad parent
  directory **degrades to a warning and still exits normally** — `init_tracing` is
  infallible by design and a test must hold it there.
- `README.md` — a row in the flag table (around line 228, next to `--log-level`).

**Verify**

```
cargo test -p girpr
```

plus, by hand, that `girpr --game-path <tmp> --biz hk4e_global --log-file
<tmp>/x.log --check-only --dry-run` creates `<tmp>/x.log`, and that omitting the
flag still writes under `target/debug/logs/`.

---

# Commit 3 — `girpr: replace --json-summary with --output <text|json>`

**Why.** Two defects, one fix. Today `--json-summary` makes only `REPORT` and
`SUMMARY` JSON, and `--print-plan --json-summary` emits a **bare text `PLAN …
line followed by a JSON twin** — so the JSON stream is not JSON. And `"scoped"`
is a **boolean** in the PLAN object and an **integer** in the SUMMARY object: one
key name, two types, same stream.

**This is the one breaking change in the batch.** `--json-summary` is a
documented public flag. All 12 references, found by grep:

| site | what |
|---|---|
| `src/cli.rs:134` | `pub json_summary: bool` |
| `src/cli.rs:189` | `json_summary: self.json_summary` |
| `src/config.rs:95` | `pub json_summary: bool` |
| `src/main.rs:16`, `:41` | plumbing + `format_summary_line` |
| `src/report.rs:151`, `:268`, `:316` | doc comments naming the plane |
| `src/repair/mod.rs:106`, `:281` | REPORT format, and the duplicate PLAN emit |
| `tests/cli_contract.rs:164` | help-list assertion |
| `tests/repair_offline.rs:77` | `json_summary: false` in a ctx builder |
| `README.md:99`, `:227` | prose + flag table |
| `docs/02:81`, `docs/03:53`, `docs/03:65`, `docs/04:49`, `docs/05:105` | prose |

**Model it on `girsync`** — read `tools/sync/src/report/mod.rs` (the
`OutputFormat` enum at line 49) and `tools/sync/src/report/record.rs` (the
`Record` type) before you start, and follow their shape:

- `OutputFormat` is a `clap::ValueEnum`, `#[default] Text`, declared
  `global = true`.
- A `Record` is an event name plus ordered fields, **held once and rendered
  twice**, so the two modes cannot drift.
- JSON is NDJSON — one object per line, no enclosing array. `event` is the text
  label lowercased: `REPORT`→`report`, `PROGRESS`→`progress`, `PLAN`→`plan`,
  `SUMMARY`→`summary`.
- Within an object, keys are **names**; read by name, never by position.

**Files and specifics**

- `src/cli.rs` — replace the flag with
  `pub output: OutputFormat` (`#[arg(long, value_enum, default_value_t, global = true)]`).
- `src/config.rs` — `RunCtx.output: OutputFormat`. The `json: bool` parameter on
  `report::format_report_line` / `format_summary_line` becomes the format.
- `src/report.rs` — introduce `Record` and route **all four** stdout lines
  through it. `emit()` keeps its current behaviour of printing the rendered line
  *and* `tracing::info!`-ing it, so whichever format is active the log file
  carries the same data plane a stdout parser would see.
- `src/repair/mod.rs:279-292` — **delete the duplicate `PLAN` emit.** Today it
  prints the text line and then, if JSON, a second line. With one record per
  line that becomes impossible.
- **Normalise the `scoped` collision:** `plan` emits `"scoped": true` (bool)
  while `summary` emits `"scoped":1` (int). Emit **both** as `1`/`0` — the
  `SUMMARY` shape is the documented one and must not change.
- `PROGRESS` is emitted from a background task every 10 s
  (`report.rs:250`). Under `--output json` it becomes one more NDJSON line.
  Harmless; just make sure it goes through the same record path.
- `tests/cli_contract.rs` — `--json-summary` → `--output` in the help list; add a
  case that `--output xml` exits `1`; **add a case asserting `--output json`
  emits only JSON lines on stdout** (no `KEY=value` text anywhere). That last one
  is the property downstream tooling will depend on.
- `tests/repair_offline.rs:77` — `json_summary: false` → `output: OutputFormat::Text`.
- `README.md:99`, `:227` and the five doc references above — update the wording,
  keep the links.

**Verify**

```
cargo test -p girpr
cargo clippy --workspace --all-targets
```

By hand: an unflagged run's stdout is byte-identical to before this commit.

---

# Commit 4 — `girpr: retry only what a retry can fix`

**Why.** `girpr` has **six unconditional retry loops** and not one looks at the
status code before trying again:

| loop | site |
|---|---|
| `fetch_manifest` | `src/sophon.rs:105` (sleep at `:110`, `error_for_status` at `:128`) |
| `fetch_chunk_bytes` | `src/sophon.rs:162` (`error_for_status` at `:164`, sleep at `:177`) |
| `get_node` | `src/hyp.rs:240` (sleep at `:274`) |
| `get_direct` | `src/hyp.rs:284` (sleeps at `:296`, `:323`) |

Measured consequences against a real bundle:

| the shim answers | today |
|---|---|
| 404 on a chunk that must be downloaded | **88.5 s**, exit `3`, 25 requests |
| 404 on a manifest | **15 s**, exit `2` |
| `retcode -201` on `getBuild` | **15 s** |
| `retcode -1001` on `getGameConfigs` | **15 s** |

Every one of those is terminal. `gircap` already encodes the correct policy —
`RetryPolicy::should_retry_status` at `tools/capture/src/http/mod.rs:205` retries
`408 | 429 | 5xx` and nothing else, with the reasoning in the module doc at
lines 24–28 and a test at line 1116 named
`a_404_is_not_retried_because_retrying_it_changes_nothing`. Two tools in one
repository disagree about the same status code.

**Files and specifics**

- `src/sophon.rs` — add one `pub fn retryable(status: u16) -> bool` beside
  `join_url` (that module owns both blob fetches), with the same rule. Use it in
  `fetch_manifest` and `fetch_chunk_bytes`: stop the loop on a non-retryable
  status instead of sleeping and re-issuing.
- `src/hyp.rs` — add terminal retcodes beside the existing `-202` special case
  (around line 308): `-201` and `-1001`. `get_node` has **no** terminal handling
  at all today. Note `retcode: 0` with an empty node is already handled correctly
  downstream — `hyp.rs:339`'s `empty game_branches` — so leave that alone.
- **The failure outcome must not change.** Exit `3` for a failed file, exit `2`
  for metadata, exactly as today. Only the wall clock changes: ~88 s → ~10 s (or
  ~0 s under `--fail-fast`) for a missing chunk, ~15 s → ~0 s for a missing
  manifest.
- **Why not retrying a 404 is safe here**, and this belongs in a code comment: a
  404 arrives for a chunk or manifest whose id came out of that same build's
  `getBuild` body — a namespace published atomically. There is no propagation
  window in which the id is advertised and the blob is not yet present.
- `tests/common/mod.rs` — `MockOpts` gains `not_found_chunks` and
  `not_found_manifests`, alongside the existing `fail_chunks` (which returns
  500). The mock already has the shape; it lacks the status. The 404 branch in
  `serve_one` must **still log the request** — "was this chunk even asked for"
  is the assertion that needs it, and a 404 is still a request.
- `tests/repair_offline.rs` — assert that a 404'd chunk reaches the server
  **once**, and that the run fails with exit `3` promptly.
- `README.md` — the `--fail-fast` row currently says "one bad file costs ~125 s
  of pure retry sleeping". Add that a 4xx now costs nothing, and note the policy.

**Verify**

```
cargo test -p girpr
```

Confirm by hand that a `fail_chunks`-style 500 **still** retries (that policy is
unchanged) while a 404 does not.

---

# Commit 5 — `gircap: record build_id on every chunk reference`

**Why.** The live chunk URL embeds the build id
(`…/chunks/cxhpq4g4rgg0/sMXGW2ll3Fuu`), and the same id under the *other* build's
prefix is a 404 — confirmed by measurement. So the build id is part of a chunk's
address, and the index does not record it.

**Files**

- `tools/capture/src/blobs/chunks.rs` — `Reference` (struct at line 96) has
  `tag` at line 99 and no `build_id`. Add it. The capture has the
  `tag → build_id` map from `captured.versions`, so it can always be filled;
  prefer a plain field, or `Option<String>` with `#[serde(default)]` if you
  prefer old documents to stay readable.
- `tools/capture/docs/01-artifact-schema.md` §12 — document the field.

**Back-compat is the point.** The field is additive; `verify` reads whatever is
on disk. Both existing bundles predate it and **must still verify 90/90 and exit
`0`**. Anything downstream reading `build_id` must fall back to
MANIFEST.json → versions.{old,new}.build_id`. If you capture a fresh bundle to
check the new shape, capture to a temp directory and leave `artifacts/` alone.

**Verify**

```
cargo test -p gircap
cargo run -p gircap -- verify --artifact artifacts/hk4e_global/7.0.0__7.1.0/20261015T111345Z
cargo run -p gircap -- verify --artifact artifacts/hk4e_global/7.0.0__7.1.0/20261015T092615Z
```

Both exit `0`. Optionally, `cargo run -p gircap -- capture --out <temp dir>` and
confirm the fresh `chunks/index.json` carries `build_id` on its references.

---

# Commit 6 — `gircap: capture the negative responses into `api/error_shapes.json``

**Why.** A bundle contains **no not-found evidence at all** — 49 requests, 49 ×
`status: 200`. A tool that replays a bundle therefore has to invent what a
missing object looks like, and its "the API 404s things" claim is unfounded.
This commit makes the measurement in this file into bundle content.

The precedent already exists: `--probe-tags` probes tags the API is expected not
to serve, and `api/tag_probe.json` records `http` + `retcode` + `served` for
them. Do the same thing for bad ids, but record the **response shape** — status,
content-type and body.

**Files and specifics**

- `tools/capture/src/http/probe.rs` — extend the probe mechanism (it already
  types `TagProbe` / `TagProbeRow`) to also record negative responses.
- `tools/capture/src/run.rs` — compose and run the probes next to the tag probe
  (around lines 162–183), after the pair's manifests are known so the blob
  prefixes are available.
- `tools/capture/src/emit/mod.rs` — write `api/error_shapes.json` beside
  `api/tag_probe.json` in `write_api`, and **add it to the returned `written`
  list** so it reaches `MANIFEST.json → files[]` and `verify/checksums.json`
  without further work (those are built from that list).
- `tools/capture/src/http/mod.rs` — the probe must **not** fail the capture and
  must not retry. A negative response is a successful observation. The fetcher
  already refuses to retry 4xx, so this is mostly a matter of routing the result
  into the document instead of into an `ApiError`.
- **Probes to record**, taken from the evidence section above:
  `blob_missing` (manifest prefix + bogus id), `chunk_missing`,
  `blob_wrong_build` (a valid id under the other build's prefix),
  `getbuild_unknown_tag` (`-202`), `getbuild_bad_package` (`-201`),
  `getgameconfigs_bad_game_id` (`-1001`), and
  `getgamebranches_bad_game_id` — which returns `retcode 0` with an empty node
  and is **not an error**; recording it is what stops a later reader
  mistaking it for one.
- **Mark which fields are volatile** (`RequestId`, `HostId`, `Key`, and the body
  length that follows from `Key`) and set `retryable: false` on each entry, so
  the document states in the artifact the fact commit 4 acts on.
- `tools/capture/docs/01-artifact-schema.md` — a new section for the document,
  and add it to the §1 tree.
- `tools/capture/docs/00-spec.md` §4 — say that `api/` carries the negative
  responses too, and that a consumer must not synthesize its own.

**Back-compat.** A bundle without `api/error_shapes.json` is still a valid
bundle: `Bundle::open` (`tools/capture/src/verify/mod.rs:113`) must **not**
require it. A consumer that wants the not-found behaviour and finds no such file
must skip those assertions and say so.

**Verify**

```
cargo test -p gircap
cargo run -p gircap -- verify --artifact artifacts/hk4e_global/7.0.0__7.1.0/20261015T111345Z   # exit 0
cargo run -p gircap -- verify --artifact artifacts/hk4e_global/7.0.0__7.1.0/20261015T092615Z   # exit 0
```

A unit test must confirm a capture's `error_shapes.json` reproduces the measured
statuses — use the existing mock-server infrastructure in `tools/capture`'s
tests, as the rest of the suite does, and keep that the primary check because it
needs no network.

A **live** capture is the stronger acceptance, and this commit is the one place
it earns its keep: `error_shapes.json` records what the real origin answered, so
a mock that merely agrees with itself proves nothing. If you run one, capture to
a **temp directory**, check the recorded statuses against the evidence section
above, and leave `artifacts/` untouched unless the bundle is good and you expect
to re-use it — see *Ground rules*.

---

# Commit 7 — `gircap: list emits one record per bundle`

**Why.** `gircap list --root artifacts` prints **one** `SUMMARY`
(`bundles=2 … first=… last=…`) and puts every per-bundle fact — path, tags,
capture id, patchmap count, blob count, selfcheck result — on **stderr at INFO**.
A machine can consume it only by parsing narration.

**Files and specifics**

- `tools/capture/src/report.rs` — generalize `Summary` into a `Record`
  (event + ordered fields) so a run may emit more than one line; keep `Summary`
  as the terminal record. **Rewrite the module doc** at lines 3–6 — it currently
  asserts "`gircap` writes exactly one line per run: a `SUMMARY`". Also note the
  `--output` flag already exists here; only the record shape is new.
  **Do not** route this through `serde_json::Map` to gain ordering — lines
  261–266 explain why that is wrong, and the warning applies to your change too.
- `tools/capture/src/run.rs` — `list()` (line 1172) emits one
  `BUNDLE path=… tags=… patchmaps=… blobs=… selfcheck=… complete=…` record, or
  `{"event":"bundle",…}` under `--output json`, per **valid** bundle, before the
  `SUMMARY`. Directories that look like bundles but do not open stay
  stderr-only, as they are today.
- `tools/capture/docs/00-spec.md` §7 (Output) — "one `SUMMARY` line, or a single
  JSON object" becomes "records, one per line, summary last".
- `tools/capture/docs/01-artifact-schema.md` — no change; this is stdout, not
  bundle content.
- `tools/capture/tests/cli_contract.rs` — assert the NDJSON shape of
  `list --output json`.

**Verify**

```
cargo test -p gircap
cargo run -p gircap -- list --root artifacts --output json
```

One `bundle` record per bundle, then one `summary`. Stdout is pure NDJSON.

---

# Commit 8 — rewrite `docs/06-artifact-bridge-spec.md`

**Why.** The spec is what the bridge will be written against, and as it stands
it contradicts itself and omits several facts established above. No code in this
commit — it is a rewrite in place, keeping the existing §1–§3 and §7–§8
structure where it is still right.

**Specifics, section by section**

**§1 Independence.** Remove the "acceptance grep … must keep returning doc
comments only" framing. The intent is *do not couple*; say that and stop. There is
no CI check and this order does not add one.

**§2 Placement.** "the fifteen existing offline tests" is wrong —
`tests/repair_offline.rs` has 50, `tests/cli_contract.rs` 4, the lib 76. State
the module mechanics: `tests/artifact_bridge.rs` plus `mod server;` resolving to
`tests/artifact_bridge/server.rs`, so neither `repair_offline.rs` nor
`cli_contract.rs` recompiles the server or sees its warnings.

**§2 + §5 + §6.3 — parallel safety. The most important fix in this commit.**
As written: §2 places the bridge in `tests/`, §5 says one run per capture, §6.3
says `game/` is copied to scratch **once** and the runs accumulate on it. Rust
runs `#[test]`s **in parallel threads**, so a shared tree, a shared listener and
a shared request log mean concurrent `girpr` processes fighting over one `game/`.
Rewrite as: **per capture, a fresh `game/` copy and its own listeners.** Copying
the fixture eight times takes 0.035 s, so isolation is free. Keep the shared blob
cache outside the listeners so it still pays across captures. Keep the
one-run-per-capture rule: it is what makes `download_bytes` attributable to a
single capture.

**§4 + §4.1 — the route table contradicts itself.** §4.1 says "replace every
`url_prefix` with the listener's own origin", which collapses the manifest and
chunk namespaces into one root, while §4's table assumes two distinct prefixes.
New rule: **change only the scheme and authority; keep the path**, and run **two
blob listeners** (one for manifests, one for chunks) alongside the hyp and
sophon metadata ones, so the two namespaces stay distinguishable by port.
Resulting shapes:

```
manifest : …/client_app/sophon/manifests/cxhpq4g4rgg0/<build_id>/manifest_<16hex>_<32hex>
chunk    : …/client_app/sophon/chunks/cxhpq4g4rgg0/<build_id>/<16hex>_<32hex>
```

Each listener takes the **last two** path segments and ignores the rest, so the
route is indistinguishable from production's and the test still proves `girpr`
builds URLs from `url_prefix` + `id` alone. State the invariant this buys:
**every blob request's `build_id` must equal the build whose manifest declared
that id** — the namespacing rule from 00-spec §3.7, now assertable.

**§4 — how a manifest id resolves.** The live ids are
`manifest_<16hex>_<32hex>`; the bundle stores blobs under field names. Say to
build the `id → (tag, field)` map **while rewriting** the body — the loop is
already there — and 404 anything absent. Without this sentence an implementer
reaches for a filesystem scan over `manifests/*/*.meta.json` and gets a slower,
different answer.

**§4 — `getBuild`.** Say explicitly that a tagless request serves
`api/getBuild.<versions.new.tag>.json`, and that an unserved tag returns the
recorded `retcode -202` body from `api/error_shapes.json` — **not one the shim
made up**. The consequence of getting it wrong is measured: exit **0**,
`local_manifest=unavailable`, `download_bytes=913040` instead of 274 426, and
the `must_not_fetch` chunk requested. That is the "degraded-but-green" of §7,
confirmed rather than hypothesised.

**§4 — not-found. Currently unstated, and everything else in §4 rests on it.**
§4.2 says "404 an unknown chunk id" as if it were a free choice. It is not; it
is a measured property, and the two halves of the API disagree with each other.
Restate the evidence section of this file: blob routes answer 404 with
`application/xml` and an AliyunOSS `NoSuchKey` body, JSON routes answer 200 and
put the failure in `retcode`, and `getGameBranches` with an empty node is not an
error at all. The shim must **replay** status, content-type and body from
`api/error_shapes.json`, synthesizing only the fields marked volatile.
`girpr` observes exactly two of those — status and content-type — so those two
are the contract; the body is fidelity, for a human reading the shim's log. When
the bundle predates that file, the shim 404s with `application/xml` and the
bridge **logs that the not-found assertions were skipped for lack of evidence**
rather than passing them.

Also note that after commit 4 "must fail fast and visibly" is now actually true —
one request, no backoff — so the shim can 404 honestly instead of hiding a gap
behind 88 seconds of silence.

**§4.3 — request log.** Say the server's own record of what it was asked for is
the **primary** source for `must_not_fetch`. `--log-file` (commit 2) is the
documented **last resort**: parse `chunk downloaded chunk=<id>` and
`no local reuse candidate -> download chunk=<id>` out of the trace lines, and
match on the **download record**, never the bare id — a reused chunk is logged
too, which is how the first attempt at this check false-positived on 19 chunks
that were never downloaded.

**§4 — which manifests are served.** Stop hardcoding `en-us`. Measured on the
real bundle: the `getBuild` body carries five manifests (`game`, `zh-cn`,
`en-us`, `ja-jp`, `ko-kr`); the run fetches exactly `game` + `en-us` — four
manifests per run, four more per `--print-plan` pre-pass — because
`sophon::select_manifests` drops every field whose name is five or ten
characters and contains a hyphen, then re-adds only the selected language, and
the other three blobs are not in the bundle. The rule becomes "index whatever
`manifests/<tag>/` holds; the run selects `game` ∪ the resolved audio set". Keep
the warning that adding `--audio` widens it.

**§4.2 — the 404 note.** Fold in what commits 4 and 6 establish: the server
answers 404 honestly and `girpr` now fails on the first attempt, so the "roughly
fifteen seconds of silence per chunk" warning is historical. Replace it with the
measured 88.5 s → ~10 s (~0 s under `--fail-fast`).

**§5 — one flag table with a "which runs" column**, replacing §5's table and
§6.4's prose so `--purge-only` and `--max-files 1` are not specified twice.
Add **`--fail-fast`**, which §5 currently omits and `00-spec.md` §8.1.1 step 3
passes: with the measured 17.6 s vs 88.5 s as the reason. Clarify the `--include`
cell — "the `path` field of `patchmaps/<id>.json`", not the arrow notation.
State the parse plane: `--output json` NDJSON, `report` / `plan` / `summary`
records, which commit 3 makes clean.

**§5.1 — the pre-pass rule is false for `removed`.** "`files=1` and
`est_download_bytes` equal the patchmap total" does not hold: `removed` has no
`totals` and prints
`PLAN files=0 est_download_bytes=0 purge_files=1 purge_bytes=1731133 scoped=1`.
Carve it out — for that capture the purge counters are the pre-pass assertion.

**§6 — assertions.** State that the tree diff is **per run** (before this run vs
after this run), because §6.3's copy is per capture under the §2 rewrite. Add
the structural assertion: **capture paths are pairwise distinct.** It is true for
the reference bundle (8 distinct paths) and `select`'s exclusion set keeps it
true, so this is a cheap guard rather than a known bug — but a repeat would make
the second run silently skip and report `download_bytes=0` against a non-zero
expectation, which is a baffling failure to diagnose.

---

# Commit 9 — correct the docs outside the bridge spec

**Files and specifics**

- **`README.md:250-252` is factually wrong, and it is exactly the gap the bridge
  exists to close.** It claims *"`tests/repair_offline.rs` drives the real binary
  through the flags against the mock, which is the only way to prove a flag
  actually reaches `HypClient`"*. That is false: `repair_offline.rs` calls
  `repair::run(ctx)` **in-process** — 50 call sites, no `Command`, no
  `CARGO_BIN_EXE_*`. `cli_contract.rs` is the only file that spawns the binary
  and it never reaches the API. Rewrite the claim and point at
  `tests/artifact_bridge.rs` as the thing that delivers the property — describing
  it as the test that is coming, since it does not exist yet.
- `README.md:239` — "34 unit + 15 integration" is stale. Use the numbers as of
  your commits 2–4, not the ones quoted in this file.
- `README.md:232-234` — the `--hyp-base` / `--sophon-base` note ("manifest and
  chunk URLs come from `url_prefix` inside the response bodies") is correct and
  load-bearing. Keep it; do not soften it.
- **`tools/capture/docs/00-spec.md` §2** — remove the `Enforcement` column's "a
  CI check rejects the string", and any wording that reads as though independence
  is mechanically enforced. The intent is *do not couple*; say that and stop. Do
  not add a test.
- **`tools/capture/docs/00-spec.md` §3.7** — promote "same chunk id under the
  other build's prefix → 404" from a note to a recorded observation, and extend
  §3 with the two failure shapes: the blob routes' 404 and the JSON routes'
  always-200-plus-`retcode`, with the measured `-202` / `-201` / `-1001`
  meanings. Both projects' retry policies are written against this and today it
  exists only in a review.
- **`tools/capture/docs/00-spec.md` §8.1.4 assertion 3** — demote to the
  predecessor of the server-side request log and say which is normative. Keep
  its own caveat (a reused chunk is logged too, so a bare substring search for an
  id false-positives) as the *reason* the server log is better.
- **`tools/capture/docs/00-spec.md` §8.1.1 step 2** — state the `--check-only`
  expectation rather than dropping it. Its `expect_md5` / `actual_md5` come from
  `tracing::warn!` in `src/repair/check.rs`, i.e. **stderr and the log file only,
  never stdout**, and under `--include <capture path>` the seven repairable
  captures are *expected* to report damage (exit `4`). Only `identical` is clean.
- **`tools/capture/README.md`** — the Status section says "Built to milestone
  **M4**". It is through **M12**: `verify` / `materialize` / `list`, `game/`,
  `expected/`, `patchmaps/`, `chunks/`, gzip, and A18/A19/A20 all shipped — both
  real bundles verify 90/90. Rewrite it. Add `gircap list --root artifacts` and
  `--output json` to the command block, since that is the command a reader is
  sent to. **Leave the `## Artifact bundles` section alone** — it already states
  the capture workflow correctly (where bundles live, when to re-capture,
  scratch-first, promote-when-settled) and it is where that policy belongs
  permanently.

---

## Do not

- Do **not** write `tests/artifact_bridge.rs` or the replay server. That is the
  next piece of work and needs the rewritten spec from commit 8 first.
- Do **not** modify, move or delete anything under `artifacts/`. Capturing is
  fine (see *Ground rules*) — capture to a temp directory, and only promote into
  `artifacts/` deliberately.
- Do **not** add a workspace member, a `src/bin/`, or a dependency between the
  two crates in either direction.
- Do **not** add an independence test. Commit 9 removes the wording that implies
  one should exist.
- Do **not** fix the other 21 rustdoc warnings, the `--plan-out` behaviour, or
  anything else this order does not mention. If you think something here is
  wrong, stop and say so.

---

## After all nine

Turn around and report. `tests/artifact_bridge.rs` becomes writable: an
`#[ignore]`d integration test in the `girpr` package, plus its server module,
that skips rather than fails when no bundle is present, and that runs
`gircap verify --artifact <dir>` as a precondition.