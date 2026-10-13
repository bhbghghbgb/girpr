# Implementation plan — `gircap`

Build order, module tree, test strategy, and the risks worth knowing before
milestone 0 starts. Companions: [`00-spec.md`](00-spec.md) (what),
[`01-artifact-schema.md`](01-artifact-schema.md) (the bytes).

---

## 0. Shape of the crate

```
tools/capture/
├── Cargo.toml            # package gircap; [workspace]-detached OR workspace member (see M0)
├── README.md             # orientation only — no spec duplication
├── docs/                 # 00-spec, 01-artifact-schema, this file
├── src/
│   ├── main.rs           # shim: parse -> tracing -> run -> SUMMARY -> exit
│   ├── lib.rs            # module map + re-exports
│   ├── cli.rs            # clap surface
│   ├── exit.rs           # ExitCode enum + the exit-code table
│   ├── config.rs         # Resolved plan: tags, classes, budgets, paths
│   ├── biz.rs            # launcher/game ids, hyp+sophon bases, channel tuple
│   ├── report.rs         # stdout data plane (single owner of println!)
│   ├── logging.rs        # tracing init + --log-file (JSON lines)
│   ├── http/
│   │   ├── mod.rs        # Fetcher: retries, timeout, user-agent, reqlog sink
│   │   └── probe.rs      # tag_probe.json
│   ├── hyp/
│   │   ├── mod.rs        # the 4 HoYoPlay endpoints + getBuild
│   │   └── types.rs      # DTOs, incl. the fields most clients drop
│   ├── manifest/
│   │   ├── mod.rs        # fetch -> zstd -> verify -> decode -> store
│   │   ├── proto.rs      # prost structs + opaque field 6
│   │   ├── dump.rs       # full JSON/gz dump + paths.tsv sidecar
│   │   └── diff.rs       # path diff, reuse pool, stats reconciliation
│   ├── select/
│   │   ├── mod.rs        # scoring + admission
│   │   ├── classes.rs    # the class predicates
│   │   └── budget.rs     # smallest-first under --max-download-bytes
│   ├── blobs/
│   │   ├── mod.rs        # content-addressed store + props + selfcheck rows
│   │   ├── props.rs      # container sniff, md5/sha256, header extraction
│   │   └── chunks.rs     # the three sets: materialize_old / _new / patch_fetch
│   ├── assemble/
│   │   ├── mod.rs        # build a file from chunks; old, new, and patch flow
│   │   └── ranges.rs     # range algebra + gap/overlap checks
│   ├── emit/
│   │   ├── mod.rs        # artifact writer; tmp/ -> move-into-place
│   │   ├── gamedir.rs    # config.ini + audio scan file + variant files
│   │   ├── index.rs      # MANIFEST.json, candidates.json, chunks/index.json
│   │   └── selfcheck.rs  # the A1–A17 assertion log
│   └── verify/
│       ├── mod.rs        # `verify` + `list` subcommands, offline
│       └── materialize.rs# `materialize`, rebuild from chunks/ with no network
└── tests/
    ├── common/mod.rs     # local mock server (own code; borrows nothing from girpr)
    ├── capture_layout.rs # end-to-end artifact shape + selfcheck green
    ├── matrix.rs         # class predicates + greedy admission + budget
    └── schema.rs         # JSON schema stability, gzip rule, checksums coverage
```

### Dependencies

`clap`, `anyhow`, `tokio`, `reqwest`, `serde`, `serde_json`, `prost`, `zstd`,
`md5`, `sha2`, `flate2` (gzip), `tracing`, `tracing-subscriber`, `chrono`.

Deliberately **not** `twox-hash`: the manifest-id prefix turned out not to be a
hash of anything observable (spec §3.7), so an xxh64 implementation would be
unused code asserting something false.

### Style constraints (inherited from this repo)

- One writer per channel: `report.rs` owns stdout, `logging.rs` owns stderr. The
  only `println!` is the summary emitter.
- Structured events only — a value belongs in a field, not in prose.
- `mod.rs` is a facade re-exporting its children; leaf files each own one concern.
- Inline `#[cfg(test)]` tests travel with the code they cover; `tests/` drives
  the public API.
- Comments explain *why*, and cite the reference project or the measurement that
  justifies the decision — never restate the code.

---

## 1. Milestones

Each ends with something runnable and a stated acceptance check. Nothing starts
before the previous milestone's check passes.

