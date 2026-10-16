# `docs/06-artifact-bridge-spec.md` — serving a capture bundle to `girpr`

**Status: implemented.** `tests/artifact_bridge.rs` +
`tests/artifact_bridge/server.rs` in the `girpr` package. The build order and
what each milestone decided are in
[`07-artifact-bridge-implementation-plan.md`](07-artifact-bridge-implementation-plan.md);
§0.1 of that file is the only place allowed to record what changed from what is
written below, and it lists two places this document disagrees with itself.

This document states what the harness does and what it does not do. The
implementation is written against it, so the contract stands as written even
where the implementation had to choose.

**Scope:** `girpr` (the main crate) only, plus its test tree. It names
`tools/capture` because that is where the input comes from; it changes nothing
there.

**Why it exists.** `tools/capture` writes a self-contained bundle per version
pair: a real `game/` fixture holding one file per test-matrix variant, the API
bodies and manifests behind them, every chunk blob, `expected/` ground truth,
and a `patchmaps/<id>.json` per capture. Three consumption paths work — inspect
the metadata, rebuild old/new offline, prove the bundle's own integrity. The
fourth does not: **run `girpr --game-path <artifact>/game` and assert it patches
to the new version.** That path is what this spec designs.

---

## 1. What the bridge is, and what it is not

A **loopback server that replays one artifact bundle**, plus a **driver that
runs the real `girpr` binary against it and checks the result**.

| | the bridge does | the bridge does not |
|---|---|---|
| network | loopback only | touch the internet, ever |
| disk | reads the bundle, writes a scratch copy of `game/` | write inside the bundle |
| the bundle | consumes it | regenerates, repairs, or re-captures it |
| `girpr` | runs the real binary, reads `REPORT`/`PLAN`/`SUMMARY` | imports `girpr` as a library, links it, or reimplements its logic |

Preparing a bundle is a **separate, human-run, network-using step** that already
exists — `gircap capture`. The bridge begins after it. That split is what makes
"offline" a property of the design rather than a promise: the bridge never
captures, so it never needs the internet.

**Independence is one-directional, and the point is to not couple.** `gircap`
must not depend on `girpr` and must not invoke it, so the bridge lives on the
`girpr` side. That is a design constraint, not a mechanical one: nothing here
decides whether a given piece of coupling exists, and a test that did would only
be a test of the test. The server may be reusable by the capture's own tests, but
nothing in the capture's tree reaches for the `girpr` binary.

## 2. Placement

**A `#[ignore]`d integration test in the `girpr` package** —
`tests/artifact_bridge.rs`, with the server in a module beside it — not a new
workspace member and not a `src/bin/`.

The deciding constraint is Cargo's `CARGO_BIN_EXE_<name>`, which is set only
when building a test, bench or example target. An integration test in the
package that owns the binary gets the absolute path to the binary Cargo just
built, for free. A separate crate does not, and neither does a plain
`src/bin/` helper: both start by hand-rolling discovery or demanding a prior
`cargo build -p girpr` plus a guess at `target/{debug,release}/girpr[.exe]`.

That matters because the whole point of the bridge is to prove that **`--include`
and `--hyp-base` reach `HypClient`** through the real flag-parsing path. Calling
`repair::run` with a hand-built `RunCtx` would exercise the library and prove
nothing about the surface — the lesson `docs/05` M7 already paid for once.

| alternative | why not |
|---|---|
| `tools/e2e` crate | earns its keep only if it also captures bundles, which `gircap capture` already does. Costs binary discovery, a workspace member, and README/doc rows |
| `src/bin/*.rs` | same discovery problem, inside the shipping package |
| feature-gated test (`#![cfg(feature)]`) | a broken bridge would stop compiling with no default build noticing; compile-time coverage of the harness is worth more than a few hundred lines of test compilation |

**The server is its own module, reached from the bridge test alone.**

```rust
// tests/artifact_bridge.rs
#[path = "artifact_bridge/server.rs"]
mod server;
```

