# Implementation plan — `gircap`

Build order, module tree, test strategy, and the risks worth knowing before
milestone 0 starts. Companions: [`00-spec.md`](00-spec.md) (what),
[`01-artifact-schema.md`](01-artifact-schema.md) (the bytes).

---

## 0. Decisions

All four questions from scoping are settled; nothing below re-opens them.

| decision | value |
|---|---|
| workspace membership | **workspace member** — `members = [".", "tools/sync", "tools/capture"]`, same as `girsync` |
| artifact root | **`artifacts/` in the repo, gitignored** — bundles are large and fully regenerable |
| `--audio` default | **`en-us`** — a launchable-install default, and it widens the selection pool (§0.1) |
| per-class variants | **no individual flags**; `--per-class N` only, default 1 |
| commit discipline | **one commit per milestone. No milestone lands uncommitted.** |

### 0.1 `--audio` and the selection pool (why the default matters)

`--audio` sets the scan file written into `game/` **and** the set of
`matching_field` manifests captured and scored. *(measured)* each audio manifest
is ~1 MB compressed and carries 177 further file entries. Widening the audio
selection therefore widens the candidate corpus for the matrix — the backstop
when the `game` pool has no candidate for a required class in a given version
pair. Cost ranking keeps this safe: audio files average ~80 MB each, so
smallest-first naturally avoids them unless a class has no cheaper candidate.

---

## 1. External blocker (does not gate the build)

`girpr` has **no path include/exclude**. Run against `game/`, it builds the plan
from the whole live manifest and would fetch the entire game instead of the one
captured file. The fix is a `girpr` feature; `gircap` neither depends on it nor
works around it.

