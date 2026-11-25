# Implementation — `gircap`

The shape of the crate: its module tree, its dependencies, its style
constraints, how its tests are layered, and the risks worth knowing.
Companions: [`00-spec.md`](00-spec.md) (what),
[`01-artifact-schema.md`](01-artifact-schema.md) (the bytes),
[`03-design-notes.md`](03-design-notes.md) (why the awkward parts are the way
they are).

The build order this crate was written in is in `git log`, not here. The commit
series that built it reads as its own record, and a document that repeats it
goes stale the moment a module is added.

---

## 0. Decisions

| decision | value |
|---|---|
| workspace membership | **workspace member** — the root `Cargo.toml` lists the product, the shared `crates/*` leaves, the dev tools and gircap's own mechanics/test-support crates; `default-members` keeps a plain root build to the product |
| artifact root | **`artifacts/` in the repo, gitignored** — bundles are large and fully regenerable |
| `--audio` default | **`en-us`** — a launchable-install default, and it widens the selection pool (§0.1) |
| per-class variants | **no individual flags**; `--per-class N` only, default 1 |
| commit discipline | **one commit per change, each with its own gate result in the message** |

### 0.1 `--audio` and the selection pool (why the default matters)

`--audio` sets the scan file written into `game/` **and** the set of
`matching_field` manifests captured and scored. *(measured)* each audio manifest
carries further file entries at a modest compressed cost. Widening the audio
selection therefore widens the candidate corpus for the matrix — the backstop
when the `game` pool has no candidate for a required class in a given version
pair. Cost ranking keeps this safe: audio files are comparatively large, so
smallest-first naturally avoids them unless a class has no cheaper candidate.

---

## 1. The `girpr` side: path scope

`girpr` ships `--include` / `--exclude`, plus `--max-files`,
`--max-download-bytes`, `--print-plan` and `--purge-only`. A path outside the
scope is treated as nonexistent — invisible to the repair *and* to the
files-cleanup — and a scoped run does not bump `config.ini`. Without that, a run
against `game/` would build its plan from the whole live manifest and fetch the
entire game instead of the one captured file.

The end-to-end check lives on `girpr`'s side, as
`girpr`'s `tests/artifact_repair_bridge.rs`. It is `#[ignore]`d because it needs a
bundle and is expensive, and it is run by hand. That is
also the only arrangement that keeps §2 of the spec honest: `gircap` does not
invoke `girpr`, read its flags or share its exit codes, so the grep over
`tools/capture/src/` returns doc comments only.