That `#[path]` is **required**, not decorative. For a crate root at
`tests/artifact_bridge.rs`, `mod server;` resolves relative to `tests/` — so it
looks for `tests/server.rs` or `tests/server/mod.rs`, and
`tests/artifact_bridge/server.rs` is not on that path. Both alternatives were
tried: `mod server;` with `tests/artifact_bridge/server.rs` and with
`tests/artifact_bridge/server/mod.rs` both fail with `E0583: file not found for
module 'server'`. The `#[path]` spelling is the one that works, and it is the one
`tests/repair_offline.rs` already uses for `mod common`.

The point of the module being its own is compilation isolation: `mod server;` in
`repair_offline.rs` or `cli_contract.rs` would put a several-hundred-line replay
server into both of those compilation units, where it would be recompiled on
every change to either and whose warnings would land on files that do not use it.
`tests/common/` is already shared by those two, which is fine — it is a mock, and
it is small. This one is not small and neither test wants it.

## 3. Two independent gates, both required

**Gate 1 — `#[ignore]` on every bridge test, because the suite is expensive.**
One capture is a `--print-plan` pre-pass plus a real run, and each run fetches
four manifests — ~20 MB of real bundle bytes — before it can plan a single file.
The suite is ~18 cases and ~2 minutes. That is worth paying when something has
gone wrong and not worth paying on every `cargo test`.

So it runs **by hand, deliberately, and only when the repair spec is suspected
to have regressed**:

```
cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
```

Nothing else runs it. `cargo test` still *compiles* the file, which is what
keeps it from rotting against `RunCtx`, `PathScope`, and the `SUMMARY`
contract: a broken bridge fails to build rather than failing silently.

**Gate 2 — a missing prerequisite is an error, not a skip.** This follows
directly from Gate 1. Because the suite only ever runs intentionally, nobody
types `--ignored` here by accident, so the hazard a skip would guard against
does not exist — while its opposite does. "I ran the check and it found nothing
to check" must never render as a green line. A run without a bundle **fails**,
and the failure names the exact command that produces one.

> **Correction.** An earlier draft of this section specified the opposite —
> "skip, never fail, when there is no bundle" — on the reasoning that
> `cargo test -- --ignored` is a thing people type by habit and a red run there
> would put them off. That reasoning applies to a suite wired into a default
> run. This one is deliberately kept out of every default run and priced so that
> only a human starts it, which reverses the argument: **an absent bundle is
> missing input for a check someone asked for**, and it must say so loudly.

Resolution order, unchanged:

1. `GIRPR_ARTIFACT_BUNDLE` — an explicit path wins, always;
2. otherwise discover under `artifacts/` — the primitive for this already exists
   (`gircap list --root artifacts`, which now reports one `bundle` record per
   bundle on stdout rather than only on stderr);
3. otherwise **fail**, naming the reason and the exact command:
   `cargo run -p gircap -- capture --out artifacts`.

The same rule applies one level down: a capture the suite names must exist in
the bundle. A class the bundle's own matrix records as *not captured, with a
reason* is a fact rather than a missing prerequisite — the case stands down out
loud — but a capture that is simply missing fails.

**Precondition.** Before any run, the bundle's own integrity is established —
`gircap verify --artifact <dir>`, exit `0`. `00-spec.md` §8.2 says why: it makes
a failing check rule the bundle out as the cause instead of leaving it as the
first suspect. If `gircap` is not built, skip the precondition rather than
inventing a substitute; the bridge does not re-implement `verify`.

## 4. The server

An ephemeral `127.0.0.1:<port>` listener per route family. **Four families, six
listeners** — two for the blob namespaces (§4.1), plus one each for the HoYoPlay
JSON API, the Sophon `getBuild`, manifests and chunks.

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
over: the port is not known until the listener binds, and A16 requires the
bundle to stay byte-identical so `gircap verify` still passes afterwards.

