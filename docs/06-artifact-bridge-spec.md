# Artifact bridge — serving a capture bundle to `girpr`

**Status: shipped.** `tests/artifact_bridge.rs` + `tests/artifact_bridge/server.rs`
in the `girpr` package.

**Scope:** the `girpr` crate and its test tree. `tools/capture` (`gircap`) is
named because that is where the input comes from; nothing here changes it.

This document states what the harness does and what it does not do. The
implementation is written against it, so the contract stands as written.

---

## 1. Why it exists

`gircap` writes a self-contained bundle per version pair: a real `game/` fixture
holding one file per test-matrix variant, the API bodies and manifests behind
them, every chunk blob, `expected/` ground truth, and a `patchmaps/<id>.json` per
capture. Three consumption paths work — inspect the metadata, rebuild old/new
offline, prove the bundle's own integrity. The fourth does not: **run `girpr
--game-path <artifact>/game` and assert it patches to the new version.** That
path is what this designs.

A **loopback server that replays one artifact bundle**, plus a **driver that runs
the real `girpr` binary against it and checks the result**.

| | the bridge does | the bridge does not |
|---|---|---|
| network | loopback only | touch the internet, ever |
| disk | reads the bundle, writes a scratch copy of `game/` | write inside the bundle |
| the bundle | consumes it | regenerate, repair, or re-capture it |
| `girpr` | runs the real binary, reads `REPORT`/`PLAN`/`SUMMARY` | import `girpr` as a library, link it, or reimplement its logic |

Preparing a bundle is a **separate, human-run, network-using step** that already
exists — `gircap capture`. The bridge begins after it. That split is what makes
"offline" a property of the design rather than a promise: the bridge never
captures, so it never needs the internet.

**Independence is one-directional, and the point is to not couple.** `gircap`
must not depend on `girpr` and must not invoke it, so the bridge lives on the
`girpr` side. Nothing in the capture's tree reaches for the `girpr` binary.

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

---

## 3. Two gates, both required

### 3.1 Gate 1 — every bridge test is `#[ignore]`d, because the suite is expensive

One capture is a `--print-plan` pre-pass plus a real run, and each run fetches
four manifests — ~20 MB of real bundle bytes — before it can plan a single file.
The suite is twenty-six cases and ~21 minutes. That is worth paying when something
has gone wrong and not worth paying on every `cargo test`.

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
   `discovery_agrees_whether_it_asks_gircap_or_scans` asserts the two name the
   same bundles in the same order, so which one ran never depends on whether a
   sibling workspace member happened to be compiled;
4. otherwise **fail**, naming the reason and the exact command:
   `cargo run -p gircap -- capture --out artifacts`.

The same rule applies one level down: a class the suite names must be answered
by the bundle. **The class is resolved through the matrix**, not by looking for
a patchmap named after it: `MANIFEST.json → matrix[]` is the only place that
knows which capture answers which class, and the two are not the same thing. A
class the matrix records as `covered_by` has **no capture of its own** — the
file that shows it is another capture's (`trim_deferred` is answered by
`moved_ascending`'s capture).

A class the matrix records as *not answered*, with a reason, is a **fact** rather
than a missing prerequisite — the case stands down out loud (`moved_mixed` has
no candidate the budget could pay for and `MANIFEST.json` says so) — but a
class the matrix does not mention at all is a missing prerequisite and fails.

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
~696 MiB bundle; across the suite's runs that is over a gigabyte re-read for
nothing. They are immutable per bundle, so cache them.

The cache is process-global, keyed by `(bundle, tag, field)`, and shared by every
listener: it holds bytes that are a function of the bundle alone, so nothing
about per-capture isolation is lost by sharing it.

### 4.7 The request log is not optional, and it is the primary source