Consequence for this plan: **M11 step 2 is blocked and is recorded as blocked**,
not skipped. Everything up to and including M10 is unblocked, and `gircap verify`
/ `gircap materialize` give the offline half of the acceptance check regardless.
Full detail and the intended invocation: [`00-spec.md` §8](00-spec.md#8-consuming-the-bundle--and-one-external-blocker).

---

## 2. Shape of the crate

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

## 3. Milestones

Each ends with something runnable, a stated acceptance check, and **its own
commit**. Nothing starts before the previous milestone's check passes, and no
milestone is left uncommitted — if a milestone is abandoned, the work is
reverted or the reason is in the commit message.

Commit messages use `gircap: M<N> — <summary>` as the subject, so
`git log --oneline` reads as the build order.

### M0 — skeleton and independence
Add the workspace entry (`Cargo.toml` `members`), the module tree with empty
modules, `cli.rs`, `exit.rs`, `report.rs`, `logging.rs`. Add `artifacts/` to
`.gitignore`.
**Accept:** `cargo build -p gircap` and `cargo test -p gircap` pass;
`gircap --help` exits 0; a grep for `girpr` in `src/` and `Cargo.toml` returns
only doc comments; `cargo clippy -p gircap --all-targets` is clean.
**Commit:** `gircap: M0 — crate skeleton, CLI surface, workspace member`

### M1 — transport and provenance
`biz.rs`, `http/mod.rs` (retries, timeout, UA, `attempts` count), `http/probe.rs`.
**Accept:** against the live API, `api/requests.ndjson` is written with correct
status/elapsed/headers for a `getGameConfigs` and a `getBuild`; a deliberately
bad tag exits `2`. Unit tests for URL building, retry/backoff counting, and the
`Range` header path run offline.
**Commit:** `gircap: M1 — HTTP fetcher with request provenance log and tag probe`

### M2 — metadata endpoints
`hyp/mod.rs`, `hyp/types.rs`. Capture all four HoYoPlay bodies byte-exact plus
both `getBuild` bodies; resolve the version pair and validate the download mode.
**Accept:** live run writes the seven `api/*.json` files plus `tag_probe.json`;
every captured body re-parses; `retcode != 0` anywhere exits `2` with the URL
named. `types.rs` models `stats`, `compression`, `encryption`, `password`,
`categories[]` — the fields a minimal client discards.
**Commit:** `gircap: M2 — HoYoPlay + Sophon getBuild capture and version-pair resolution`

### M3 — manifests
`manifest/{mod,proto,dump}.rs`. zstd-decode, verify decompressed MD5 against
`checksum` and against the id suffix, label the prefix `opaque`, dump full JSON
(gzipped past the threshold) and the `paths.tsv` sidecar.
**Accept:** live run stores both `game` manifests; A3/A4/A5 green; A6 of
`diff` shape present in `meta.json → decoded.structure`; gzip rule observed and
recorded (`encoding` in `MANIFEST.json`). Unit tests cover the protobuf decoder
including an empty file entry and an `is_folder` entry.
**Commit:** `gircap: M3 — manifest fetch, verify, decode and dump`

### M4 — diff and the reuse pool
`manifest/diff.rs`.
**Accept:** live run reproduces the measured numbers — 2 662 common, 1 592
identical, 82 added, 11 removed, 98 910 same-manifest md5 hits, 2 cross-file-only.
These are asserted in a test against a frozen fixture, not just eyeballed.
**Commit:** `gircap: M4 — path diff and cross-file reuse-pool measurement`

### M5 — scoring, classes, budget
`select/{mod,classes,budget}.rs`.
**Accept:** with `--files /dev/null` (score only, fetch nothing) the tool
reproduces the measured candidate table for all nine classes; greedy admission
produces the expected winner per class; `--per-class 2` admits two and tags the
second `over_budget` under a tight ceiling. Pure unit tests, no network.
**Commit:** `gircap: M5 — scoring, variant classes and smallest-first budget admission`

**Landed.** Three places the implementation departed from the text above, all
recorded here because each one is a decision a later milestone inherits.

1. **"score only, fetch nothing" is the whole test suite, not a flag.** There is
   no `/dev/null` and no need for one: `tests/measured_fixture.rs` builds the
   two decoded manifests in memory and `select` is pure arithmetic over them, so
   the acceptance is an ordinary offline test. `capture` has no dry-run mode.
2. **`--require-class` requires nothing by default.** §Risk already said it
   should ("a class has no candidate → `unavailable` + reason, run still exits
   `0`; `--require-class` turns it into exit `4`"), and the live pair proves
   why: 7.0.0 → 7.1.0 has no `full_reuse_reordered` file, so requiring all nine
   by default would make *every* run of it exit `4`. `unavailable` is a fact
   about the manifests; `over_budget` is a fact about this run's flags; only a
   class the caller named is fatal.
3. **The frozen M4 fixture was corrected.** It could not have satisfied this
   acceptance: `data_revision` reused its own chunk (making it
   `full_reuse_reordered`, not `zero_reuse_single`), `09801749.blk` reused all
   three (same class again), `zero_reuse_multi` had no candidate at all, and
   `zero_reuse_single` was therefore never exercised. Re-modelled against the
   §4.1 table, which also fixed its `pair()` helper returning the same diff
   twice.

The selection pool is `game` ∪ `--audio`'s codes, so a `capture` now fetches
and diffs the audio manifests too — about 1.2 MB compressed per tag per code.

### M6 — the chunk store
`blobs/{mod,props,chunks}.rs`. Content-addressed by chunk id; one fetch per id;
`materialize_old` / `materialize_new` / `patch_fetch` set construction; A6 rows.
**Accept:** live run on the `partial_same_offset` candidate fetches 5 distinct
chunk ids (3 old + 2 new), stores 5 blobs, and produces the
`chunks/index.json` shape in the schema doc byte-for-byte in field names.
**Commit:** `gircap: M6 — content-addressed chunk store with the three retention sets`

**Landed.** Live, on the `partial_same_offset` candidate: 5 distinct chunk ids,
5 blobs, 0 A6 failures. Across all eight admitted captures: 38 blobs, 27 612 723
B, no `tmp/` leftovers. Two design decisions the docs did not settle, both of
which the measured pair forced:

1. **Reuse identity is `(uncompressed_md5, uncompressed_size)`, not
   `chunk.id`.** The id is `<16 opaque hex>_<uncompressed_md5>` and the prefix
   is not a function of anything observable. The measured `HoYoChannel.dll`
   proves the point: offset 0 is `bbce8a31…` in 7.0.0 and `de1cfe69…` in 7.1.0 —
   same offset, same length, different bytes — while offset 690719 has the *same*
   id in both because its content really is the same. An id-keyed test would be
   right only by accident.
2. **The store is keyed by `chunk_id` alone, with a source list per blob.** The
   alternative — `(tag, chunk_id)` — produces *six* wants for the acceptance's
   five, because the shared chunk would be fetched once per build. A `Want` now
   carries every build that declares the id, ordered newest-first, so the old
   CDN is only asked if the new one has gone. Two builds declaring one id with
   *different* content is recorded as a conflict rather than resolved, since
   resolving it silently would hide the contradiction from the selfcheck.

The store never fails a run on a bad blob: a truncated or mismatched chunk is
measured, recorded with `verified: false` and a named `decode_error`, and left in
the index. Only a blob with no zstd frame at all is an error, because the build
declared `compression: 1` and there is then nothing to measure.

### M7 — assembly
`assemble/{mod,ranges}.rs`. Build old from `materialize_old`, new from
`materialize_new`, and the patch flow (REUSE slice verified by MD5, else FETCH)
then assert the patch result byte-equals the direct new assembly.
**Accept:** A7/A8/A9/A10/A11/A12 all green live. Ranges tests cover gap,
overlap, out-of-order, and moved-offset cases.
**Commit:** `gircap: M7 — assemble old, new and the patch flow from retained chunks`

### M8 — the artifact
`emit/{mod,gamedir,index,selfcheck}.rs`. Full tree, `game/`, `expected/`,
`config.ini`, audio scan file, `MANIFEST.json`, `checksums.json`, `README.md`.
**Accept:** a live capture produces a tree that passes `emit`'s own completeness
check; `game/` contains nothing but `config.ini`, the audio scan file, and the
variant files; A13–A17 green.
**Commit:** `gircap: M8 — artifact writer: game fixture, expected ground truth, index and selfcheck`

### M9 — offline re-verification
`verify/{mod,materialize}.rs`.
**Accept:** `gircap verify` on a good artifact exits 0; after corrupting one byte
of one chunk blob it exits `3` naming the blob and the assertion. `gircap
materialize` rebuilds both files from `chunks/` with the network unplugged and
matches `game/` and `expected/`. **This milestone is what makes the artifact
trustworthy**, so it is not optional.
**Commit:** `gircap: M9 — offline verify and materialize subcommands`

### M10 — tests
`tests/*` against a local mock server.
**Accept:** `cargo test -p gircap` is green and **touches no network** — the mock
serves synthetic manifests built by the test, exactly as `girpr`'s own suite does
against its own mock. A test asserting a captured artifact has zero failed
selfcheck rows. A test asserting the gzip rule fires on a >threshold body and not
below.
**Commit:** `gircap: M10 — offline integration suite against a local mock`

### M11 — first real capture
Run against `hk4e_global` 7.0.0 → 7.1.0, selection pool `game` + `en-us`, all
nine classes.
**Accept:** exit 0; `selfcheck.json` `result: "pass"`; artifact under
`artifacts/hk4e_global/7.0.0__7.1.0/<id>/`; then:
1. `gircap verify` on it, offline, exit 0;
2. `gircap materialize` on it, offline, and the rebuilt files match `game/` and
   `expected/` byte-for-byte;
3. `diff -r game/ expected/` lists exactly the non-`identical` variant files;
4. **BLOCKED** — hand `game/` to `girpr` and confirm it reaches `7.1.0` with
   `SUMMARY download_bytes` equal to each capture's
   `patchmaps/<id>.json → totals.download_bytes`. This cannot run until `girpr`
   has path include/exclude (see §1); without it the run would fetch the whole
   game. It is **not** skipped — it is left open, and M11 is not "done" until it
   runs. Run by hand once when unblocked, never automated, because `gircap` must
   not depend on `girpr`.
**Commit:** `gircap: M11 — first live capture (end-to-end check blocked on girpr path filters)`

---

## 4. Test strategy

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

## 5. Risks

| risk | impact | handling |
|---|---|---|
| HoYo retires a tag mid-run | manifest 404s partway | exit `2`, `MANIFEST.json` `complete:false`, selfcheck names the missing tag; partial artifacts are never presented as good |
| A future version violates the structural invariants of spec §3.6 (non-gapless chunks, unsorted offsets, folders) | assembly or diff logic silently wrong | they are **assertions** (A11) and recorded per manifest in `decoded.structure`; a violation exits `3`, it does not pass quietly |
| A matrix class has no candidate | incomplete test matrix | `status:"unavailable"` + reason, run still exits `0`; `--require-class` turns it into exit `4` |
| `partial_moved_offset` and `partial_size_changed` are ~22 MB each | a default capture is ~96 MB | `--max-download-bytes`, smallest-first admission, and the exact cost is printed *before* fetching so a run is never a surprise |
| gzip hurts agent readability | an agent tries to `cat` a `.gz` | the threshold is a flag, every gzipped doc ships a plain `.tsv` sidecar, and `MANIFEST.json → files[].encoding` is authoritative |
| `flate2` output is not byte-stable across zlib versions | re-running produces a different `manifest.json.gz` | `checksums.json` covers the *decompressed* digest for gzipped files (`container:"gzip"`, `md5` of the plain body) so artifact identity does not depend on the compressor |
| The tool drifts toward "also do repair" | loses its purpose | M7/M9 assemble purely to *prove* the artifact, never to ship a patched game; there is no output path outside `game/` and `expected/` |
| **The end-to-end check cannot run** because `girpr` has no path include/exclude | the artifact is never validated against the real tool | recorded as an external blocker (plan §1, spec §8). Mitigated by `verify`/`materialize` giving the offline half now; the online half is left visibly open, not dropped |
| An audio manifest supplies a capture | `game/` then holds a VO asset the fixture was not designed around | captures record `matching_field`; `game` wins every cost tie, so audio is only reached when the `game` pool has no candidate |

---

## 6. Explicitly deferred

- `hk4e_cn` / `hk4e_bilibili` captures (code paths exist from M1; only `hk4e_global`
  is exercised live in M11)
- audio **bulk** — audio manifests are captured for the selection pool and a
  selected audio file is materialised like any other, but no audio data is ever
  fetched speculatively
- capture diffing or deduplicating across two capture runs
- any variant that needs a *third* version as input

## 7. Resolved

All scoping questions are closed and recorded in §0. Nothing is carried into M0.