# Implementation plan — `gircap`↔`girpr` e2e artifact bridge

Build order, module tree, acceptance checks and the open questions worth
settling before milestone 1 starts. Companion:
[`06-artifact-bridge-spec.md`](06-artifact-bridge-spec.md) — *what* the harness
does and must not do. This file is *how* it gets built, and the only place
allowed to record what landed.

Scope: the `girpr` crate and its test tree. `tools/capture` is named as the
input's origin and is **not modified, not depended on, not imported**.

---

## 0. Decisions

| decision | value | why |
|---|---|---|
| placement | `#[ignore]`d `tests/artifact_bridge.rs` + `tests/artifact_bridge/server.rs` | spec §2; `CARGO_BIN_EXE_girpr` is only set for a target in the package that owns the binary |
| module path | `#[path = "artifact_bridge/server.rs"] mod server;` | required, not decorative: `mod server;` from a crate root at `tests/artifact_bridge.rs` resolves to `tests/server.rs` (`E0583`) |
| gate 1 | **every** bridge test is `#[ignore]`d, including the pure-logic ones | **the suite is expensive** — ~18 cases, ~2 min of real binary runs over ~20 MB of bundle bytes each. Kept out of every default run; started by hand only when a repair-spec change is suspected. It still compiles under `cargo test`, so a broken bridge fails to build |
| gate 2 | `GIRPR_ARTIFACT_BUNDLE` → `gircap list --root artifacts` → directory scan → **fail**, naming `cargo run -p gircap -- capture --out <root>` | Gate 1 is what makes this right: only a human starts this suite, so an absent bundle is missing input for a check someone asked for, never a habitual `cargo test -- --ignored`. "Checked nothing" must not render as green. *(Corrected — see §0.1 M6.)* |
| network | loopback only, four listeners, ephemeral ports | spec §4 |
| bundle writes | **none.** Rewrites are in-memory; `game/` is copied to a scratch dir | spec §4.1, §6.4 |
| one run per capture | kept even though the eight could share a process | a `download_bytes` total is only attributable to one capture (spec §5) |
| isolation | per capture: scratch `game/` copy, own listeners, own request log, own tree diff | spec §6.3; `#[test]`s run in parallel |
| served-manifest cache | process-global, keyed by `(bundle, tag, field)`, shared by every listener | spec §4.6 / §6.3 — bytes are a function of the bundle alone |
| request log | `Vec<(Kind, String)>` behind a `Mutex`; **no forbidden-set** | §4.7 leaves the choice open. The log already turns a reuse regression into a loud failure at assertion 3, with a message that names the id and the capture; a forbidden-set would answer with a 404 the shim manufactured and blur the two failure modes |
| not-found shapes | replayed from `api/error_shapes.json`; status + content-type are the contract, body is fidelity | spec §4.4 |
| commit discipline | **one commit per milestone. No milestone lands uncommitted.** | house rule (`docs/05` §0) |

### 0.1 Two places the spec does not agree with itself

Both are settled here against the source, and both are recorded because
`docs/06` is the document a later reader will check this implementation
against.