`SUMMARY` is byte-level. For `expectations.must_not_fetch` — "no chunk id in this
list was requested", the assertion that catches a broken reuse path and is
invisible in a byte count — **the server's own record of what it was asked for is
the clean source**: `Vec<(Kind, id)>` behind a mutex, appended before each
response is written. The log already turns a reuse regression into a loud failure
at §7 assertion 3, with a message that names the id and the capture. A
forbidden-set — answering such a request with an error instead of bytes — would
answer with a 404 the shim manufactured and blur the two failure modes, so the
log alone is the mechanism.

`--log-file` is the **documented last resort**, not the first choice: parse
`chunk downloaded chunk=<id>` and `no local reuse candidate -> download
chunk=<id>` out of the trace lines.

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

Per capture, against a **fresh scratch copy of that capture's `game/` tree** and
**its own listeners** (§6.3 for why):

| flag | why | which runs |
|---|---|---|
| `--game-path <scratch>/game` | never the bundle; the bundle must stay byte-identical | all |
| `--biz hk4e_global` | matches `MANIFEST.json → biz` | all |
| `--hyp-base`, `--sophon-base` | the listener's own origins | all |
| `--include "<the `path` field of `patchmaps/<id>.json>`" | exactly one path | all |
| `--purge-only` | the files-cleanup, without the repair loop | `removed` only |
| `--io-threads 1` | one planned file; determinism over throughput | all |
| `--max-files 1` | the backstop against a glob that matched more than intended — `*` crosses `/` | all but `removed` |
| `--fail-fast` | a shim fault should stop the run, not spend the budget discovering the same 404 on every remaining file: 17.6 s with it, 88.5 s without | all |
| `--output json` | NDJSON, so the driver reads records by name — `report`, `plan`, `summary` — rather than scraping `key=value` | all |
| `--repair-mode inplace` | the mode is **asked for**, so the run reports `inplace` / `temp` / `kept_bytes` — the counters that say which path ran | all but `removed` |
| `--scan` | the scan is what makes `damaged_chunks` appear, and it turns the download estimate from an approximation into the precise sum | all but `removed` |
| `--max-ram-bytes 1 GiB` | the budget is what makes `ram_bytes` appear; the value is large enough for the staged tier, which is the default | all but `removed` |
| `--max-write-bytes 1 TiB` | the ceiling is what makes `write_bytes` appear; the value never trips | all but `removed` |

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

- `files=1` and `est_download_bytes` equal to the patchmap's total → **the scope
  selected the right thing**;
- the real run's `SUMMARY download_bytes` equal to the same figure → **the repair
  did the right thing**.

With `--scan` the estimate is not an estimate. It is the precise sum over the
chunks that are **wrong where they belong** *and* have **no intact source** —
the two conditions together, because a chunk that is wrong but has a source is
copied rather than downloaded. For the bundle's fixture that is exactly the
patchmap's `totals.download_bytes`, so the pre-pass asserts an **equality**
rather than an approximation.

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

**Rust runs `#[test]`s in parallel threads.** A shared `game/` copy, a shared
listener and a shared request log mean concurrent `girpr` processes fighting over
one tree: two runs would patch the same path, one run's `must_not_fetch` would
see the other's requests, and a tree diff would compare a tree some other run had
already changed. The failure would be intermittent and would look like a product
bug.

So, **per capture:**

- a **fresh copy of the whole fixture `game/`** — not just the capture's path,
  because a capture is one path *out of* the fixture and the other nine are what
  "outside the scope is untouched" is measured against. ~213 MiB per run, removed on
  `Drop`;
- **its own listeners** (its own manifest and chunk ports, its own hyp/sophon
  origins), so the `build_id` invariant of §4.1 is per-capture too;
- **its own request log** and **its own log file** for the pre-pass;
- **its own before/after tree diff**, `rel -> (bytes, md5)`. MD5 rather than
  length and mtime, because "untouched" has to mean *byte-identical*: a run that
  rewrote a file to the same length would otherwise pass.

**The served-bytes cache stays shared** (§4.6) because it holds nothing
capture-specific.

### 6.3 The bundle is the fixture; a copy is the working tree