### M0 — skeleton and independence
Add the workspace entry (`Cargo.toml` `members`), the module tree with empty
modules, `cli.rs`, `exit.rs`, `report.rs`, `logging.rs`.
**Accept:** `cargo build -p gircap` and `cargo test -p gircap` pass;
`gircap --help` exits 0; a grep for `girpr` in `src/` and `Cargo.toml` returns
only doc comments; `cargo clippy -p gircap --all-targets` is clean.

### M1 — transport and provenance
`biz.rs`, `http/mod.rs` (retries, timeout, UA, `attempts` count), `http/probe.rs`.
**Accept:** against the live API, `api/requests.ndjson` is written with correct
status/elapsed/headers for a `getGameConfigs` and a `getBuild`; a deliberately
bad tag exits `2`. Unit tests for URL building, retry/backoff counting, and the
`Range` header path run offline.

### M2 — metadata endpoints
`hyp/mod.rs`, `hyp/types.rs`. Capture all four HoYoPlay bodies byte-exact plus
both `getBuild` bodies; resolve the version pair and validate the download mode.
**Accept:** live run writes the seven `api/*.json` files plus `tag_probe.json`;
every captured body re-parses; `retcode != 0` anywhere exits `2` with the URL
named. `types.rs` models `stats`, `compression`, `encryption`, `password`,
`categories[]` — the fields a minimal client discards.

### M3 — manifests
`manifest/{mod,proto,dump}.rs`. zstd-decode, verify decompressed MD5 against
`checksum` and against the id suffix, label the prefix `opaque`, dump full JSON
(gzipped past the threshold) and the `paths.tsv` sidecar.
**Accept:** live run stores both `game` manifests; A3/A4/A5 green; A6 of
`diff` shape present in `meta.json → decoded.structure`; gzip rule observed and
recorded (`encoding` in `MANIFEST.json`). Unit tests cover the protobuf decoder
including an empty file entry and an `is_folder` entry.

### M4 — diff and the reuse pool
`manifest/diff.rs`.
**Accept:** live run reproduces the measured numbers — 2 662 common, 1 592
identical, 82 added, 11 removed, 98 910 same-manifest md5 hits, 2 cross-file-only.
These are asserted in a test against a frozen fixture, not just eyeballed.

### M5 — scoring, classes, budget
`select/{mod,classes,budget}.rs`.
**Accept:** with `--files /dev/null` (score only, fetch nothing) the tool
reproduces the measured candidate table for all nine classes; greedy admission
produces the expected winner per class; `--per-class 2` admits two and tags the
second `over_budget` under a tight ceiling. Pure unit tests, no network.

### M6 — the chunk store
`blobs/{mod,props,chunks}.rs`. Content-addressed by chunk id; one fetch per id;
`materialize_old` / `materialize_new` / `patch_fetch` set construction; A6 rows.
**Accept:** live run on the `partial_same_offset` candidate fetches 5 distinct
chunk ids (3 old + 2 new), stores 5 blobs, and produces the
`chunks/index.json` shape in the schema doc byte-for-byte in field names.

### M7 — assembly
`assemble/{mod,ranges}.rs`. Build old from `materialize_old`, new from
`materialize_new`, and the patch flow (REUSE slice verified by MD5, else FETCH)
then assert the patch result byte-equals the direct new assembly.
**Accept:** A7/A8/A9/A10/A11/A12 all green live. Ranges tests cover gap,
overlap, out-of-order, and moved-offset cases.

### M8 — the artifact
`emit/{mod,gamedir,index,selfcheck}.rs`. Full tree, `game/`, `expected/`,
`config.ini`, audio scan file, `MANIFEST.json`, `checksums.json`, `README.md`.
**Accept:** a live capture produces a tree that passes `emit`'s own completeness
check; `game/` contains nothing but `config.ini`, the audio scan file, and the
variant files; A13–A17 green.

### M9 — offline re-verification
`verify/{mod,materialize}.rs`.
**Accept:** `gircap verify` on a good artifact exits 0; after corrupting one byte
of one chunk blob it exits `3` naming the blob and the assertion. `gircap
materialize` rebuilds both files from `chunks/` with the network unplugged and
matches `game/` and `expected/`. **This milestone is what makes the artifact
trustworthy**, so it is not optional.

### M10 — tests
`tests/*` against a local mock server.
**Accept:** `cargo test -p gircap` is green and **touches no network** — the mock
serves synthetic manifests built by the test, exactly as `girpr`'s own suite does
against its own mock. A test asserting a captured artifact has zero failed
selfcheck rows. A test asserting the gzip rule fires on a >threshold body and not
below.

