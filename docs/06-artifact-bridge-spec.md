# Artifact bridge — serving a capture bundle to `girpr`

**Status: shipped.** `tests/artifact_bridge.rs` + `tests/artifact_bridge/`
in the `girpr` package.

**Scope:** the `girpr` crate and its test tree. `tools/capture` (`gircap`) is
named because that is where the input comes from; nothing here changes it.

This document states what the harness does and what it does not do. The
implementation is written against it, so the contract stands as written.

---

## 1. Why it exists

`gircap` writes a self-contained bundle per version pair: a real `game/` fixture
holding one file per patch shape, the API bodies and manifests behind them,
every chunk blob, `expected/` ground truth, and a `patchmaps/<id>.json` per
capture. Three consumption paths work — inspect the metadata, rebuild old/new
offline, prove the bundle's own integrity. The fourth does not: **run `girpr
--game-path <artifact>/game` and assert it patches to the new version.** That
path is what this designs.

A **loopback server that replays one artifact bundle**, plus a **driver that runs
the real `girpr` binary against it and checks the result**.

### 1.1 The claim, and only the claim

> **`girpr`'s output is identical to what another repair tool computes, whatever
> `girpr`'s internal implementation happens to be.**

`gircap` computes the patch independently — its own manifest diff, its own
reuse scoring, its own arithmetic — and records the ground truth in the
bundle. `girpr` is then run against the same inputs and its result is compared.
That is a **cross-tool** claim, and it is **three assertions per capture** (§7).

Everything else about `girpr` — which tier it picked, which direction it wrote
in, whether it trimmed eagerly, what its `SUMMARY` line said — is **`girpr`'s own
business**. It is not shared with any other repair tool, it is not what the
bundle is for, and it is tested where it belongs: on crafted fixtures, in
`girpr`'s `tests/repair_machinery.rs`. A 192-byte input states a decision
exactly, and a bundle of real game bytes states it at the price of a download.

**The claim is not specific to one version pair.** It has been checked against
`hk4e_global` 7.0.0 → 7.1.0 and against 6.7.0 → 7.1.0: two bundles, two
different sets of files and two different sets of numbers, and the same six
captures green on both. A claim that held on one pair alone would be a claim
about that pair.

**The second bundle is not kept in the tree**, because `artifacts/` is
gitignored and a bundle is a hundred megabytes of game bytes rather than a
document — so this is a claim a reader can check, not one they have to take on
faith. Reproduce it with

```
cargo run -p gircap -- capture --old-tag 6.7.0 --new-tag 7.1.0 --out artifacts
```

and point the bridge at the result with `GIRPR_ARTIFACT_BUNDLE=<dir>`. The
captures are then the same six classes over a different pair's files, and the
byte totals are the pair's own: on 6.7.0 → 7.1.0 the `reuse_in_place` capture
fetches 839 603 B where the 7.0.0 pair fetches 274 426 B, and `reuse_moved`
fetches 2 745 798 B where it fetches 1 564 894 B.

| | the bridge does | the bridge does not |
|---|---|---|
| network | loopback only | touch the internet, ever |
| disk | reads the bundle, writes a scratch copy of one file | write inside the bundle |
| the bundle | consumes it | regenerate, repair, or re-capture it |
| `girpr` | runs the real binary, reads `REPORT`/`PLAN`/`SUMMARY` | import `girpr` as a library, link it, or reimplement its logic |

Preparing a bundle is a **separate, human-run, network-using step** that already
exists — `gircap capture`. The bridge begins after it. That split is what makes
"offline" a property of the design rather than a promise: the bridge never
captures, so it never needs the internet.

**Independence is one-directional, and the point is to not couple.** `gircap`
must not depend on `girpr` and must not invoke it, so the bridge lives on the
`girpr` side. Nothing in the capture's tree reaches for the `girpr` binary.

### 1.2 What is not here

| | where it is |
|---|---|
| the repair's own decisions — the tier, the write direction, the trim, the growth, the mode, the resume, and what a malformed manifest does | `girpr`'s `tests/repair_machinery.rs` — crafted fixtures, always on |
| the replay server's own behaviour — the rewrite, the `build_id` invariant, the request log, the recorded failure shapes | `girpr`'s `tests/artifact_server.rs` — a synthetic mini-bundle, always on |
| the bundle's own consistency — the matrix against the patchmaps, and the discovery that finds them | `gircap`'s `tests/bundle_consistency.rs` — the bundle is built by the tool's own writers |

---

## 2. Placement

**A `#[ignore]`d integration test in the `girpr` package** —
`tests/artifact_bridge.rs`, with the server in a module beside it — not a new
workspace member and not a `src/bin/`.