The bundle's `game/` is the **input**. The working tree is a scratch copy of it.
Each run touches exactly one path and leaves the rest alone, and the reuse map
stays anchored to the old version because a scoped run does not bump
`config.ini`. The bundle itself is never a working tree — pointing `--game-path`
at it in place is what turns a cleanup flag into a way to destroy the evidence.

---

## 7. The assertions

Three per capture, plus a structural set and the axes. None of them is optional;
the third is the one that earns the whole exercise.

| # | assertion | source |
|---|---|---|
| 1 | `SUMMARY download_bytes` equals the patchmap's total | `patchmaps/<id>.json` |
| 2 | the patched file's MD5 equals `expectations.result_md5` | same |
| 3 | **no id in `expectations.must_not_fetch` was requested** | the server's request log |
| 4 | exit `0`, `config_bumped=0`, and every path outside the scope is untouched on disk | `SUMMARY` + the per-run tree diff |
| 5 | **the capture paths are pairwise distinct** | `patchmaps/*.json` |
| 6 | **the two repair modes produce the same file, and the in-place one writes less** | a second run over a second copy |
| 7 | **the `expectations.summary` counters are what the run reports** | `patchmaps/<id>.json` |
| 8 | **the ordered tier downloads exactly the patchmap's total** | a run with `--max-ram-bytes 0` |
| 9 | **the discard mode produces the same file, and downloads all of it that the patch could have reused** | a run with `--repair-mode discard` |
| 10 | **the in-place path re-verifies: a damaged file is fixed, and only the damaged chunk is** | a second run over a damaged file |

Assertion 4's `config_bumped=0` is the tool stating that a scoped run
deliberately did not put the bytes in place; the tree diff is what proves it did
not delete anything either, and is the only check that the scope held across the
whole filesystem rather than just the repair plan. The diff is per run — before
this run against after this run — because §6.2 gives every capture its own copy
and a shared baseline would compare two different trees.

Assertion 5 is a cheap guard rather than a known bug: it is true for the
reference bundle (9 distinct paths) and `select`'s exclusion set keeps it true.
But a repeat would make the **second** run silently skip — the file is already at
the new version — and report `download_bytes=0` against a non-zero expectation,
which is a baffling failure to diagnose from the number that is wrong.

Assertion 6 is the **differential**, and it is the one that makes the space
claim checkable. Each repair capture is driven **twice**, on two copies of the
same fixture: once with `--repair-mode temp` and once with the default. The two
runs must produce the **same file** — the mode is a choice of how to get there,
not of what to produce — and the in-place run must write **strictly fewer**
bytes than the temp run, which writes the whole file. The offline fixture is
128 bytes, too small to tell "wrote 128" from "wrote 64"; here the difference
is megabytes, and the bridge is the only place that can show it.

The strict inequality is keyed on the run's own `kept_bytes`, not on the class's
name: a `moved_*` capture whose file keeps **nothing** (every chunk moves or is
fetched — `resources.assets` does) writes as much as the temp path does, and a
class-name rule would have said otherwise.

Nothing is relaxed for it. Assertion 1 still holds for **both** modes: a chunk
whose content the old file cannot supply cannot be "already correct" at its
destination, so it is always fetched, and a chunk it *can* supply is copied —
by either mode.

### 7.1 The figure assertion is exact, not approximate

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
than relaxing: it is what proves the new writer fetches the same set the patchmap
says a consumer must fetch, and not one chunk more.

### 7.1a The ordered tier, and the direction it picks

The staged tier stages the moving bytes, so **every** source survives and both
directions download the same bytes: the direction rule is invisible there. The
**ordered** tier stages nothing — each source is read from the file at the
moment it is written, and the write order is what keeps the sources alive.
`--max-ram-bytes 0` forces it ("stage nothing at all" is a mode, not a
shortage).

With the **right** direction the ordered tier downloads exactly the patchmap's
total, because every movable chunk's source survives. With a **wrong** one the
figure is larger: the chunks whose source the wrong order destroys have to be
downloaded too. That single equality is what pins the direction, and it is the
only place the rule is observable at all.

