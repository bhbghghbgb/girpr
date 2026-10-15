# `docs/06-artifact-bridge-spec.md` — serving a capture bundle to `girpr`

**Status: spec. Nothing here is implemented.** It states what a harness must do
and what it must not do, so the implementation can be written against a contract
rather than against the shape of whatever came first. No code, no module tree,
no milestone split — those belong to the implementation.

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

**Independence is one-directional and load-bearing.** `gircap` must not depend on
`girpr` and must not invoke it. So the bridge lives on the `girpr` side. Its
server may be reusable by the capture's own tests, but nothing in the capture's
tree may reach for the `girpr` binary — the acceptance grep for `girpr` in
`tools/capture/src/` and its `Cargo.toml` must keep returning doc comments only.

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

The `tests/common/` directory is shared by `repair_offline.rs` and
`cli_contract.rs`; the server belongs in its own file pulled in by the bridge
test alone, so the fifteen existing offline tests neither recompile it nor see
its warnings. Adding a row to the README's entry-point table is part of the
implementation, not of this spec.

## 3. Two independent gates, both required

**Gate 1 — `#[ignore]` on every bridge test.** Never runs under `cargo test`.
Still compiles, which is what keeps it from rotting against `RunCtx`,
`PathScope`, and the `SUMMARY` contract. Invoked as:

```
cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
```

**Gate 2 — skip, never fail, when there is no bundle.** `cargo test -- --ignored`
is a thing people type, and it must not turn red because nobody has captured
anything. Resolution order:

1. `GIRPR_ARTIFACT_BUNDLE` — an explicit path wins, always;
2. otherwise discover under `artifacts/` — the primitive for this already exists
   (`gircap list --root artifacts`);
3. otherwise print the reason and the exact command that would produce one, and
   return success.

The skip message is part of the contract. A bridge that fails on a missing input
teaches people to run it with `--ignored` forever and stop reading.

**Precondition.** Before any run, the bundle's own integrity is established —
`gircap verify --artifact <dir>`, exit `0`. `00-spec.md` §8.2 says why: it makes
a failing check rule the bundle out as the cause instead of leaving it as the
first suspect. If `gircap` is not built, skip the precondition rather than
inventing a substitute; the bridge does not re-implement `verify`.

## 4. The server

An ephemeral `127.0.0.1:<port>` listener. Four route families.

| request | served from | rewritten? |
|---|---|---|
| `GET <hyp>/getGameConfigs` | `api/getGameConfigs.json` | no |
| `GET <hyp>/getGameBranches` | `api/getGameBranches.json` | no |
| `GET <hyp>/getGameDeprecatedFileConfigs` | `api/getGameDeprecatedFileConfigs.json` | no |
| `GET <sophon>/getBuild[?tag=]` | `api/getBuild.<tag>.json` | **yes** |
| `GET <manifest-prefix>/<manifest-id>` | `manifests/<tag>/<field>.manifest.zst` | address only |
| `GET <chunk-prefix>/<chunk-id>` | `chunks/blobs/<chunk-id>.zst` | address only |

### 4.1 The base swap happens in memory, and only in memory

`--hyp-base` / `--sophon-base` redirect the **metadata calls only**. Manifest and
chunk URLs are built by `sophon::join_url` from a `url_prefix` read out of the
**response body**, so a body served verbatim sends `girpr` to the production CDN
regardless of the flags. Rewriting the bundle on disk is not an option twice
over: the port is not known until the listener binds, and A16 requires the
bundle to stay byte-identical so `gircap verify` still passes afterwards.

So: parse `api/getBuild.<tag>.json`, replace every
`manifests[].chunk_download.url_prefix` and `manifests[].manifest_download.url_prefix`
with the listener's own origin, re-serialize, serve that. **In memory only —
the bundle is opened read-only and never written.**

The rewrite is deliberately narrow. Everything else in the body is passed through
untouched, and two fields are **load-bearing and must not change**:

- `manifest.checksum` — `sophon::fetch_manifest` MD5-verifies the *decompressed*
  manifest against it. Alter it and the run dies at metadata with exit `2`.
- `manifest.id` — it is the URL path segment and the `url_suffix` pairing.

`url_suffix` is irrelevant: `sophon::join_url` builds `{prefix}/{id}` and never
reads it.

### 4.2 What the server must get right, and how it fails when it does not

**Serve both tags.** `girpr` asks for `getBuild` twice — once with no `tag` for
the latest build, once with `tag=<game_version from config.ini>` for the local
one. The second is what populates the chunk-reuse map, and it is fetched
best-effort: on failure the run logs a warning, proceeds with an **empty reuse
map**, downloads every chunk, and **still exits `0`**. Every reuse assertion
then fails at once with nothing pointing at the cause. The server therefore has
to answer `?tag=7.0.0`, and the bridge has to assert that it did — the request
log (§4.3) is where that assertion comes from.

**404 an unknown chunk id.** A chunk the bundle does not hold must fail fast and
visibly. Left alone, `sophon::fetch_chunk_bytes` retries five times with 1–5 s
linear backoff: roughly fifteen seconds of silence per chunk, repeated for every
chunk of the file.

**Serve only what the run will ask for, and know why that is enough.** A live
`getBuild` body for this biz carries five categories — `game`, `zh-cn`, `en-us`,
`ja-jp`, `ko-kr`. `sophon::select_manifests` drops every audio field whose name
is five or ten characters and contains a hyphen, then re-adds only the selected
languages; the fixture's scan file selects `en-us`. So the run fetches exactly
`game` and `en-us`, and the bundle stores exactly those two manifests per tag.
The other three are never requested. This holds only while the run does not pass
`--audio`: adding a language adds a manifest the bundle does not have.