The deciding constraint is Cargo's `CARGO_BIN_EXE_<name>`, which is set only
when building a test, bench or example target. An integration test in the
package that owns the binary gets the absolute path to the binary Cargo just
built, for free. A separate crate does not, and neither does a plain `src/bin/`
helper: both start by hand-rolling discovery or demanding a prior
`cargo build -p girpr` plus a guess at `target/{debug,release}/girpr[.exe]`.

That matters because the whole point of the bridge is to prove that **`--include`
and `--hyp-base` reach `HypClient`** through the real flag-parsing path. Calling
`repair::run` with a hand-built `RunCtx` would exercise the library and prove
nothing about the surface.

| alternative | why not |
|---|---|
| `tools/e2e` crate | earns its keep only if it also captures bundles, which `gircap capture` already does. Costs binary discovery, a workspace member, and README/doc rows |
| `src/bin/*.rs` | same discovery problem, inside the shipping package |
| feature-gated test (`#![cfg(feature)]`) | a broken bridge would stop compiling with no default build noticing; compile-time coverage of the harness is worth more than a few hundred lines of test compilation |

### 2.1 The server is its own module

```rust
// tests/artifact_bridge.rs
#[path = "artifact_bridge/server.rs"]
mod server;
#[path = "artifact_bridge/bundle.rs"]
mod bundle;
```

That `#[path]` is **required**, not decorative. For a crate root at
`tests/artifact_bridge.rs`, `mod server;` resolves relative to `tests/` — so it
looks for `tests/server.rs` or `tests/server/mod.rs`, and
`tests/artifact_bridge/server.rs` is not on that path. Both alternatives fail
with `E0583: file not found for module 'server'`.

The point of the module being its own is compilation isolation: `mod server;` in
`repair_offline.rs` or `cli_contract.rs` would put a several-hundred-line replay
server into both of those compilation units, where it would be recompiled on
every change to either and whose warnings would land on files that do not use it.
`tests/common/` is already shared by those two, which is fine — it is a mock, and
it is small. This one is not small and neither test wants it.

**The split is a split of concerns, not of convenience.** `server.rs` knows the
**routes**: it serves bytes and answers requests. `bundle.rs` knows the
**records**: it reads the matrix and the patchmaps. The server's own tests
(`tests/artifact_server.rs`) run against a synthetic mini-bundle and must not
depend on the reader — so the reader is not what they use.

---

## 3. Two gates, both required

### 3.1 Gate 1 — every bridge test is `#[ignore]`d, because the suite is expensive

One capture is a `--print-plan` pre-pass plus a real run, and each run fetches
four manifests — ~20 MB of real bundle bytes — before it can plan a single file.
The suite is six cases and well under a minute in release. That is worth paying
when something has gone wrong and not worth paying on every `cargo test`.

So it runs **by hand, deliberately, and only when the repair spec is suspected
to have regressed**:

```
cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
```

Nothing else runs it. `cargo test` still *compiles* the file, which is what keeps
it from rotting against `RunCtx`, `PathScope`, and the `SUMMARY` contract: a
broken bridge fails to build rather than failing silently.

### 3.2 Gate 2 — a missing prerequisite is an error, not a skip