The two captures that carry a uniform shift are the load-bearing ones. On
`09801749.blk` the ascending order keeps all four movable chunks and the
descending one none; on `resources.assets` it is the other way round, with 27
kept. The bridge asserts the equality for every capture, because a wrong
direction turns it into a larger number and a failure that says so is the point.

**The mixed shift is the case the rule exists for**, and it is the one capture
where the equality does *not* hold: `moved_mixed`'s moves go both ways, so
neither order keeps every source — the descending one keeps 60 of the 67
movable chunks and the ascending one keeps 7 — and the ordered tier has to
fetch the seven it cannot salvage. So the assertion is an equality where the
direction keeps every source and a **strictly larger** figure where it cannot,
and the artifact's `keeps_ascending` / `keeps_descending` say which case each
capture is.

The ordered tier also reserves **no** RAM — it stages nothing — so
`ram_bytes` is `0`, against the patchmap's `ram_bytes_ordered`.

### 7.2 Per-class expectations

| class | what the assertion is |
|---|---|
| `identical` | nothing is downloaded and the file is not touched. It *is* in the manifest, so it is planned and then skipped on size+MD5 — `skipped=1 repaired=0 download_bytes=0`, exit `0`. The request log being empty is the assertion that carries it, since `download_bytes=0` is also what a broken run reports |
| `kept_in_place`, `moved_*`, `trim_*`, `zero_reuse_*` | the patchable classes and the zero-reuse ones are the load-bearing cases; assertions 1–3 all bite here |
| `added` | the path is absent from `game/` and present in the new manifest, so it is planned and downloaded in full. Assertion 2 plus `must_not_fetch`, which is empty |
| `removed` | no repair, so no byte total. It gets the `--purge-only` run of §5 and the purge-counter pre-pass of §6.1; the file is gone afterwards and its size is priced into `deleted_extra_bytes` |

The vocabulary is the repair spec's: a chunk is **kept** (already correct where
it belongs), **copied** (its bytes live elsewhere in the file) or **fetched**
(no usable local source). `kept_in_place` is not a reuse class — a same-offset
source *is* the chunk's own destination, so the chunk is kept rather than
copied, the payload is 0 and the staged tier stages nothing.

These live in `drive_capture` rather than in the tests, so a new capture id needs
no new assertion code. The patchable and zero-reuse classes must be **repaired**
(never skipped) and must show their bytes moving on disk.

**The `expectations.summary` block is asserted too.** The new repair spec
reports counters that are *conditional on a flag* — `inplace` / `temp` /
`kept_bytes` with `--repair-mode`, `ram_bytes` with `--max-ram-bytes`,
`damaged_chunks` with `--scan` and `write_bytes` with `--max-write-bytes` — so
the patchmap states them per flag and the run asks for all of them. The
patchmap's own `repair` block is the same numbers seen twice, and the bridge
asserts that the two agree.

**And the `repair` block is asserted against the run, field by field.** The
counts (`kept` + `copied` + `fetched` is the plan's length), the growth (the
difference between the file's two sizes) and the direction (the order that
keeps more sources alive) are all cross-checked. The **trim** is the one that
needed the log rather than a counter: the file ends at the target size either
way, so the outcome is invisible in the file's bytes and in the summary, and
the log is the only place it shows. So `trim_eager` must log "trimming the
dead tail" and must **not** log the deferred line, `trim_deferred` the other
way round, and the artifact's `repair.trim` says which one it expects.

The matrix and the patchmaps are also checked to agree **in both directions**,
and the `covered_by` row needs care: a class the matrix records as `covered_by`
has **no patchmap of its own**, and the row names the *covering* capture's id —
which must be a capture that does have one. A `captured` row names its own
capture; an `over_budget` or `unavailable` row has no capture id and carries a
reason; and no patchmap exists without a matrix entry.

### 7.3 The flag axes and the fault injections

Three of the repair spec's per-file decisions are **not** content decisions:
they come from a flag, a filesystem or an interruption, so the same file answers
them whatever the content is, and no capture class can force them. They are
covered here instead, and this is the list of where each one lives.