**Cache served manifest bytes.** Each run fetches four manifests and the
`manifests/` directory is ~43 MB of the ~135 MB bundle. Across eight runs that
is ~344 MB re-read from disk for nothing. They are immutable per bundle.

### 4.3 The request log is not optional

`SUMMARY` is byte-level. Per-chunk detail exists in the binary only as `trace`
lines in `<exe-dir>/logs/girpr_<ts>.log`. For `expectations.must_not_fetch` —
"no chunk id in this list was requested", the assertion that catches a broken
reuse path and is invisible in a byte count — the server's own record of what it
was asked for is the clean source, and it should be kept as such:

`Vec<(kind, id)>` behind a mutex, with the option of a **forbidden set** per
run: a request for a forbidden id returns an error rather than bytes, turning a
silent reuse regression into a loud failure. Which of the two a given run wants
is a decision for the implementation; keeping the log is not.

## 5. The driver

One run per capture. That is what makes a byte total attributable: a single run
over all of them sums to the same number but cannot be compared per capture.

Per capture, against a scratch copy of `game/`:

| flag | why |
|---|---|
| `--game-path <scratch>/game` | never the bundle; the bundle must stay byte-identical |
| `--biz hk4e_global` | matches `MANIFEST.json → biz` |
| `--hyp-base`, `--sophon-base` | the listener's origin |
| `--include "<patchmaps/<id>.json → path>"` | exactly one path |
| `--io-threads 1` | one planned file; determinism over throughput |
| `--max-files 1` | the backstop against a glob that matched more than intended — `*` crosses `/` |

**`--audio` is omitted, deliberately.** An explicit audio selection under an
active path scope is refused when the scan file falls outside that scope, and
`GenshinImpact_Data/Persistent/audio_lang_14` does. Omitting the flag lets the
run autodetect from the scan file and resolve to `en-us` — the state the bundle
was captured in.

**No cleanup flag** for the repair runs: a scoped cleanup sees only the one
selected path, which the manifest contains, so it has nothing to delete. The one
capture that *wants* a cleanup is `removed`, and it gets its own run (§6.4).

### 5.1 A free pre-pass

`--print-plan` returns before `create_dir_all` and emits

```
PLAN files=1 est_download_bytes=274426 purge_files=0 purge_bytes=0 max_chunk_bytes=… scoped=1
```

at zero disk cost. Run it once per capture first. It separates the two failure
modes that otherwise arrive tangled in one red run:

- `files=1` and `est_download_bytes` equal to the patchmap's total → **the scope
  selected the right thing**;
- the real run's `SUMMARY download_bytes` equal to the same figure → **the
  repair did the right thing**.

When the first fails, the glob is wrong. When the second fails, reuse is wrong.
`--plan-out` gives the per-file JSON for the same run.

## 6. The assertions

Three per capture, plus one structural. None of them is optional; the third is
the one that earns the whole exercise.

| # | assertion | source |
|---|---|---|
| 1 | `SUMMARY download_bytes` equals the patchmap's total | `patchmaps/<id>.json` |
| 2 | the patched file's MD5 equals `expectations.result_md5` | same |
| 3 | **no id in `expectations.must_not_fetch` was requested** | the server's request log |
| 4 | exit `0`, `config_bumped=0`, and every path outside the scope is untouched on disk | `SUMMARY` + a before/after tree diff |

Assertion 4's `config_bumped=0` is the tool stating that a scoped run
deliberately did not put the bytes in place; the tree diff is what proves it did
not delete anything either, and is the only check that the scope held across the
whole filesystem rather than just the repair plan.

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

### 6.3 The bundle is the fixture; a copy is the working tree

`game/` is copied to scratch once, and the runs accumulate on that copy: each run
touches exactly one path, leaves the rest alone, and the reuse map stays anchored
to the old version because a scoped run does not bump `config.ini`. The bundle
itself is never a working tree — pointing `--game-path` at it in place is what
turns a cleanup flag into a way to destroy the evidence.

### 6.4 `removed` needs a different run, and a different assertion

A `removed` path is on disk and absent from the new manifest. It is therefore
absent from every repair plan, and the only work that can touch it is the
files-cleanup. It gets a cleanup-only run scoped to that one path, and its
assertion is that the path is **gone afterwards** and that nothing was
downloaded — not a byte total, because there is no patch to cost.

Whether that run is currently accepted is a question for `girpr`'s own
behaviour, not for this document.

## 7. Failure modes this design must not have

- **Degraded-but-green.** A run that reused nothing because the local manifest
  did not load reports the same exit code and a plausible byte count. The
  pre-flight comparison against the patchmap catches it; so must the assertion
  that the old tag was actually served.
- **A glob that matches more than intended.** `*` crosses `/`, and the fixture
  contains paths with spaces in them. `--max-files 1` is the cheap answer.
- **A shim that quietly serves less than the bundle.** Every served byte must
  come from a file the bundle vouches for; anything else is a different test.
- **A fixture mutated in place.** The bundle must still pass `verify`
  afterwards. That is the precondition of §3 and the point of §6.3.
- **A bridge that fails when the bundle is absent.** Gate 2 exists for this.

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