This follows directly from Gate 1. Because the suite only ever runs
intentionally, nobody types `--ignored` here by accident, so the hazard a skip
would guard against does not exist — while its opposite does. "I ran the check
and it found nothing to check" must never render as a green line. A run without
a bundle **fails**, and the failure names the exact command that produces one.
An absent bundle is *missing input for a check someone asked for*, and it says so
loudly.

Resolution order:

1. `GIRPR_ARTIFACT_BUNDLE` — an explicit path wins, always;
2. otherwise `gircap list --root artifacts`, which reports one `bundle` record
   per bundle on stdout;
3. otherwise a directory scan under `artifacts/`, which must agree with (2) —
   `gircap`'s own `tests/bundle_consistency.rs` asserts the two name the same
   bundles in the same order, so which one ran never depends on whether a
   sibling workspace member happened to be compiled;
4. otherwise **fail**, naming the reason and the exact command:
   `cargo run -p gircap -- capture --out artifacts`.

The same rule applies one level down: a capture the suite names must be in the
bundle. **The capture is resolved through the matrix**, not by looking for a
patchmap named after it: `MANIFEST.json → matrix[]` is the only place that
knows which capture answers which class, and the two are not the same thing.

A capture the matrix records as *not answered*, with a reason, is a **fact**
rather than a missing prerequisite — the case stands down out loud — but a
capture the matrix does not mention at all is a missing prerequisite and fails.

**Precondition.** Before any run, the bundle's own integrity is established —
`gircap verify --artifact <dir>`, exit `0`. It makes a failing check rule the
bundle out as the cause instead of leaving it as the first suspect. The binary is
found next to `CARGO_BIN_EXE_girpr`, so there is no guessing and no
`cargo build -p gircap` step to remember. When `gircap` is not built the
precondition is **skipped with its reason printed**; the bridge does not
re-implement `verify`. A bundle that *fails* verification is a hard failure, not
a skip — a corrupt bundle is a real problem and must not hide behind a green run.

---

## 4. The server