| what | how the bridge reaches it | what it proves |
|---|---|---|
| `--repair-mode temp` | the differential (§7, assertion 6) | the same file, the whole file written |
| `--repair-mode discard` | `the_discard_mode_downloads_the_whole_file_and_still_produces_it` | the same file, a full download, zero peak extra disk |
| `--max-ram-bytes 0` | `the_ordered_tier_downloads_exactly_the_patchmap_total` | the ordered tier, and the direction it picks |
| `--max-ram-bytes <small>` | `a_ram_budget_too_small_for_the_payload_degrades_to_the_ordered_tier` | a file that cannot get its reservation degrades to the ordered tier instead of failing, and says so |
| the target cannot be opened for writing | **not covered** — see below | the fallback to the temp path, and the same bytes |
| the in-place resume | `the_in_place_path_re_verifies_and_continues` | in place re-verifies and continues, and only the damaged chunk is redone |
| the temp resume by length | **not covered** — see below | a temp file that is already partly written is resumed, not restarted |
| the "redo the mismatching chunks" second pass | **not covered** — see below | only the named chunk is redone |
| the zero-fill of a hole | **not covered** — see below | the hole is zero-filled, and the file still verifies |

**Why four of them are not covered here.** Each needs something the bundle
cannot provide, and saying so is better than a case that pretends:

- **The target cannot be opened for writing.** The mechanism is not the
  read-only attribute: `girpr` **clears** that attribute and retries the open
  (the same thing Starward does), so a read-only file is repaired in place, by
  design. The fallback needs an open that fails for a reason clearing the
  attribute cannot fix — an exclusive lock, which is a Windows fixture rather
  than a bundle case.
- **The temp resume by length.** The temp path resumes by the length of the
  `{path}_tmp` file it left behind, and a run that was killed mid-write is the
  only thing that leaves one. The bridge has no way to kill a run at a chosen
  byte offset, and a fixture that fakes it would test the fake rather than the
  resume.
- **The "redo the mismatching chunks" second pass.** It fires when the
  whole-file MD5 disagrees but every chunk's own MD5 agrees, which needs a
  manifest whose coverage does not tile its file. The server's manifests always
  do.
- **The zero-fill of a hole.** The same thing: a hole is a manifest that does
  not tile, and the server's manifests do not have holes.

The last two are the plan's **D9** — a malformed manifest rather than a
malformed file — and they belong to a synthetic manifest, not to a bundle.

---

## 8. Failure modes this design must not have

- **Degraded-but-green.** A run that reused nothing because the local manifest did
  not load reports the same exit code and a plausible byte count (§4.3). The
  byte comparison against the patchmap catches it; so must the assertion that
  `local_manifest` did not come back `unavailable`, and that the old tag was
  actually served.
- **A glob that matches more than intended.** `*` crosses `/`, and the fixture
  contains paths with spaces in them. `--max-files 1` is the cheap answer.
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
    server.rs                   Bundle, Bridge, routes, request log, error shapes
```

Both are **test-only**; nothing under `src/` changes. `src/sophon.rs` carries a
note that a trace log can be parsed for `chunk downloaded chunk=<id>` — that note
stays accurate, and that line remains the documented last resort rather than what
the bridge actually reads.

---

## 10. Non-goals

- **Capturing.** `gircap capture` does that, with the network, deliberately.
- **Re-verifying the bundle.** `gircap verify` and `gircap materialize` own that
  half; the bridge consumes their verdict (§3.2), it does not recompute it.
- **A second mock server.** `tests/common/mod.rs` already covers the synthetic
  path with a two-file fixture. The bridge is the opposite: real bytes, from
  disk, at production scale.
- **Automating the bundle preparation.** A bundle is captured when someone wants
  one, by hand.
- **Sharing code with `gircap`.** A server that replays a bundle's bytes shares no
  knowledge of how the bundle was made. A shared crate would make two tools that
  must stay independent depend on each other to prove one of them works.