So: parse `api/getBuild.<tag>.json`, replace the **scheme and authority** of
every `manifests[].chunk_download.url_prefix` and
`manifests[].manifest_download.url_prefix` with the listener's own, re-serialize,
serve that. **In memory only — the bundle is opened read-only and never written.**

**Keep the path.** Replacing the whole prefix with one origin would collapse the
manifest and chunk namespaces into a single root, which contradicts the route
table above and — worse — would stop the two namespaces being *distinguishable*.
They stay distinguishable **by port**: one listener on the manifest prefix's
port, one on the chunk prefix's port. The shapes `girpr` builds are then

```
manifest : <origin>/client_app/sophon/manifests/cxhpq4g4rgg0/<build_id>/manifest_<16hex>_<32hex>
chunk    : <origin>/client_app/sophon/chunks/cxhpq4g4rgg0/<build_id>/<16hex>_<32hex>
```

Each blob listener **takes the last two path segments and ignores the rest**. The
route is then indistinguishable from production's, and the bridge still proves
`girpr` builds URLs from `url_prefix` + `id` alone — the point of §2.

The invariant this buys is the one worth stating out loud:

> **every blob request's `<build_id>` must equal the build whose manifest
> declared that id.**

That is the namespacing rule of `00-spec.md` §3.7, measured and now assertable.
The same chunk id under the other build's prefix is a `404` (`api/error_shapes.json`,
`blob_wrong_build`), so a shim that got the prefixes crossed would produce a
`404` it had manufactured itself — which is precisely the failure the two ports
exist to make impossible.

The rewrite is otherwise deliberately narrow. Everything else in the body passes
through untouched, and two fields are **load-bearing and must not change**:

- `manifest.checksum` — `sophon::fetch_manifest` MD5-verifies the *decompressed*
  manifest against it. Alter it and the run dies at metadata with exit `2`.
- `manifest.id` — it is the URL path segment and the `url_suffix` pairing.

`url_suffix` is irrelevant: `sophon::join_url` builds `{prefix}/{id}` and never
reads it.

### 4.2 How a manifest id resolves to a file

The live ids are `manifest_<16hex>_<32hex>`; the bundle stores archives under
*field* names (`manifests/<tag>/game.manifest.zst`). So there is a translation,
and **build the `id → (tag, field)` map while rewriting the body** — the loop over
`manifests[]` is already there, and the rewrite is where every id in the bundle
passes through. **404 anything absent from that map.**

Doing it with a filesystem scan over `manifests/*/*.meta.json` instead would be
slower and would answer a different question: it would find any archive in the
bundle rather than the one *this* build declared, which is the distinction the
`build_id` invariant above is about.

### 4.3 `getBuild`, and what happens when it is wrong

**A tagless request serves `api/getBuild.<versions.new.tag>.json`.**

**An unserved tag returns the recorded `retcode -202` body from
`api/error_shapes.json`** (`getbuild_unknown_tag`) — **not one the shim made up.**
The second `getBuild` is `?tag=<game_version from config.ini>`, and it is the one
that populates the chunk-reuse map.

The consequence of getting it wrong is measured, and it is the worst failure in
this document because it is **green**:

> A tag the shim does not serve degrades to the local-build fallback: the run
> proceeds with an **empty reuse map**, downloads every chunk, and **still exits
> `0`** — reporting `local_manifest=unavailable`, `download_bytes=913040` instead
> of `274426`, and requesting the `must_not_fetch` chunk.

That is "degraded-but-green" (§7) confirmed rather than hypothesised. Two
assertions catch it, and both are required: the byte comparison of §6.1, and the
assertion that `local_manifest` did not come back `unavailable`.

### 4.4 Not-found is measured, and replayed — not chosen

This is the part everything else in §4 rests on, and it was previously stated as
though it were a free choice ("404 an unknown chunk id"). It is not.

