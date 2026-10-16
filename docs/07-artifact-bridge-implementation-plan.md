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
| gate 1 | **every** bridge test is `#[ignore]`d, including the pure-logic ones | spec §3 gate 1; "still compiles" is the anti-rot property |
| gate 2 | `GIRPR_ARTIFACT_BUNDLE` → `gircap list --root artifacts` → filesystem scan → print reason + `gircap capture` command, exit success | spec §3 |
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