1. **"Four families, six listeners"** (§4). The route table lists four origins
   and the load-bearing requirement is only that *manifests and chunks are on
   different ports* (§4.1's `build_id` invariant). Two more listeners buy
   nothing that one port plus a `build_id` check inside it does not already
   buy. **Four listeners**: hyp, sophon `getBuild`, manifests, chunks.
2. **`--include` is "all but `removed`"** (§5's table), **but** §5.1's own
   sample line for `removed` is `… files=0 purge_files=1 purge_bytes=1731133
   scoped=1` — and `scoped=1` is unreachable without a scope flag. The source
   settles it: `src/repair/mod.rs` §4.2 says in as many words that
   `--purge-only --include <that path>` "is precisely the request that deletes
   it". **Every capture gets `--include`**, including `removed`; `removed` is
   the only one that gets `--purge-only` and the only one that does not get
   `--max-files`.

---

## 1. Milestones

Each is one commit. Each ends by handing back for review.

### M0 — this plan

Acceptance: `docs/07-…` committed; nothing else touched.

### M1 — the replay server

`tests/artifact_bridge/server.rs`, with `tests/artifact_bridge.rs` reduced to a
crate root plus `mod server;` so the module compiles.

Contents:

- `Bundle` — opened **read-only**: `MANIFEST.json` (`biz`, `versions`,
  `audio`), the four `api/*.json` bodies, the `patchmaps/*.json` set,
  `manifests/<tag>/<field>.manifest.zst`, `chunks/blobs/<id>.zst`,
  `expected/`, `game/`.
- `Bridge::start(bundle)` — binds the four listeners **first** (the ports are
  inputs to the rewrite, and the rewrite happens after the listeners exist),
  then builds the served state.
- **The rewrite**: parse `api/getBuild.<tag>.json`, replace only the *scheme
  and authority* of every `manifests[].chunk_download.url_prefix` and
  `manifests[].manifest_download.url_prefix` with the listener origin that
  serves that namespace, re-serialize. Everything else passes through
  untouched — in particular `manifest.checksum` (MD5 of the *decompressed*
  manifest, checked by `sophon::fetch_manifest`) and `manifest.id` (the URL
  path segment) must survive byte-for-byte, or the run dies at metadata with
  exit `2` for a reason that looks like a product bug.
- **The `id → (tag, field)` map** is built *inside that same loop* over
  `manifests[]`, because that loop is where every id in the bundle passes
  through. 404 anything absent from it (§4.2). Not a filesystem scan over
  `manifests/*/*.meta.json`: that would answer "is this archive in the bundle"
  instead of "did *this build* declare it".
- **Blob routing**: last two path segments, `([build_id], [id])`. Both blob
  listeners enforce the §4.1 invariant — `build_id` must equal the build whose
  `getBuild` declared that id, else the recorded `blob_wrong_build` 404.
- **`getBuild`**: tagless → `versions.new.tag`; `?tag=<t>` → `getBuild.<t>.json`
  if present, else the recorded `getbuild_unknown_tag` body. Any other tag
  than the bundle's two is 404-shaped on the JSON side by replaying `-202`,
  because that is what production does.
- **Request log**: `Arc<Mutex<Vec<(Kind, String)>>>` per bridge, appended
  before the response is written.
- **Served-manifest cache**: process-global `OnceLock<Mutex<HashMap<CacheKey,
  Arc<Vec<u8>>>>>`.

Acceptance:

- `cargo test -p girpr --test artifact_bridge` compiles and reports only
  ignored tests.
- `#[ignore]`d cases inside the bridge prove, against the real bundle: the
  rewrite changes only the two `url_prefix` fields (deep-compare the parsed
  body before/after); every rewritten prefix carries the right origin and the
  **path is unchanged**; the `id → (tag, field)` map resolves a real manifest
  id of both tags and 404s a synthetic one; a blob request under the wrong
  `build_id` is refused; `getBuild?tag=<unknown>` returns the recorded `-202`.
- **`gircap verify --artifact <dir>` still exits `0`** after the milestone —
  the bundle is byte-identical, which is the §3 precondition and the §6.4
  point, checked by the tool that owns that verdict rather than by this crate.

### M2 — the driver, one capture green

`tests/artifact_bridge.rs` grows the harness the rest of the milestones reuse.

- Gate 2 resolution + skip message; the `gircap verify` precondition (skipped,
  with its reason printed, when `gircap` is not built).
- Scratch copy of the whole fixture `game/` (all eight captures' paths, because
  "outside the scope is untouched" is measured against the other seven), plus
  a `BTreeMap<rel, (len, md5)>` before/after snapshot — **per run**, not per
  bundle (§6.3).
- Flag set exactly as §5's table, `--output json`, records read by
  `event` name (`report` / `plan` / `summary`).
- One run per capture. The `--print-plan` pre-pass first (§5.1), then the real
  run.

Acceptance: **`partial_same_offset` goes green end to end** — the capture whose
figure `docs/06` quotes (`274426`). Assertions 1–4 all bite, i.e.:

1. `SUMMARY download_bytes == patchmaps/partial_same_offset.json → totals.download_bytes`;
2. the patched file's MD5 equals `expectations.result_md5`;
3. no id in `expectations.must_not_fetch` appears in the request log;
4. exit `0`, `config_bumped=0`, and the before/after tree diff shows no change
   outside the selected path.

…plus the §4.3 degraded-but-green guards: `local_manifest=ok`, and
`est_download_bytes` from the pre-pass equal to the patchmap total.

### M3 — every capture, and the structural assertions

One `#[ignore]`d `#[tokio::test]` per `patchmaps/<id>.json`, each with its own
scratch dir / listeners / log / diff (§6.3). The per-class differences in §6.2
are pinned where they differ:

| class | what differs |
|---|---|
| `identical` | `skipped=1 repaired=0 download_bytes=0`, exit `0`; `must_not_fetch` still bites |
| `partial_*`, `zero_reuse_*` | assertions 1–3 all bite; these are the load-bearing cases |
| `added` | the path is absent from `game/`, planned and downloaded in full; `must_not_fetch` is empty |
| `removed` | `--purge-only`; the **purge counters** are the assertion (`files=0`, `purge_files=1`, `purge_bytes=1731133`), the file is gone afterwards, no byte total |

Plus the two structural ones:

- assertion 5 — the capture paths are pairwise distinct, because a repeat makes
  the second run silently skip and report `download_bytes=0` against a
  non-zero expectation;
- the bundle is still intact: `gircap verify` exits `0` after the whole
  milestone.

Acceptance: all eight green, no capture skipped for a reason other than
"the bundle does not carry it".

### M4 — not-found replay, and the docs

- §4.4's four measured shapes served from `api/error_shapes.json`: blob
  missing (manifest and chunk), `blob_wrong_build`, `getBuild?tag=<unknown>`.
  Status and content-type are the contract; the body is fidelity, with only the
  fields the document marks volatile synthesized (`RequestId`, `HostId`, `Key`,
  and the length that follows from `Key`).
- The **no-evidence** branch: a bundle with no `api/error_shapes.json` falls
  back to the measured `404` + `application/xml` shape and the bridge *logs
  that the not-found assertions were skipped for lack of evidence* rather than
  passing them silently.
- `--log-file` stays the documented last resort; the request log is the
  primary source (§4.7), and when the log is used as a cross-check it matches
  on the **download record**, never on the bare id.
- README: the `tests/artifact_bridge.rs` + `tests/artifact_bridge/server.rs`
  module-map rows, the "nothing yet proves a flag reaches the HTTP client"
  bullet replaced by what now does, and the `cargo test --test artifact_bridge
  -- --ignored --nocapture` invocation. `docs/06`'s status line, and the
  `tools/capture` README row that points at the consumer.

Acceptance: `cargo test -p girpr --test artifact_bridge -- --ignored
--nocapture` is green end to end; `cargo test` (no filter) is unchanged and
still green; `cargo clippy --all-targets` clean.

---

## 2. Module tree

```
tests/
  artifact_bridge.rs            the #[ignore]d driver + gate 2 + the eight cases
  artifact_bridge/
    server.rs                   Bridge, Bundle, routes, request log, error shapes
```

Both are **test-only**; nothing under `src/` changes. `src/sophon.rs` already
carries a note that the bridge parses its `chunk downloaded chunk=<id>` trace
line — that note stays accurate, and that line remains the documented last
resort rather than what the bridge actually reads.

---

## 3. Risks

| risk | what it would look like | answer |
|---|---|---|
| the rewrite breaks `manifest.checksum` | run dies at metadata, exit `2`, looking like a product bug | checksum is MD5 of the *decompressed* manifest and is **not** touched by an authority swap; M1 deep-compares the parsed body before/after and asserts the only two changed fields are the two `url_prefix`es |
| `--include` matches more than one path | `plan_files=2`, `est_download_bytes` twice the patchmap total | `--max-files 1` refuses with exit `5` before any download; the pre-pass asserts `files=1` first, which names the glob rather than the ceiling |
| `--audio` is omitted and the selection drifts | the run fetches a manifest the bundle does not carry, 404s | deliberate (§5). Keep the warning; §4.4's honest 404 is fatal-once, not silent |
| a fixture mutated in place | `gircap verify` fails afterwards and the bundle is no longer evidence | scratch copy per capture (§6.4); M3 re-verifies |
| parallel runs share a tree | intermittent failures that read as product bugs | per-capture scratch dir + own listeners + own log (§6.3) |
| the served-manifest cache holds capture-specific state | a run sees another capture's bytes | it holds `(bundle, tag, field) → bytes`, which is a function of the bundle alone (§4.6) |
| temp usage | 8 × ~32 MB of scratch `game/` copies | removed by each test's `Drop`; peak is bounded by the number of tests in flight |

---

## 0.1 Landed

Appended per milestone, newest last.

### M1 — the replay server

`tests/artifact_bridge/server.rs` (~900 lines) and a `tests/artifact_bridge.rs`
that is, for now, the crate root plus the server module. Six `#[ignore]`d cases,
all green against the reference bundle, and `gircap verify` still reports
`checks_total=90 checks_failed=0 exit_code=0` afterwards — the bundle was
opened read-only and never written.

What the six pin, in the order they matter:

- **`the_rewrite_changes_only_the_two_url_prefixes`** — for both tags, every
  field of every entry in `data.manifests[]` is compared against the recorded
  body, `manifest{}` included, with `manifest.checksum` and `manifest.id` then
  pinned by name and by reason.
- **`each_namespace_keeps_its_path_and_gets_its_own_origin`** — every rewritten
  prefix starts with its own listener's origin and the remainder equals the
  recorded prefix's path byte-for-byte.
- **`a_manifest_id_resolves_to_the_field_archive_of_its_own_build`** — each
  declared id resolves to `(tag, matching_field, declaring build_id)`, the
  `carried` flag agrees with the archive existing on disk, the carried ones are
  fetched over loopback and come back 200, and a synthetic id is a 404 with the
  measured `application/xml` content-type. It also asserts that **some**
  declared manifests are *not* carried (2 of 5 per tag here) — otherwise
  `docs/06` §4.5's `--audio` warning would have no teeth.
- **`a_blob_under_the_other_builds_prefix_is_refused`** — the new build's own
  manifest id is served under the new build and refused under the old one, and
  the replayed body names the build actually requested.
- **`an_unserved_getbuild_tag_replays_the_recorded_not_found`** — both served
  tags are real bodies, an unserved one is `200` + `retcode -202` with `data:
  null`, and the body is compared against the recorded shape verbatim (this one
  has no volatile fields, so it *can* be). The request log distinguishes
  "latest" from an explicit tag.
- **`the_capture_matrix_loads_with_distinct_paths`** — `docs/06` assertion 5,
  plus the per-capture ground-truth invariants: a capture with no byte total is
  `absent_after`, one with a zero byte total has a non-empty `must_not_fetch`,
  and every `result_path` exists.

Three things this milestone decided or discovered:

1. **The `build_id` invariant is enforced strictly for manifests and as
   "the latest build" for chunks.** `docs/06` §4.1 states one rule for both
   blob namespaces. For manifests the map built during the rewrite answers it
   exactly. For chunks it cannot be answered from the bundle:
   `chunks/index.json` records `referenced_by` — which *capture* referenced a
   blob, for materialization — not which build's manifest declares it. And it
   would be the wrong question anyway: every chunk download goes to the latest
   build's `chunk_download.url_prefix` (`repair::manifest::fetch_latest`),
   because the local build's manifests only feed the *reuse* map, which is a
   local copy. So the chunk listener serves an id only under the latest build's
   id, and refuses everything else as `blob_wrong_build`. That still catches
   the failure the two ports exist to catch — a shim with the blob prefixes
   crossed.
2. **`gircap capture` takes `--out`, not `--root`.** `gircap list` takes
   `--root`. The Gate 2 skip message names the exact command that would produce
   a bundle, so this is not a cosmetic detail: the first draft said
   `capture --root`, which would have shipped a skip message naming a flag that
   does not exist.
3. **`manifests/<tag>/<field>.manifest.meta.json` was checked and deliberately
   not wired in.** It carries `tag`, `build_id`, `manifest.id`,
   `manifest.checksum` and the *recorded* `declared.*.url_prefix`, so it could
   cross-check the rewrite against the bundle's own record of the live prefix.
   It was left out: `getBuild.<tag>.json` is the authoritative body the client
   actually parses, and adding a second source of truth for the same fact is
   the kind of thing that drifts.

Not yet done, per the milestone split: the `gircap verify` precondition, the
scratch `game/` copy, the tree diff, the `--print-plan` pre-pass and the real
`girpr` runs are all M2.

### M2 — the driver, one capture green

The harness the rest of the milestones reuse, plus the first real `girpr` runs.
Seven `#[ignore]`d cases green (M1's six plus the new one).

What landed:

- **`assert_bundle_verifies`** — the §3 precondition, `gircap verify --artifact
  <dir>` exit `0`, found next to `CARGO_BIN_EXE_girpr` (a workspace member
  lands in the same target directory, so there is no guessing and no
  `cargo build -p gircap` step to remember). Not re-implemented here: `gircap`
  owns that verdict. When `gircap` is not built the precondition is **skipped
  with its reason printed**, per §3.
- **`Scratch`** — a per-run copy of the **whole** `game/` fixture, removed on
  `Drop`. The whole fixture, not the one selected path, because a capture is
  one path *out of* the fixture and the other seven are what "outside the scope
  is untouched" is measured against (§6.4).
- **`snapshot` / `tree_delta` / `tree_changed`** — `rel -> (bytes, md5)` before
  and after. MD5 rather than length and mtime, because "untouched" has to mean
  *byte-identical*: a run that rewrote a file to the same length would otherwise
  pass.
- **`girpr(...)`** — spawns the real binary, parses `--output json` NDJSON into
  records indexed by `event` name. `--log-file` is pointed into the scratch dir
  so a run does not scatter `logs/` under the build directory.
- **`base_args` / `capture_args`** — §5's flag set, verbatim, with `--audio`
  omitted deliberately.
- **`drive_capture`** — pre-pass, real run, then §6's four assertions plus the
  §4.3 degraded-but-green guards.

Measured, on `partial_same_offset`:

```
bridge partial_same_offset  plan files=1 est=274426
  | run total=1 skipped=0 repaired=1 failed=0 dl=274426 deleted=0
  | chunks requested=2 | tree +0 -0 ~1
```

That is the figure `docs/06` §5.1 quotes, and the reuse map is provably what
produced it: only 2 chunks were requested (the patchmap's `fetch=2`), the one id
in `must_not_fetch` was never asked for, and `local_manifest=ok`. §4.3's
degraded-but-green alternative for this same run is `913040` with
`local_manifest=unavailable` — a factor of 3.3 that would still have exited `0`.

Two decisions this milestone made:

1. **A bundle that fails `gircap verify` is a hard failure, not a skip.** Gate 2
   is about an *absent* input turning people off `--ignored` forever; a corrupt
   bundle is a real problem and must not hide behind a green run. §3 wants the
   verdict consumed, and consuming it means saying so when it is bad.
2. **`Scratch` copies the whole fixture**, not just the capture's path — as
   §6.3 requires, and ~32 MB per run.

Scope note, so the next reader is not misled: `drive_capture` has two branches
(the byte total for a repair capture, the purge counters for `removed`) and only
the repair branch has been exercised. `removed` is M3's.

### M3 — every capture, each in its own world

One `#[ignore]`d test per `patchmaps/<id>.json`, each with its own `Bridge`, its
own `Scratch`, its own request log and its own before/after diff — which is what
§6.3 requires and also what makes one test per capture *possible*: Rust runs
tests in parallel threads, so a shared tree and a shared log would mean two runs
fighting over one path and one run's `must_not_fetch` seeing the other's
requests.

Fifteen cases green in 132 s wall-clock (the eight captures run concurrently;
each is a pre-pass plus a real run, ~20 MB of manifests each).

Measured, every capture, against its patchmap:

| capture | plan `files` | plan `est` | run `dl` | chunks asked | tree |
|---|---|---|---|---|---|
| `identical` | 1 | 0 | 0 | **0** | untouched |
| `zero_reuse_single` | 1 | 21 | 21 | 1 | 1 changed |
| `zero_reuse_multi` | 1 | 336852 | 336852 | 2 | 1 changed |
| `partial_same_offset` | 1 | 274426 | 274426 | 2 | 1 changed |
| `partial_moved_offset` | 1 | 1564894 | 1564894 | 2 | 1 changed |
| `partial_size_changed` | 1 | 197192 | 197192 | 1 | 1 changed |
| `added` | 1 | 114357 | 114357 | 1 | **1 added** |
| `removed` | **0** | 0 | 0 | 0 | **1 removed**, `deleted_extra_bytes=1731133` |

Two of these are worth reading twice:

- **`removed`** prints `files=0 purge_files=1 purge_bytes=1731133 scoped=1` —
  byte-for-byte the line `docs/06` §5.1 predicts, including `scoped=1`. That
  settles §0.1's second disagreement **by measurement rather than by reading the
  source**: `--include` is given for `removed` too. The pre-pass now asserts
  `scoped=1` uniformly for every capture, which is the shape `scoped=1` can only
  have with a scope flag passed.
- **`identical`** downloads nothing and touches nothing, so
  `download_bytes=0` — which is also what a *broken* run reports. The assertion
  that carries it is the request log being empty, and the tree diff showing no
  change at all.

Per-class expectations (§6.2) are now in `drive_capture` rather than in the
tests, so a new capture id needs no new assertion code: `identical` is planned
then skipped and untouched; `added` shows an *addition* and has an empty
`must_not_fetch`; `removed` has `total=0` and prices the deletion into
`deleted_extra_bytes`; the five patchable/zero-reuse classes must be *repaired*
(never skipped) and must show their bytes moving on disk.

Two structural additions:

- **The `build_id` invariant, measured through a real run.** Every chunk
  request's `<build_id>` must equal the new build's. That is §4.1's invariant as
  a *measured* fact rather than a probed one, and it is the only route by which
  a shim with the two blob prefixes crossed could pass.
- **`the_matrix_and_the_patchmaps_agree_in_both_directions`.** `MANIFEST.json`'s
  matrix and `patchmaps/*.json` must agree in both directions: a `captured` entry
  has a patchmap and a non-`captured` entry has a reason and no patchmap, and no
  patchmap exists without a matrix entry. Here that is 9 entries, 8 captured,
  8 on disk — the ninth, `full_reuse_reordered`, is recorded **unavailable** with
  its reason, and is skipped rather than invented.

One thing the first draft got wrong: the matrix's `capture_id` is present only
on `captured` entries — an entry that never materialised has no capture id, so
the key to match on is `class`. `class == capture_id` for every captured entry
(they are the patchmap stem), and that equality is now asserted.

### M4 — not-found is replayed, not invented; and the docs

`docs/06` §4.4, in full, and the documentation the bridge needs to be findable.
**Seventeen cases green**; the plan's M0–M4 are all landed.

**Not-found replay.** `not_found_is_replayed_from_the_recorded_shapes` drives
all four shapes that have a route here and compares each against the bundle's
own `api/error_shapes.json` record:

- status and content-type must equal the recorded ones (they are the contract —
  `girpr` calls `.error_for_status()` and never reads the body);
- the body must be equal to the recorded one **after blanking `Key` and
  `RequestId`**, which are the only fields `docs/06` §4.4.1 marks volatile;
- the body must name the key that was actually requested;
- **the body length must equal `recorded − len(recorded Key) + len(requested
  Key)`** — the length follows from the substituted `Key` alone;
- the recorded `RequestId` must not survive.

The document's other three shapes (`getbuild_bad_package`,
`getgameconfigs_bad_game_id`, `getgamebranches_bad_game_id`) have **no route**,
because the bridge never issues such a request — a bad `package_id` or a bad
`game_ids[]` is something only `gircap capture` sends. They are checked for
presence, status, content-type and retcode so the bundle's record is read rather
than assumed, and the test says so out loud.

**The no-evidence branch.** `a_bundle_without_recorded_shapes_says_so_instead_of_inventing`
builds a shadow bundle from the real one's *small* files — `MANIFEST.json`,
`patchmaps/`, the four `api/` bodies, ~120 KB — deliberately **without**
`api/error_shapes.json`, and with `manifests/` and `chunks/` absent (nothing
here asks for a real id, and placeholder archives would be exactly the "serves
something the bundle does not vouch for" §7 forbids). It then asserts
`shapes_synthesized()`, that `recorded_shape("chunk_missing")` is `None`, that
the 404s still come back `404` + `application/xml` with a `NoSuchKey` body
naming the requested key — and prints that the replayed-shape assertions were
skipped for lack of evidence, which is the contract §4.4.2 asks for.

**The `--log-file` last resort, proven.** `downloaded_chunks` parses the trace
log matching **only** the two download records — `chunk downloaded chunk=<id>`
(`sophon::try_fetch_chunk`) and `no local reuse candidate -> download
chunk=<id>` (`repair::file::try_reuse_slice`) — and `drive_capture` asserts its
result equals the server's request log, for every capture. That is what makes
§4.7's rule worth writing down: a reused chunk is logged too (`reuse slice md5
ok -> copy chunk=<id>`), so a bare-id search false-positives. The pre-pass gets
its **own** log file, because `logging::open_log_file` opens with `append` and
one shared file could not say which run downloaded what.

One fix this milestone forced: the synthetic `RequestId` is truncated to 24
characters, like the recorded ones. At 32 it made the replayed body 8 bytes
longer for a second, undocumented reason — and body length is one of the two
things the not-found assertions compare.

**Docs.** `README.md`: module-map rows for both new files, the `--ignored`
invocation, and the "nothing yet proves a flag reaches the HTTP client" bullet
replaced by what now does and what it asserts. `tools/capture/README.md`: the
end-to-end check is no longer "run by hand … does not exist yet", plus the
one-directional-independence note and the consumer's invocation.
`docs/06-artifact-bridge-spec.md`: status line flipped from "spec. Nothing here
is implemented" to implemented, pointing at this file for what changed and
leaving the contract itself as written.

### Done

`cargo test -p girpr --test artifact_bridge -- --ignored --nocapture` — 17 cases,
~140 s with the eight captures concurrent. `cargo test` — 82 + 6 + 53, with the
17 bridge cases reported ignored. `cargo clippy --all-targets` and
`cargo fmt --check` clean. `gircap verify` 90/90 exit `0` throughout: the bundle
was never written.

### M5 — closing the audit

A pass back through `docs/06` against what actually shipped turned up one gap
and one thing that was already covered elsewhere.

**The gap: Gate 2's discovery primitive.** §0 of this file says the resolution
order is `GIRPR_ARTIFACT_BUNDLE` → `gircap list --root artifacts` → print the
reason. The implementation did the first and the third and quietly replaced the
second with a directory scan — which works, but is not what the plan claims and
not what §3 names ("the primitive for this already exists"). It is now
`gircap_list` first, with the scan as the fallback when `gircap` is not built,
and `discovery_agrees_whether_it_asks_gircap_or_scans` asserts the two name the
same bundles in the same order. Without that assertion a bundle could be found
by one mechanism and missed by the other, and which one ran would depend on
whether a sibling workspace member happened to be compiled.

**Not a gap: §4.4.3's "a 404 is asked once, with no backoff."** That is already
pinned, and pinned better than the bridge could:
`tests/repair_offline.rs::a_404_is_asked_once_where_a_500_is_asked_five_times`
counts the requests from the origin's side of the wire — 1 for a 404 against
`ATTEMPTS` (5) for a 500, with a wall-clock ceiling on each. Duplicating it here
would have been a second, weaker copy. What the bridge contributes is the
*honest* 404 that makes the cheap branch reachable at all.

**The file was also reordered.** The M1 cases had drifted to the bottom of the
file, under M4's banner, because the driver section was written into the M1
banner's place. The sections now read M1 → M2 → M3 → M4, which is the order the
milestones landed in.

Eighteen cases, and the same four numbers as M4: `cargo test` 82 + 6 + 53 with
the 18 bridge cases ignored; `gircap verify` 90/90 exit `0`; clippy and fmt
clean; no scratch directories left behind.

### M6 — the gates, reframed: cost, and errors rather than skips

**M0–M5 got Gate 1's reason wrong, and Gate 2 exactly backwards.** M0 recorded
the `#[ignore]` as a tidiness property — "never runs under `cargo test`" — and
M2 built Gate 2 out of the spec's literal words: skip, never fail, print the
reason, return success. Both are now corrected, and the correction is recorded
here because `docs/06` §3 is the document a later reader will check.

**What the `#[ignore]` is actually for: cost.** One capture is a `--print-plan`
pre-pass plus a real run, and each run fetches four manifests — ~20 MB of real
bundle bytes — before it can plan a single file. Eighteen cases, ~2 minutes.
That is worth paying when something has gone wrong and not worth paying on
every `cargo test`. So the suite runs **by hand, deliberately, only when the
repair spec is suspected to have regressed**, and the `#[ignore]` reason on
every case now says exactly that rather than naming a missing bundle.

**Why that makes Gate 2 an error, not a skip.** The skip rule was reasoned as
"people type `cargo test -- --ignored` by habit and a red run there would put
them off". That reasoning belongs to a suite wired into a default run. This one
is priced so that only a human ever starts it — which *reverses* the argument.
Nobody reaches this suite by accident, so the habit does not exist; while the
opposite failure does: someone asks for the check, the bundle was never
captured, and a skip renders "I checked and it is fine" for a check that never
ran. An absent bundle — or a missing capture — now **fails**, with the exact
command that produces one.

The one distinction kept: a class the bundle's own matrix records as *not
captured, with a reason* is a **fact**, not a missing prerequisite.
`full_reuse_reordered` has no path for 7.0.0 -> 7.1.0 and `MANIFEST.json` says
so, so a case for it stands down out loud. A capture that is missing with no
such record fails. That is `Bundle::matrix` — read once at `Bundle::open`
rather than re-parsed by the driver — and `MatrixClass::absent_by_record` is
where the distinction lives.

Also: `the_matrix_and_the_patchmaps_agree_in_both_directions` now reads the
parsed matrix instead of re-reading `MANIFEST.json` with a second JSON reader in
the test file, and asserts the captured count equals the patchmap count — the
same fact stated twice, so the two can be compared.

Measured both ways: with the bundle **18 passed, 0 failed** in ~134 s; with an
empty `GIRPR_ARTIFACT_ROOT` **18 failed, 0 passed**, every one naming
`cargo run -p gircap -- capture --out <root>`.

Docs corrected to match: `docs/06` §3 (both gates, with the superseded rule
kept as a visible correction rather than deleted), `docs/06` §7's
"a bridge that fails when the bundle is absent" bullet, which named the opposite
of the current behaviour, and the `README.md` / `tools/capture/README.md` rows.