Measured against the live endpoints on 2026-10-16, the two route families answer
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
   synthesize only the fields that document marks volatile — `RequestId`,
   `HostId`, `Key`, and the body length that follows from `Key`. `girpr` calls
   `.error_for_status()` and never reads the body, so **status and content-type
   are the contract and the body is fidelity**, for a human reading the shim's
   log.
2. **A bundle that predates the file has no `error_shapes.json`.** The shim then
   404s with `application/xml` — the measured shape — and the bridge **logs that
   the not-found assertions were skipped for lack of evidence** rather than
   passing them silently. Inventing the statuses would make the document's claim
   about the API unfounded.
3. **Do not reproduce the old advice about waiting.** `girpr` retries only
   `408 | 429 | 5xx` now (`sophon::retryable`), so a 404 is asked **once, with no
   backoff**. The honest 404 is genuinely cheap: measured, a run against a real
   bundle with one chunk 404'd went from **88.5 s and 25 requests** to roughly
   **10 s and one request**, and to **~0 s** under `--fail-fast`, with the exit
   code unchanged at `3`.

### 4.5 Which manifests to serve

**Index whatever `manifests/<tag>/` holds** — that is, `game` plus whichever
audio fields the capture stored. Do not hardcode a language.

*(measured)* the `getBuild` body carries five manifests for this biz: `game`,
`zh-cn`, `en-us`, `ja-jp`, `ko-kr`. `sophon::select_manifests` drops every field
whose name is five or ten characters and contains a hyphen, then re-adds only the
selected languages; the bundle's scan file selects `en-us`, and only the blobs the
capture stored are present. So a run fetches **`game` + `en-us`** — four
manifests per run, plus four more for the `--print-plan` pre-pass (§5.1) — and the
other three are never requested.

The rule generalises to whatever the bundle holds, so the run's selection and the
bundle's contents decide the set between them. **Keep the `--audio` warning:**
adding a language widens the selection, and a bundle that does not carry that
field's manifest will 404 on it — which, after §4.4, fails immediately and
visibly rather than after fifteen seconds of silence.

### 4.6 Cache served manifest bytes

Each run fetches four manifests and the `manifests/` directory is ~40 MB of the
~130 MB bundle; across eight runs that is ~320 MB re-read for nothing. They are
immutable per bundle, so cache them.

**The cache lives outside the listeners and is shared across them** (§2, §5,
§6.3): it holds bytes that are a function of the bundle alone, so nothing about
per-capture isolation is lost by sharing it, and every capture pays the disk read
once instead of eight times.

### 4.7 The request log is not optional, and it is the primary source

`SUMMARY` is byte-level. For `expectations.must_not_fetch` — "no chunk id in
this list was requested", the assertion that catches a broken reuse path and is
invisible in a byte count — **the server's own record of what it was asked for is
the clean source**, and it should be kept as such: `Vec<(kind, id)>` behind a
mutex, with the option of a **forbidden set** per run, so a request for a
forbidden id returns an error rather than bytes and turns a silent reuse
regression into a loud failure. Which of the two a given run wants is a decision
for the implementation; keeping the log is not.

`--log-file` is the **documented last resort**, not the first choice: parse
`chunk downloaded chunk=<id>` and `no local reuse candidate -> download
chunk=<id>` out of the trace lines.