An ephemeral `127.0.0.1:<port>` listener per route family. **Four listeners, one
route family each** — the only load-bearing requirement is that *manifests and
chunks are on different ports* (§4.1's `build_id` invariant), and extra
listeners buy nothing that one port plus a `build_id` check inside it does not.

| request | served from | rewritten? |
|---|---|---|
| `GET <hyp>/getGameConfigs` | `api/getGameConfigs.json` | no |
| `GET <hyp>/getGameBranches` | `api/getGameBranches.json` | no |
| `GET <hyp>/getGameDeprecatedFileConfigs` | `api/getGameDeprecatedFileConfigs.json` | no |
| `GET <sophon>/getBuild[?tag=]` | `api/getBuild.<tag>.json`, or `api/error_shapes.json` | **yes** |
| `GET <manifest-prefix>/<manifest-id>` | `manifests/<tag>/<field>.manifest.zst` | origin only |
| `GET <chunk-prefix>/<chunk-id>` | `chunks/blobs/<chunk-id>.zst` | origin only |

### 4.1 Change the scheme and authority; keep the path

`--hyp-base` / `--sophon-base` redirect the **metadata calls only**. Manifest and
chunk URLs are built by `sophon::join_url` from a `url_prefix` read out of the
**response body**, so a body served verbatim sends `girpr` to the production CDN
regardless of the flags. Rewriting the bundle on disk is not an option twice
over: the port is not known until the listener binds, and the bundle has to stay
byte-identical so `gircap verify` still passes afterwards.

So: parse `api/getBuild.<tag>.json`, replace the **scheme and authority** of every
`manifests[].chunk_download.url_prefix` and
`manifests[].manifest_download.url_prefix` with the listener's own, re-serialize,
serve that. **In memory only — the bundle is opened read-only and never written.**

**Keep the path.** Replacing the whole prefix with one origin would collapse the
manifest and chunk namespaces into a single root and stop the two namespaces
being *distinguishable*. They stay distinguishable **by port**: one listener on
the manifest prefix's port, one on the chunk prefix's port. The shapes `girpr`
builds are then

```
manifest : <origin>/client_app/sophon/manifests/cxhpq4g4rgg0/<build_id>/manifest_<16hex>_<32hex>
chunk    : <origin>/client_app/sophon/chunks/cxhpq4g4rgg0/<build_id>/<16hex>_<32hex>
```

Each blob listener **takes the last two path segments and ignores the rest**. The
route is then indistinguishable from production's, and the bridge still proves
`girpr` builds URLs from `url_prefix` + `id` alone.

The invariant this buys is the one worth stating out loud:

> **every blob request's `<build_id>` must equal the build whose manifest
> declared that id.**

A blob request for the same id under the other build's prefix is a `404`
(`blob_wrong_build`), so a shim that got the prefixes crossed would produce a
`404` it had manufactured itself — which is precisely the failure the two ports
exist to make impossible.

The rewrite is otherwise deliberately narrow. Everything else in the body passes
through untouched, and two fields are **load-bearing and must not change**:

- `manifest.checksum` — `sophon::fetch_manifest` MD5-verifies the *decompressed*
  manifest against it. Alter it and the run dies at metadata with exit `2`,
  looking like a product bug.
- `manifest.id` — it is the URL path segment and the `url_suffix` pairing.

`url_suffix` is irrelevant: `sophon::join_url` builds `{prefix}/{id}` and never
reads it.

### 4.2 How a manifest id resolves to a file

The live ids are `manifest_<16hex>_<32hex>`; the bundle stores archives under
*field* names (`manifests/<tag>/game.manifest.zst`). So there is a translation,
and the `id → (tag, field)` map is built **while rewriting the body** — the loop
over `manifests[]` is already there, and the rewrite is where every id in the
bundle passes through. **404 anything absent from that map.**

Doing it with a filesystem scan over `manifests/*/*.meta.json` instead would be
slower and would answer a different question: it would find any archive in the
bundle rather than the one *this* build declared, which is the distinction the
`build_id` invariant above is about. The `.meta.json` sidecars carry the same
facts but are deliberately not wired in: `getBuild.<tag>.json` is the
authoritative body the client actually parses, and a second source of truth for
the same fact is the kind of thing that drifts.

### 4.3 `getBuild`, and what happens when it is wrong

**A tagless request serves `api/getBuild.<versions.new.tag>.json`.**

**An unserved tag returns the recorded `retcode -202` body from
`api/error_shapes.json`** (`getbuild_unknown_tag`) — **not one the shim made
up.** The second `getBuild` is `?tag=<game_version from config.ini>`, and it is
the one that populates the chunk-reuse map.

The consequence of getting it wrong is the worst failure in this document,
because it is **green**:

> A tag the shim does not serve degrades to the local-build fallback: the run
> proceeds with an **empty reuse map**, downloads every chunk, and **still exits
> `0`** — reporting `local_manifest=unavailable`, `download_bytes=913040` instead
> of `274426`, and requesting the `must_not_fetch` chunk.

Two assertions catch it, and both are required: the byte comparison of §7.1, and
the assertion that `local_manifest` did not come back `unavailable`.

### 4.4 Not-found is replayed, not chosen

Measured against the live endpoints on 2026-10-16, the route families answer
failure **differently**:

| family | request | answer |
|---|---|---|
| blob (`autopatchhk.yuanshen.com`, `Server: AliyunOSS`) | bogus / short id, unknown `build_id`, **a valid id under the other build's prefix** | **`404`** + `application/xml` `<Error><Code>NoSuchKey</Code>…` |
| JSON (`sg-hyp-api`, `sg-downloader-api`) | `getBuild?tag=<unknown>` | `200` + `retcode -202` |
| | `getBuild` bad `package_id` | `200` + `retcode -201` |
| | `getGameConfigs` bad `game_ids[]` | `200` + `retcode -1001` |
| | `getGameBranches` bad `game_ids[]` | `200` + `retcode 0`, **empty node — not an error** |

Three consequences for the shim:

1. **Replay status, content-type and body from `api/error_shapes.json`**, and
   synthesize only the fields that document marks volatile — `RequestId` (24
   characters, as recorded), `HostId`, `Key`, and the body length that follows
   from `Key`. `girpr` calls `.error_for_status()` and never reads the body, so
   **status and content-type are the contract and the body is fidelity**, for a
   human reading the shim's log.
2. **A bundle that predates the file has no `error_shapes.json`.** The shim then
   404s with `application/xml` — the measured shape — and the bridge **logs that
   the not-found assertions were skipped for lack of evidence** rather than
   passing them silently. Inventing the statuses would make this document's claim
   about the API unfounded.
3. **A 404 costs almost nothing.** `girpr` retries only `408 | 429 | 5xx`
   (`sophon::retryable`), so a 404 is asked **once, with no backoff**. Measured: a
   run against a real bundle with one chunk 404'd went from **88.5 s and 25
   requests** to roughly **10 s and one request**, and to **~0 s** under
   `--fail-fast`, with the exit code unchanged at `3`.

Shapes with no route are still checked for presence, status, content-type and
retcode, so the bundle's record is read rather than assumed — but they are not
asserted as replayed, because the bridge never issues such a request (a bad
`package_id` or `game_ids[]` is something only `gircap capture` sends).

### 4.5 Which manifests to serve

**Index whatever `manifests/<tag>/` holds** — that is, `game` plus whichever
audio fields the capture stored. Do not hardcode a language.

*(measured)* the `getBuild` body carries five manifests for this biz: `game`,
`zh-cn`, `en-us`, `ja-jp`, `ko-kr`. `sophon::select_manifests` drops every field
whose name is five or ten characters and contains a hyphen, then re-adds only the
selected languages; the bundle's scan file selects `en-us`, and only the blobs
the capture stored are present. So a run fetches **`game` + `en-us`** — four
manifests per run, plus four more for the `--print-plan` pre-pass (§6.1) — and
the other three are never requested.

The rule generalises to whatever the bundle holds, so the run's selection and the
bundle's contents decide the set between them. **Keep the `--audio` warning:**
adding a language widens the selection, and a bundle that does not carry that
field's manifest will 404 on it — which, after §4.4, fails immediately and
visibly rather than after fifteen seconds of silence.

### 4.6 Cache served manifest bytes

Each run fetches four manifests and the `manifests/` directory is ~41 MiB of the
~98 MiB bundle; across the suite's runs that is over a hundred megabytes
re-read for nothing. They are immutable per bundle, so cache them.

The cache is process-global, keyed by `(bundle, tag, field)`, and shared by every
listener: it holds bytes that are a function of the bundle alone, so nothing
about per-capture isolation is lost by sharing it.

### 4.7 The request log is not optional, and it is the primary source

`SUMMARY` is byte-level. For the **fetch set** — "these chunks were requested
and none of the others was" — **the server's own record of what it was asked for
is the clean source**: `Vec<(Kind, id)>` behind a mutex, appended before each
response is written. A byte count cannot see a chunk that was fetched when it
should have been reused *locally*, and that is exactly the failure the fetch-set
assertion exists to catch.

A forbidden-set — answering such a request with an error instead of bytes —
would answer with a 404 the shim manufactured and blur the two failure modes, so
the log alone is the mechanism.

`--log-file` is the **documented last resort**, not the first choice: parse
`chunk downloaded chunk=<id>` and `no local reuse candidate -> download
chunk=<id>` out of the trace lines. Those are `trace` records, and the log
file's own level is `debug` by default, so the run needs
`--log-file-level trace` to have them (`--log-level trace` would only make
them visible on the console).

**What the two records mean.** `chunk downloaded chunk=<id>` is emitted as soon as the
origin has answered with the chunk — it says **the origin was asked for this chunk
and served it**, not that its body decoded. A body that turned out to be corrupt
was still fetched, and the record has to say so: the assertion it serves is
"this id was not requested", and a request that is invisible to the log is a
request the log cannot rule out. The compressed byte count is on the
`chunk decoded compressed_bytes=… decompressed_bytes=…` line, which is emitted
when the stream ends. `no local reuse candidate -> download chunk=<id>` is the
*decision*, before the fetch.

**Match on the download record, never on the bare id.** A reused chunk is logged
too (`reuse slice md5 ok -> copy chunk=<id>`), so a substring search for an id
finds it in the "reused it" line and false-positives. The pre-pass gets its **own**
log file, because `logging::open_log_file` opens with `append` and one shared file
could not say which run downloaded what.

---

## 5. The driver

**One run per capture.** That is what makes a byte total attributable: a single
run over all of them sums to the same number but cannot be compared per capture.

Per capture, against a **fresh scratch copy of that capture's file** and **its own
listeners** (§6.3 for why):

| flag | why | which runs |
|---|---|---|
| `--game-path <scratch>/game` | never the bundle; the bundle must stay byte-identical | all |
| `--biz hk4e_global` | matches `MANIFEST.json → biz` | all |
| `--hyp-base`, `--sophon-base` | the listener's own origins | all |
| `--include "<the `path` field of `patchmaps/<id>.json>`" | exactly one path | all |
| `--purge-only` | the files-cleanup, without the repair loop | `removed` only |
| `--io-threads 1` | one planned file; determinism over throughput | all |
| `--fail-fast` | a shim fault should stop the run, not spend the budget discovering the same 404 on every remaining file | all |
| `--output json` | NDJSON, so the driver reads records by name — `report`, `plan`, `summary` — rather than scraping `key=value` | all |
| `--repair-mode inplace` | the mode is **asked for**, so the run's path is the one the claim is about rather than the one the mode resolver happens to pick | all but `removed` |

**Every capture gets `--include`, `removed` included.** `--purge-only --include
<that path>` is precisely the request that deletes a path the new manifest
dropped, and `scoped=1` is only reachable with a scope flag. `removed` is the
only capture that gets `--purge-only` and the only one that does not get
`--max-files` (there is no repair plan to bound).

**`--audio` is omitted, deliberately.** An explicit audio selection under an
active path scope is refused when the scan file falls outside that scope, and
`GenshinImpact_Data/Persistent/audio_lang_14` does. Omitting the flag lets the
run autodetect from the scan file and resolve to `en-us` — the state the bundle
was captured in.

**No cleanup flag** for the repair runs: a scoped cleanup sees only the one
selected path, which the manifest contains, so it has nothing to delete. The one
capture that *wants* a cleanup is `removed`, and it is the row that says so.

**The staged tier is the default, and that is what the claim needs.** The staged
tier holds the moving bytes in RAM, so every source survives and the chunks the
repair asks for are exactly the chunks the patch says have no local source. The
ordered tier is `girpr`'s business — it is a choice of how to get there — and it
is tested on a crafted fixture.

---

## 6. Isolation, and the free pre-pass

### 6.1 `--print-plan` pre-pass

`--print-plan` returns before `create_dir_all` and emits a `plan` record at zero
disk cost:

```
PLAN files=1 est_download_bytes=274426 purge_files=0 purge_bytes=0 max_chunk_bytes=… scoped=1
```

Run it once per capture first. It separates the two failure modes that otherwise
arrive tangled in one red run:

- `files=1` → **the scope selected the right thing**;
- the real run's `SUMMARY download_bytes` equal to the patchmap's total → **the
  repair did the right thing**.

When the first fails, the glob is wrong. When the second fails, reuse is wrong.
`--plan-out` gives the per-file JSON for the same run.

**`removed` is the exception, and the rule above does not apply to it.** A
`removed` patchmap carries `totals: null`, because there is nothing to patch, and
its pre-pass prints

```
PLAN files=0 est_download_bytes=0 purge_files=1 purge_bytes=1731133 scoped=1
```

So for that capture the **purge counters are the pre-pass assertion**: `files=0`
(the path is correctly absent from the repair plan) and `purge_files=1
purge_bytes=1731133` (the cleanup found it and priced it). Asserting `files=1`
here would fail forever for a reason that has nothing to do with the bridge.

### 6.2 Isolation is per capture, because the tests run in parallel

**Rust runs `#[test]`s in parallel threads.** A shared scratch tree, a shared
listener and a shared request log mean concurrent `girpr` processes fighting over
one tree: two runs would patch the same path, one run's fetch set would see the
other's requests, and a diff would compare a tree some other run had already
changed. The failure would be intermittent and would look like a product bug.

So, **per capture:**

- **its own listeners** (its own manifest and chunk ports, its own hyp/sophon
  origins), so the `build_id` invariant of §4.1 is per-capture too;
- **its own request log** and **its own log file** for the pre-pass.

**The served-bytes cache stays shared** (§4.6) because it holds nothing
capture-specific.

### 6.3 The bundle is the fixture; a copy is the working tree

The bundle's `game/` is the **input**. The working tree is a scratch copy of it.
Each run touches exactly one path and leaves the rest alone, and the reuse map
stays anchored to the old version because a scoped run does not bump
`config.ini`. The bundle itself is never a working tree — pointing `--game-path`
at it in place is what turns a cleanup flag into a way to destroy the evidence.

**The copy is of the one file the capture is about**, not of the whole fixture:

```
<scratch>/game/config.ini                        # the bundle's, verbatim
<scratch>/game/<audio_pkg_scan_dir>              # the bundle's, verbatim
<scratch>/game/<the capture's path>              # the OLD bytes
```

The other captures' files are not what this capture is about. "Every path
outside the scope is untouched" is `girpr`'s own claim, and it is tested where
it belongs: on a crafted multi-file manifest, in `girpr`'s
`tests/repair_machinery.rs`, where five small files cost five small files
rather than the whole fixture.

---

## 7. The assertions

**Three per capture**, plus the run's preconditions — and one more for the
capture that is a deletion rather than a patch. None of them is optional; the
second is the one that earns the whole exercise.

| # | assertion | source |
|---|---|---|
| 1 | the patched file's bytes are `expectations.result_md5`, and its length is `new.size` | `patchmaps/<id>.json` |
| 2 | **the chunks asked for are the chunks the patch says must be fetched, and none of the ones it says can be reused was asked for** | the patchmap's `plan[]` + the server's request log |
| 3 | `SUMMARY download_bytes` equals the patchmap's total | `patchmaps/<id>.json` |
| 3b | for the capture that is **deleted**: the path is gone, `deleted_extra_bytes` equals the patchmap's `old.size`, and `download_bytes` is `0` | the run + `patchmaps/<id>.json` |
| — | *precondition:* exit `0`, `local_manifest == "ok"`, `scoped == 1`, `config_bumped == 0` | the run |

**3b is a cross-tool fact and not `girpr`'s own business.** The bytes the run
deleted are the old file's bytes, and the patchmap states the old file's size;
a run that removed the path and reported a different number would have passed a
weaker check. The patchmap has no `totals` for that capture — there is no
target to patch — so the bundle states the byte total as zero, and that is
where the zero is checked.

### 7.1 The byte total is exact, not approximate

`girpr`'s `download_bytes` counts the actual bytes of files that were promoted,
giving back the bytes of any attempt that failed and was retried. The patchmap's
total is `Σ compressed_size` over the chunks with no `(uncompressed_md5,
uncompressed_size)` match on the old side. A6 asserts `len(blob) ==
compressed_size`, and the runtime ledger folds the real measurement back into the
manifest's reservation, so the two figures are equal by construction — not
approximately equal, and not equal only when nothing retries.

The same reasoning carries the in-place path: a chunk the old file cannot supply
is fetched by either mode, and a chunk it can supply is copied by either mode —
the in-place path copies it from inside the file instead of from a temp file. So
the equality holds for the in-place mode too, and it is worth pinning rather
than relaxing: it is what proves the writer fetches the same set the patchmap
says a consumer must fetch, and not one chunk more.

### 7.2 The fetch set, and why it is the load-bearing one

The patchmap's `plan[]` says, per chunk, whether the bytes are already in the
old file (`REUSE`) or have to come off the network (`FETCH`). The repair asked
for **exactly** the `FETCH` ones, and for **none** of the `REUSE` ones.

The negative half is the one that catches a broken reuse path: with an empty
reuse map every chunk is fetched, `local_manifest` says `unavailable`, and the
byte total is the whole file instead of the patch. The positive half says the
repair did not fetch *more* than the patch says — a chunk it could have copied
and fetched anyway would cost the consumer bandwidth and would not show up in
the byte total as an error.

### 7.3 What is not here

The repair's own decisions are not here, and that is deliberate: the tier, the
write direction, the trim, the growth, the mode, the resume and what a
malformed manifest does are `girpr`'s, and they are asserted against crafted
fixtures in `girpr`'s `tests/repair_machinery.rs`. A bundle of real game bytes
can state them, but it states them at the price of a download, and it states
them as a *fact about a bundle* rather than as a fact about the repair.

---

## 8. Failure modes this design must not have

- **Degraded-but-green.** A run that reused nothing because the local manifest did
  not load reports the same exit code and a plausible byte count (§4.3). The
  byte comparison against the patchmap catches it; so must the assertion that
  `local_manifest` did not come back `unavailable`, and that the old tag was
  actually served.
- **A glob that matches more than intended.** `*` crosses `/`, and the fixture
  contains paths with spaces in them. `--fail-fast` bounds the damage, and the
  pre-pass's `files=1` says so before anything is written.
- **A shim that quietly serves less than the bundle.** Every served byte must come
  from a file the bundle vouches for; anything else is a different test. The same
  goes for a shim that *invents* a failure shape it has no evidence for.
- **Parallel runs sharing a tree.** §6.2.
- **A fixture mutated in place.** The bundle must still pass `verify` afterwards.
  That is the precondition of §3.2 and the point of §6.3.
- **A bridge that reports success when it checked nothing.** Gate 2 exists for
  this: the suite runs only when a human asks for it, so a missing bundle — or a
  missing capture — is missing input, and it fails with the command that produces
  it.

---

## 9. Module tree

```
tests/
  artifact_bridge.rs            the #[ignore]d driver + Gate 2 + the cases
  artifact_bridge/
    bundle.rs                   the matrix and the patchmaps — the records
    server.rs                   Bundle, Bridge, routes, request log, error shapes
  artifact_server.rs            the server's own tests, against a mini-bundle
  repair_machinery.rs           the repair's own decisions, on crafted fixtures
```

All are **test-only**; nothing under `src/` changes. `src/sophon.rs` carries a
note that a trace log can be parsed for `chunk downloaded chunk=<id>` — that note
stays accurate, and that line remains the documented last resort rather than what
the bridge actually reads.

---

## 10. Non-goals

- **Capturing.** `gircap capture` does that, with the network, deliberately.
- **Re-verifying the bundle.** `gircap verify` and `gircap materialize` own that
  half; the bridge consumes their verdict (§3.2), it does not recompute it.
- **A second mock server.** `tests/common/mod.rs` already covers the synthetic
  path with a small fixture. The bridge is the opposite: real bytes, from
  disk, at production scale.
- **Automating the bundle preparation.** A bundle is captured when someone wants
  one, by hand.
- **Sharing code with `gircap`.** A server that replays a bundle's bytes shares no
  knowledge of how the bundle was made. A shared crate would make two tools that
  must stay independent depend on each other to prove one of them works.