### M11 — first real capture
Run against `hk4e_global` 7.0.0 → 7.1.0, `game` field, all nine classes.
**Accept:** exit 0; `selfcheck.json` `result: "pass"`; artifact under
`artifacts/hk4e_global/7.0.0__7.1.0/<id>/`; then:
1. `gircap verify` on it, offline, exit 0;
2. hand `game/` to `girpr` and confirm it reaches `7.1.0` and that
   `SUMMARY download_bytes` equals `patch_fetch` totals — **this is the
   acceptance test for the whole tool, and it is run by hand once, not
   automated, because `gircap` must not depend on `girpr`.**
3. `diff -r game/ expected/` lists exactly the non-`identical` variant files.

---

## 2. Test strategy

| layer | what it pins | network |
|---|---|---|
| unit, inline | URL building, protobuf decode, range algebra, class predicates, greedy admission, budget trim, gzip rule, hash helpers, `config.ini` + audio-scan writers | never |
| `tests/common` | a local HTTP server serving synthetic `game` manifests/chunks for `hk4e_global`-shaped responses | never |
| `tests/capture_layout` | the emitted tree matches the schema doc's paths and key names; `MANIFEST.json` is complete | never |
| `tests/matrix` | each class admits the expected path; `covered_by` / `over_budget` / `unavailable` are all reachable | never |
| `tests/schema` | `schema_version` present everywhere, gzip decision recorded, `checksums.json` covers the tree | never |
| live (manual) | M11 | yes, deliberately |

Frozen fixture: the measured 7.0.0 → 7.1.0 numbers (counts, class populations,
cheapest candidate per class) are written into a test as literal expectations over
a *stored* decoded manifest fixture. They will drift when HoYo ships a new
version — that is the point. The test names them `measured_2026_10_14` so the
drift is legible rather than mysterious.

---

## 3. Risks

| risk | impact | handling |
|---|---|---|
| HoYo retires a tag mid-run | manifest 404s partway | exit `2`, `MANIFEST.json` `complete:false`, selfcheck names the missing tag; partial artifacts are never presented as good |
| A future version violates the structural invariants of §3.6 (non-gapless chunks, unsorted offsets, folders) | assembly or diff logic silently wrong | they are **assertions** (A11) and recorded per manifest in `decoded.structure`; a violation exits `3`, it does not pass quietly |
| A matrix class has no candidate | incomplete test matrix | `status:"unavailable"` + reason, run still exits `0`; `--require-class` turns it into exit `4` |
| `partial_moved_offset` and `partial_size_changed` are ~22 MB each | a default capture is ~96 MB | `--max-download-bytes`, smallest-first admission, and the exact cost is printed *before* fetching so a run is never a surprise |
| gzip hurts agent readability | an agent tries to `cat` a `.gz` | the threshold is a flag, every gzipped doc ships a plain `.tsv` sidecar, and `MANIFEST.json → files[].encoding` is authoritative |
| `flate2` output is not byte-stable across zlib versions | re-running produces a different `manifest.json.gz` | `checksums.json` covers the *decompressed* digest for gzipped files (`container:"gzip"`, `md5` of the plain body) so artifact identity does not depend on the compressor |
| The tool drifts toward "also do repair" | loses its purpose | M7/M9 assemble purely to *prove* the artifact, never to ship a patched game; there is no output path outside `game/` and `expected/` |

---

## 4. Explicitly deferred

- `hk4e_cn` / `hk4e_bilibili` captures (code paths exist from M1; only `hk4e_global`
  is exercised live in M11)
- audio / non-`game` manifests
- capture diffing or deduplicating across two capture runs
- any variant that needs a *third* version as input

## 5. Open questions carried into M0

1. **Workspace member vs detached.** Default plan: workspace member
   (`members = [".", "tools/sync", "tools/capture"]`), matching `girsync`'s
   precedent. Detaching instead costs one empty `[workspace]` table.
2. **`--audio` default.** Spec says `en-us` (mirrors the main tool's fallback).
   An artifact whose `game/` contains `en-us` will make a repair run pull the
   `en-us` manifest too. Confirm `en-us` is the right default for a fixture.
3. **Artifact root location.** `artifacts/` in the repo (gitignored) vs outside
   it. Default: `artifacts/`, added to `.gitignore`, since bundles are large and
   regenerable.