**Match on the download record, never on the bare id.** A reused chunk is logged
too, so a substring search for an id finds it in the "reused it" line and
false-positives. That is not hypothetical: the first attempt at this check
failed on 19 chunks that were never downloaded, for exactly that reason.

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
| `--include "<the `path` field of `patchmaps/<id>.json>`" | exactly one path | all but `removed` |
| `--purge-only` | the files-cleanup, without the repair loop | `removed` only |
| `--io-threads 1` | one planned file; determinism over throughput | all |
| `--max-files 1` | the backstop against a glob that matched more than intended — `*` crosses `/` | all but `removed` |
| `--fail-fast` | a shim fault should stop the run, not spend the budget discovering the same 404 on every remaining file: 17.6 s with it, 88.5 s without | all |
| `--output json` | NDJSON, so the driver reads records by name — `report`, `plan`, `summary` — rather than scraping `key=value` | all |

**`--audio` is omitted, deliberately.** An explicit audio selection under an
active path scope is refused when the scan file falls outside that scope, and
`GenshinImpact_Data/Persistent/audio_lang_14` does. Omitting the flag lets the
run autodetect from the scan file and resolve to `en-us` — the state the bundle
was captured in.

**No cleanup flag** for the repair runs: a scoped cleanup sees only the one
selected path, which the manifest contains, so it has nothing to delete. The one
capture that *wants* a cleanup is `removed`, and it is the row that says so.

### 5.1 A free pre-pass

`--print-plan` returns before `create_dir_all` and emits a `plan` record at zero
disk cost:

```
PLAN files=1 est_download_bytes=274426 purge_files=0 purge_bytes=0 max_chunk_bytes=… scoped=1
```

Run it once per capture first. It separates the two failure modes that otherwise
arrive tangled in one red run:

- `files=1` and `est_download_bytes` equal to the patchmap's total → **the scope
  selected the right thing**;
- the real run's `SUMMARY download_bytes` equal to the same figure → **the
  repair did the right thing**.

When the first fails, the glob is wrong. When the second fails, reuse is wrong.
`--plan-out` gives the per-file JSON for the same run.

**`removed` is the exception, and the rule above is false for it.** A `removed`
patchmap carries `totals: null`, because there is nothing to patch, and its
pre-pass prints

```
PLAN files=0 est_download_bytes=0 purge_files=1 purge_bytes=1731133 scoped=1
```

So for that capture the **purge counters are the pre-pass assertion**: `files=0`
(the path is correctly absent from the repair plan) and `purge_files=1
purge_bytes=1731133` (the cleanup found it and priced it). Asserting
`files=1` here would fail forever for a reason that has nothing to do with the
bridge.

## 6. The assertions

Three per capture, plus two structural. None of them is optional; the third is
the one that earns the whole exercise.

| # | assertion | source |
|---|---|---|
| 1 | `SUMMARY download_bytes` equals the patchmap's total | `patchmaps/<id>.json` |
| 2 | the patched file's MD5 equals `expectations.result_md5` | same |
| 3 | **no id in `expectations.must_not_fetch` was requested** | the server's request log |
| 4 | exit `0`, `config_bumped=0`, and every path outside the scope is untouched on disk | `SUMMARY` + a before/after tree diff |
| 5 | **the eight capture paths are pairwise distinct** | `patchmaps/*.json` |

Assertion 4's `config_bumped=0` is the tool stating that a scoped run
deliberately did not put the bytes in place; the tree diff is what proves it did
not delete anything either, and is the only check that the scope held across the
whole filesystem rather than just the repair plan. **The diff is per run** — before
this run against after this run — because §6.3 gives every capture its own copy
and a shared baseline would compare two different trees.

Assertion 5 is a cheap guard rather than a known bug: it is true for the
reference bundle (8 distinct paths) and `select`'s exclusion set keeps it true.
But a repeat would make the **second** run silently skip — the file is already at
the new version — and report `download_bytes=0` against a non-zero expectation,
which is a baffling failure to diagnose from the number that is wrong.

### 6.1 The figure assertion is exact, not approximate

`girpr`'s `download_bytes` counts the actual bytes of files that were promoted,
giving back the bytes of any attempt that failed and was retried. The patchmap's
total is `Σ compressed_size` over the chunks with no `(uncompressed_md5,
uncompressed_size)` match on the old side. A6 asserts `len(blob) ==
compressed_size`, and the runtime ledger folds the real measurement back into
the manifest's reservation, so the two figures are equal by construction — not
approximately equal, and not equal only when nothing retries.

### 6.2 Per-class notes

| class | the assertion is different in kind |
|---|---|
| `identical` | nothing is downloaded and the file is not touched. It is in the manifest, so it **is** planned and then skipped on size+MD5 — `skipped=1 repaired=0 download_bytes=0`, exit `0` |
| `partial_*` | the three patchable classes and `zero_reuse_*` are the load-bearing cases; assertions 1–3 all bite here |
| `added` | the path is absent from `game/` and present in the new manifest, so it is planned and downloaded in full. The assertion is 2 plus `must_not_fetch`, which is empty |
| `removed` | no repair, so no byte total. It gets the `--purge-only` run of §5 and the purge-counter pre-pass of §5.1 |

### 6.3 Isolation is per capture, because the tests run in parallel

**Rust runs `#[test]`s in parallel threads.** That is not a detail of the harness;
it is the reason the following is a requirement rather than a nicety.

A shared `game/` copy, a shared listener and a shared request log mean concurrent
`girpr` processes fighting over one tree. Two runs would patch the same path, one
run's `must_not_fetch` would see the other's requests, and a tree diff would
compare a tree some other run had already changed. The failure would be
intermittent and would look like a product bug.

So, **per capture:**

- a **fresh copy of that capture's `game/` tree** — and of the whole fixture
  `game/`, because a capture is one path *out of* the fixture and the other seven
  are what "outside the scope is untouched" is measured against;
- **its own listeners** (its own manifest and chunk ports, its own hyp/sophon
  origins), so the `build_id` invariant of §4.1 is per-capture too;
- **its own request log**;
- **its own before/after tree diff**.

Copying the fixture eight times takes 0.035 s, so this costs nothing measurable
and removes a whole class of flakiness. **The served-bytes cache stays shared**
(§4.6) because it holds nothing capture-specific.

One run per capture is kept regardless: it is what makes `download_bytes`
attributable to a single capture.

### 6.4 The bundle is the fixture; a copy is the working tree

The bundle's `game/` is the **input**. The working tree is a scratch copy of it.
Each run touches exactly one path and leaves the rest alone, and the reuse map
stays anchored to the old version because a scoped run does not bump
`config.ini`. The bundle itself is never a working tree — pointing `--game-path`
at it in place is what turns a cleanup flag into a way to destroy the evidence.

## 7. Failure modes this design must not have

- **Degraded-but-green.** A run that reused nothing because the local manifest
  did not load reports the same exit code and a plausible byte count. Measured,
  in §4.3. The pre-flight comparison against the patchmap catches it; so must the
  assertion that `local_manifest` did not come back `unavailable`, and that the
  old tag was actually served.
- **A glob that matches more than intended.** `*` crosses `/`, and the fixture
  contains paths with spaces in them. `--max-files 1` is the cheap answer.
- **A shim that quietly serves less than the bundle.** Every served byte must
  come from a file the bundle vouches for; anything else is a different test. The
  same goes for a shim that *invents* a failure shape it has no evidence for.
- **Parallel runs sharing a tree.** §6.3.
- **A fixture mutated in place.** The bundle must still pass `verify`
  afterwards. That is the precondition of §3 and the point of §6.4.
- **A bridge that reports success when it checked nothing.** Gate 2 exists for
  this: the suite runs only when a human asks for it, so a missing bundle — or a
  missing capture — is missing input, and it fails with the command that
  produces it.

## 8. Non-goals

- **Capturing.** `gircap capture` does that, with the network, deliberately.
- **Re-verifying the bundle.** `gircap verify` and `gircap materialize` own that
  half; the bridge consumes their verdict (§3), it does not recompute it.
- **A second mock server.** `tests/common/mod.rs` already covers the synthetic
  path with a two-file fixture. The bridge is the opposite: real bytes, from
  disk, at production scale.
- **Automating the bundle preparation.** A bundle is captured when someone wants
  one, by hand.
- **Sharing code with `gircap`.** A server that replays a bundle's bytes shares
  no knowledge of how the bundle was made. A shared crate would make two tools
  that must stay independent depend on each other to prove one of them works.