The procedure and the figures it must produce are in
[`00-spec.md` §8.1.1](00-spec.md#811-the-procedure-and-the-numbers-it-must-produce).

---

## 2. Shape of the crate

```
tools/capture/
├── Cargo.toml            # package gircap
├── README.md             # orientation only — no spec duplication
├── docs/                 # 00-spec, 01-artifact-schema, this file, 03-design-notes, 04-bundle-differ
├── test-support/         # gircap-only dev library: the offline mock and fixture builders, reached through the thin tests/common/mod.rs shim so they compile once; borrows nothing from the repair binary
├── src/
│   ├── main.rs           # shim: parse -> tracing -> run -> records -> exit
│   ├── lib.rs            # module map + re-exports + VERSION + SCHEMA_VERSION
│   ├── encoding.rs       # shared percent-encoding (text + URL)
│   ├── cli/mod.rs        # clap surface
│   ├── exit/mod.rs       # ExitCode enum + the exit-code table
│   ├── failure/mod.rs    # Failure, its four constructors, the failure summary
│   ├── config/
│   │   ├── mod.rs        # the resolved run plan: tags, classes, budgets, paths
│   │   └── tests.rs      # plan-resolution and validation tests
│   ├── biz/mod.rs        # launcher/game ids, hyp+sophon bases, channel tuple
│   ├── (no hash/ dir)    # md5/sha256, hex, gzip rule and zstd helpers moved to the
│   │                     # logic-free tools/capture/hash/ crate; `pub use gircap_hash
│   │                     # as hash` keeps every crate::hash::* path unchanged
│   ├── run/mod.rs        # command dispatch: the four entry points
│   ├── report/
│   │   ├── mod.rs        # stdout data plane (single owner of println!)
│   │   └── tests.rs      # record ordering, escaping and event-name tests
│   ├── logging/mod.rs    # tracing init + the log file (JSON lines), over the shared log policy
│   ├── http/
│   │   ├── mod.rs        # thin re-export facade over gircap-http + the two policy modules
│   │   ├── probe.rs      # tag_probe.json + error_shapes.json (gircap policy: stays local)
│   │   ├── negative.rs   # choosing the inputs for the negative probes (stays local)
│   │   └── tests.rs      # mechanics tests over the public seams (stays local)
│   │
│   │   The mechanics moved to the logic-free `tools/capture/http/` crate:
│   │   error.rs (ApiError), transport.rs (RetryPolicy, SleepFuture, Sleep,
│   │   RealSleep, RecordingSleep, DEFAULT_MAX_RESPONSE_BYTES, Transport),
│   │   record.rs (Kind, RequestRecord, ACCEPT, ACCEPT_ENCODING, header_map),
│   │   log.rs (RequestLog, now_rfc3339), error_bodies.rs, tasks.rs and
│   │   fetcher.rs (Fetcher, Fetched, Streamed, Destination). gircap consumes it
│   │   directly so an emit/blobs/manifest edit leaves it cached.
│   ├── hyp/
│   │   ├── mod.rs        # the HYP endpoints + getBuild + the captured bodies
│   │   └── types.rs      # DTOs, incl. the fields most clients drop
│   ├── pathmap.rs        # logical→on-disk escapes (device names) and the run-wide store
│   ├── paths.rs          # the shape rules for values that name something in a bundle
│   ├── rejections.rs     # refused manifest paths/chunk ids, kept as evidence
│   ├── tasks.rs          # the ordered-join helper for concurrent fetches
│   ├── manifest/
│   │   ├── mod.rs        # zstd -> verify -> decode; the decoded-graph arithmetic
│   │   ├── fetch.rs      # one manifest per (field, build), and filing each where it belongs
│   │   ├── proto.rs      # prost structs + opaque field 6
│   │   ├── dump.rs       # full JSON/gz dump + paths.tsv sidecar
│   │   ├── diff.rs       # path diff, reuse pool, scoring, stats reconciliation
│   │   └── tests.rs      # decode, verification and diff tests
│   ├── select/
│   │   ├── mod.rs        # scoring + admission
│   │   ├── classes.rs    # the patch-shape predicates
│   │   ├── budget.rs     # smallest-first under --max-download-bytes
│   │   ├── decide.rs     # the selection decision, and the narration that reports it
│   │   ├── require.rs    # --require-class, and the one pass that builds its detail
│   │   └── tests.rs      # scoring and admission tests
│   ├── blobs/
│   │   ├── mod.rs        # content-addressed store, streaming fetch, A6 rows
│   │   ├── plan.rs       # the retention plan, A12 and the byte ceiling, before any fetch
│   │   ├── fill.rs       # execute the plan, or record that there is nothing to fill
│   │   ├── props.rs      # container sniff, md5/sha256, streaming measurement
│   │   ├── chunks.rs     # the three sets: materialize_old / _new / patch_fetch
│   │   └── tests.rs      # store and measurement tests
│   ├── assemble/
│   │   ├── mod.rs        # the assembly facade and re-exports
│   │   ├── assembly.rs   # build a side from chunks, and the REUSE/FETCH patch flow
│   │   ├── source.rs     # where each reused chunk's bytes come from
│   │   ├── types.rs      # plan entries, patchmaps, totals, expectations, outcomes
│   │   └── ranges.rs     # range algebra + gap/overlap checks
│   ├── emit/
│   │   ├── mod.rs        # artifact writer; document/tree helpers, api/ docs, diff/candidates + matrix
│   │   ├── tree.rs       # the part of the tree that exists before any chunk arrives
│   │   ├── fixture.rs    # game/, expected/, scan/ from the stored chunks
│   │   ├── gamedir.rs    # config.ini + audio scan file + variant files
│   │   ├── index.rs      # MANIFEST.json, checksums.json, the file entry, README rendering
│   │   ├── selfcheck.rs  # the A1–A17, A19 and A21–A23 assertion log (A18/A20 are verify-only)
│   │   └── closeout.rs   # the close-out documents with an ordering constraint, and A16's two passes
│   └── verify/
│       ├── mod.rs        # `verify` / `materialize` / `list`, offline
│       ├── bundle.rs     # opening a bundle and its recorded manifest
│       ├── checks.rs     # the re-derived A6/A7/A8/A11/A18/A20 checks
│       ├── list.rs       # walking a root and inventorying bundles
│       ├── report.rs     # the verify report and re-derived-id sets
│       ├── materialize.rs# `materialize`, rebuild from chunks/ with no network
│       └── tests.rs      # verify, list and materialize tests
└── tests/
    ├── common/mod.rs     # thin shim over the gircap-only test-support library (mock, world builders, shared-bundle publisher); the mock borrows nothing from girpr
    ├── bundle_consistency.rs # the matrix, patchmaps and bundle discovery must describe each other
    ├── bundle_shapes.rs  # expensive whole-capture shapes (ignored; gated behind `manual-tests`)
    ├── capture_layout.rs # end-to-end artifact shape + selfcheck green + document pins
    ├── case_coverage.rs  # gates tools/case_coverage.py against the bundle it measures
    ├── cli_contract.rs   # argv -> exit code, the process-level contract
    ├── matrix.rs         # class predicates + greedy admission + budget
    ├── measured_fixture.rs # the synthetic corpus the costs are measured against
    ├── offline_bundle.rs # build a bundle with the crate's own writers, read it back
    ├── schema.rs         # JSON schema stability, gzip rule, checksums coverage
    └── bundle_diff.rs    # two captures of one fixture must agree
```

The largest modules are `src/report/mod.rs`
and `src/select/mod.rs`. `src/http/mod.rs` is only a thin re-export facade over
the logic-free `gircap-http` crate (the transport, retrying fetcher, body
ceiling and request log) plus the two modules that carry gircap's own policy,
`probe` and `negative`; the mechanics live in `tools/capture/http/`. The
orchestrator (`src/run/mod.rs`) is one of the smaller modules, because each
phase of a capture lives in the module that owns the work it does.

### Dependencies

The direct dependency set is the CLI/runtime/report stack (`clap`, `tokio`,
`serde`, `serde_json`, `tracing`, `tracing-subscriber`, `tracing-appender`,
`chrono`), the file/format helpers (`url`, `prost`, `flate2`, `walkdir`,
`ini_core`, `percent-encoding`), typed errors (`thiserror`), the gircap-owned
`gircap-http` / `gircap-hash`, and the shared logic-free leaves. `reqwest` is no
longer a direct dependency: the transport is owned by the logic-free
`gircap-http` crate, which gircap depends on directly. MD5/SHA-256 are spelled
by `girpr-digest` and the digests and zstd decoding live in `gircap-hash`; zstd
is a dev-dependency here only for tests that build frames.

`tokio` is requested with only `rt-multi-thread`, `macros` and `time`, which
also brings `rt`. `reqwest` (via `gircap-http`) resolves tokio with `net`,
`io-util` and `sync` whatever this crate asks for, so those and their support features (`bytes`,
`libc`, `mio`, `socket2`, `windows-sys`) arrive transitively. The direct
request therefore omits `fs`, `io-std`, `process`, `signal` and
`parking_lot`, which nothing in this crate uses: those are the features the
shorter list actually saves.
`spawn`, `spawn_blocking` and `JoinSet` come with `rt`.

Deliberately **not** `twox-hash`: the manifest-id prefix is not a hash of
anything observable (spec §3.7), so an xxh64 implementation would be unused code
asserting something false.

Deliberately **not** `anyhow`, and not reqwest's `json` or `stream` features:
failures are typed (`Failure`, `UsageError`, `ApiError`, derived with
`thiserror`) and re-wrapped through `Failure`, all decoding is
`serde_json::from_slice`, and nothing reads a streaming body. A dependency that
is declared and never used is a dependency a reader has to check the crate for.

`tracing-appender` is here only for its non-blocking writer guard, which is what
flushes the tail of a log-file run. No rolling: a capture writes one log, it
is not a daemon.

The log *policy* — where the file goes, which level each sink gets, which env
var answers for which flag — is the workspace's and lives in `crates/log/`
(`girpr-log`), shared with the repair binary and `girsync`. It is a leaf crate:
no logic of any program, no dependency between programs (docs/00-spec.md §2).

### Style constraints (inherited from this repo)

- One writer per channel: `report.rs` owns stdout, `logging.rs` owns stderr. The
  only `println!` is the record emitter.
- Structured events only — a value belongs in a field, not in prose.
- `mod.rs` is a facade re-exporting its children; leaf files each own one concern.
- Inline `#[cfg(test)]` tests travel with the code they cover; `tests/` drives
  the public API.
- Comments explain *why*, and cite the reference project or the measurement that
  justifies the decision — never restate the code, and never narrate a change
  that is no longer in the tree.

### Two channels

**stdout** is the answer: a stream of `report::Record`s, one per line, the
terminal one last, or the same records as NDJSON objects under `--output json`.
**stderr** is the narration, via `tracing`. Neither writes to the other's
channel, which is what lets `--output json` hand a caller a clean stream with no
log chatter mixed in.

---

## 3. Test strategy

| layer | what it pins | network |
|---|---|---|
| unit, inline | URL building, protobuf decode, range algebra, class predicates, greedy admission, budget trim, the gzip rule, hash helpers, `config.ini` + audio-scan writers, one-pass digests vs whole-buffer digests | never |
| `tests/common` | a local HTTP/1.1 server on `127.0.0.1:0` serving synthetic `hk4e_global`-shaped manifests and chunk blobs | never |
| `tests/capture_layout` | the emitted tree matches the schema doc's paths and key names; `MANIFEST.json` is complete; per-document field pins for the volatile documents | never |
| `tests/cli_contract` | argv → exit code, at the process level, including `--help` and a missing subcommand | never |
| `tests/matrix` | each class admits the expected path; `covered_by` / `over_budget` / `unavailable` are all reachable | never |
| `tests/measured_fixture` | the measured costs and cheapest-candidate-per-class, against a synthetic corpus sized to reproduce them | never |
| `tests/offline_bundle` | a bundle built by the crate's own writers, read back through its own readers | never |
| `tests/schema` | `schema_version` present everywhere, the gzip decision recorded, `checksums.json` covers the tree, the patchmap arithmetic | never |
| `tests/bundle_consistency` | the matrix, patchmaps and bundle discovery describe the same bundle | never |
| `tests/bundle_shapes` | expensive whole-capture shapes (metadata-only, lying manifest, mis-served chunk); ignored and gated behind the empty `manual-tests` feature, run with `--features manual-tests --test bundle_shapes --run-ignored only` | never |
| `tests/case_coverage` | gates `tools/case_coverage.py` against the bundle it measures | never |
| `tests/bundle_diff` | two captures of one fixture agree, once the never-deterministic fields are normalised | never |
| live (manual) | a capture against the real API, and `girpr`'s `artifact_repair_bridge` against the result | yes, deliberately |

**There is no stored decoded-manifest fixture, and that is a decision, not an
oversight.** A checked-in fixture can only ever test what it happened to contain,
and it silently becomes wrong the moment HoYo ships a new version — with the
failure looking like a code bug. The measured numbers are still asserted, as
literals, against a corpus constructed to reproduce them, and the corpus is
regenerated by the test every run.

Further properties the suite holds deliberately, worth preserving through
any change to it:

- **no network** — every capture points at `127.0.0.1:0` via
  `--hyp-base`/`--sophon-base`, and two tests read `api/requests.ndjson` back out
  of a captured bundle to check that *every* recorded URL, manifest and chunk CDN
  included, is a loopback address the mock owns. "Touches no network" is proven,
  not asserted.
- **no silently skipped checks** — the expensive whole-capture cases in
  `tests/bundle_shapes.rs` are `#[ignore]`d and run by hand with
  `cargo nextest run -p gircap --features manual-tests --test bundle_shapes
  --run-ignored only`; an ignored case is named, not hidden in the default run.
  The empty `manual-tests` feature gates the test target so a normal build or
  test run does not compile or link that binary.
- **no fixture files under `tests/`** — the directory holds only Rust sources,
  and every fixture is generated at test time.

**Cheap enough to gate every commit.** The default run has no sleeps and no
retry loops — which is the property that makes the suite usable as a gate rather
than something run occasionally. The runner is `cargo nextest run`, and it runs
**each test in its own process**: the isolation is the point, so the mock's
world is built once per *test* rather than once per binary, and nothing one
test does — a global, a temp directory, an environment variable — can reach the
next one.

### The gate that compares two captures, and what it cannot see

`tests/bundle_diff.rs` captures the same fixture twice and compares the two
trees byte-for-byte after normalising the fields that were never deterministic
(timestamps, `elapsed_ms`, the capture id, the `--out` root). It is the gate for
every change that must not move a bundle byte.

It compares **two captures of one binary**, so it is blind by construction to a
change applied **uniformly to every capture** — a field that starts being written
where it was null, or a wording narrowed in every row. Closing that needs
a *written expectation*, not a comparison, and that is what the per-document pins
in `tests/capture_layout.rs` are: `api/requests.ndjson`'s rows by kind, and
`MANIFEST.json` / `verify/checksums.json` / `README.md` / `verify/selfcheck.json`
by document inventory, scalar headers and assertion wording. Digests inside those
documents are deliberately unpinned — A16 owns them, and they move for
`requests.ndjson` and its cascade rows by design.

`[`docs/04-bundle-differ.md`](04-bundle-differ.md) is the instrument for the
cross-binary version of the same question, and
[`docs/03-design-notes.md`](03-design-notes.md#3-the-gates-and-what-each-one-cannot-see)
says which is which.

---

## 4. Risks

| risk | impact | handling |
|---|---|---|
| HoYo retires a tag mid-run | manifest 404s partway | exit `2`, `MANIFEST.json` `complete:false`, selfcheck names the missing tag; partial artifacts are never presented as good |
| A future version violates the structural invariants of spec §3.6 (non-gapless chunks, unsorted offsets, folders) | assembly or diff logic silently wrong | they are **assertions** (A11) and recorded per manifest in `decoded.structure`; a violation exits `3`, it does not pass quietly |
| A matrix class has no candidate | incomplete test matrix | `status:"unavailable"` + reason, run still exits `0`; `--require-class` turns it into exit `4` |
| a two-sided reuse class can be the most expensive | a default capture is large | `--max-download-bytes`, smallest-first admission, and the exact cost is printed *before* fetching so a run is never a surprise |
| gzip hurts agent readability | an agent tries to `cat` a `.gz` | the threshold is a flag, every gzipped doc ships a plain `.tsv` sidecar, and `MANIFEST.json → files[].encoding` is authoritative |
| `flate2` output is not byte-stable across zlib versions | re-running produces a different `manifest.json.gz` | `checksums.json` records the *decompressed* digest for gzipped files, so artifact identity does not depend on the compressor |
| The tool drifts toward "also do repair" | loses its purpose | assembly exists purely to *prove* the artifact, never to ship a patched game; there is no output path outside `game/` and `expected/` |
| An audio manifest supplies a capture | `game/` then holds a VO asset the fixture was not designed around | captures record `matching_field`; `game` wins every cost tie, so audio is only reached when the `game` pool has no candidate |
| A document or field is added and an existing bundle stops verifying | a bundle you already paid for becomes unreadable | a new *field* is additive within a version — a reader must tolerate an unknown field and reject only an unknown `schema_version`. Version 3 deliberately made the evidence fields and documents **required**, because a bundle that omitted them could not distinguish "nothing was there" from "nobody looked": the cost is one re-capture, and it is paid rather than teaching every reader two shapes. What is never acceptable is a **required field that defaults**, because the default is indistinguishable from the real value. |

---

## 5. Explicitly deferred

- `hk4e_cn` / `hk4e_bilibili` captures — the code paths exist, but only
  `hk4e_global` is exercised against the live API.
- audio **bulk** — audio manifests are captured for the selection pool and a
  selected audio file is materialised like any other, but no audio data is ever
  fetched speculatively.
- capture diffing or deduplicating across two capture runs
- any variant that needs a *third* version as input

## 6. What is not on the table

These are the changes that look like cleanup and are not. They are listed so
nobody mistakes them for tidying.

| | why |
|---|---|
| Renaming, renumbering or removing **A1–A23** | spec'd at [`00-spec.md` §6](00-spec.md). The ids are a published contract: `girpr`'s `artifact_repair_bridge` and any recorded bundle refer to them. |
| Changing `SCHEMA_VERSION` or any document's required fields | [`01-artifact-schema.md`](01-artifact-schema.md); a reader must reject an unknown value rather than guess. |
| Any dependency on `girpr`, or a `Command` against its binary | the independence invariant, spec §2. A grep over `tools/capture/src/` must keep returning doc comments only. |
| Turning on `clippy::pedantic` crate-wide | clippy is clean today on the default lints, which is worth preserving. Enabling pedantic in one commit rewrites the whole crate. If wanted, adopt lint groups one at a time. |
| Release-profile tuning (LTO, `codegen-units`, `panic=abort`) | untested for a tool whose output *is* a fixture, and the perf headroom is in the algorithms, not in codegen. |
| `rustfmt` / `clippy --fix` sweeps over untouched files | a reformat makes every other change unreviewable. |
