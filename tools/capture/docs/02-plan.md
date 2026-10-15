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

## 1. External blocker — RESOLVED (was: `girpr` had no path include/exclude)

`girpr` has **no path include/exclude**. Run against `game/`, it builds the plan
from the whole live manifest and would fetch the entire game instead of the one
captured file. The fix is a `girpr` feature; `gircap` neither depends on it nor
works around it.

> **Resolved, and the check has been run.** `girpr` now ships `--include` /
> `--exclude`, plus `--max-files`, `--max-download-bytes`, `--print-plan` and
> `--purge-only`. A path outside the scope is treated as nonexistent — invisible
> to the repair *and* to the files-cleanup — and a scoped run does not bump
> `config.ini`.
>
> Consequence for this plan: **M11 step 4 is done.** All eight captures were
> repaired against a scratch copy of `game/`, every `download_bytes` matched its
> patchmap to the byte, and the 19 reused chunks that four captures declare were
> reused rather than downloaded. See
> [`00-spec.md` §8](00-spec.md#8-consuming-the-bundle-with-girpr) and the step-4
> addendum under M11.

Independence is unaffected and load-bearing: `gircap` still does not invoke
`girpr`, read its flags or share its exit codes, and the grep over
`tools/capture/src/` still returns doc comments only.
Full detail and the intended invocation: [`00-spec.md` §8](00-spec.md#8-consuming-the-bundle-with-girpr).

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

**Landed.** Live, across all eight admitted captures: **44 selfcheck rows, 0
failures**. Every file hashes to its declared MD5 and every patch flow byte-equals
the direct assembly. The `partial_same_offset` patchmap matches the schema doc's
worked example field for field — 3 chunks, 1 REUSE, 2 FETCH, 0 moved,
`download_bytes` 274 426, `must_not_fetch` the one `f5dc3ca1…` chunk.

Three decisions:

1. **A11 is checked before any byte is placed**, and it stops the assembly. A
   manifest whose chunks do not tile the file could otherwise produce bytes that
   happen to hash correctly, which is the one failure mode an MD5 check cannot
   see.
2. **A failed REUSE becomes a FETCH, and the downgrade is recorded.** The spec
   says "REUSE slice verified by MD5, else FETCH", so the decision is made
   *after* verification — but a reader who sees FETCH has no way to know a REUSE
   was attempted, so the plan entry carries `note` and the outcome carries
   `rewrites`. A silent downgrade is indistinguishable from one that was never a
   candidate, and the two demand opposite debugging.
3. **`Layout::of` returns violations rather than an error.** Same rule as M6's
   `measure`: the finding is the deliverable. `assemble_side` then refuses to
   build from a violating layout, so the record and the behaviour agree — the
   layout is described in full and *then* not used.

### M8 — the artifact
`emit/{mod,gamedir,index,selfcheck}.rs`. Full tree, `game/`, `expected/`,
`config.ini`, audio scan file, `MANIFEST.json`, `checksums.json`, `README.md`.
**Accept:** a live capture produces a tree that passes `emit`'s own completeness
check; `game/` contains nothing but `config.ini`, the audio scan file, and the
variant files; A13–A17 green.
**Commit:** `gircap: M8 — artifact writer: game fixture, expected ground truth, index and selfcheck`

**Landed.** Live, pool `game` + `en-us`: **114 selfcheck rows, 0 failures**, 95
files checksummed at 135 095 941 B, A16 green on the second pass.
`game/` holds exactly `config.ini`, the scan file at the launcher's own
`audio_pkg_scan_dir`, and 7 variant files; `expected/` holds the 7 new-side
files. `diff -r game/ expected/` shows the removed file and the added file plus
6 that need patching.

Row counts, which is what makes "one row per assertion *instance*" checkable:
A1 6 (six bodies), A2 2, A3 4 and A4 8 (four manifests, two checks each plus
the opaque prefix), A5 8, **A6 38 (one per stored blob)**, A7 7 and A8 7 (no old
side for `added`, no new side for `removed`), A9 6, A10 4, A11 14 (one per
assembled file side), A12 6, A13–A17 one each.

Three decisions:

1. **`game/` and `expected/` are written one capture at a time**, as each
   assembles, rather than after the loop. The fixture is ~31 MB and keeping
   every assembly until the end would hold all of it plus the 28 MB of chunks at
   once; writing as we go keeps peak residency at one file.
2. **A16 needs two passes.** `checksums.json` must cover `selfcheck.json`, and
   `selfcheck.json` must carry A16 — so the row is checked after the first write
   and both documents are rewritten. Two passes converge exactly: the second
   `checksums.json` is scanned after the final `selfcheck.json`. A third would
   find nothing new, which is the proof.
3. **`MANIFEST.json → files[]` is not the complete list; `checksums.json` is.**
   `MANIFEST.json` cannot list itself and cannot know the digest of a
   `README.md` that has not been written, so `files[]` covers everything before
   it and `checksums.json` covers everything except itself. Both documents say
   so. A document that hashes itself has no fixed point.

### M9 — offline re-verification
`verify/{mod,materialize}.rs`.
**Accept:** `gircap verify` on a good artifact exits 0; after corrupting one byte
of one chunk blob it exits `3` naming the blob and the assertion. `gircap
materialize` rebuilds both files from `chunks/` with the network unplugged and
matches `game/` and `expected/`. **This milestone is what makes the artifact
trustworthy**, so it is not optional.
**Commit:** `gircap: M9 — offline verify and materialize subcommands`

**Landed.** Verified live against `20261014T040448Z`: `verify` exits 0 with 83
re-derived rows and 62 216 590 B rebuilt from `chunks/` in 1.7 s; after flipping
one byte inside one blob's zstd frame it exits `3` with the chunk id and the
decode failure as the headline; `materialize` reproduces all 14 manifest-derived
files byte-for-byte (SHA-256), leaving the bundle itself untouched. Five
departures from the text above, each of which a later milestone inherits:

1. **`verify` re-derives seven of the seventeen, not all of them, and says so.**
   A1-A5, A9, A10, A12, A14 and A15 are statements about the API bodies, the
   manifests and the diff. Re-running them inside the bundle would be re-running
   the capture's own algorithm against the capture's own output — a check that
   can only ever agree. `verify` therefore reports `checks_derived=` and
   `checks_covered_by_recording=` on stdout, and `REDERIVED` is the `&'static`
   list of the overlap. "Verified" is a claim about how much was checked, and a
   reader who cannot see the split will read seventeen when seven were looked at.
   *(M12: the count is now nine — A18 was re-derived all along and simply never
   named. `REDERIVED` and `DERIVED_UNRECORDED` are separate lists; see M12.)*
2. **There is an eighteenth id, `A18`, for the closure check.** None of the
   seventeen asks whether `game/` is what the retained blobs say it is, which is
   the bundle's central claim and the reason `materialize` exists. It gets its own
   id so it cannot be mistaken for one of the seventeen, and so a reader diffing
   two `selfcheck.json` files sees a row only a verifier could have written. It is
   deliberately absent from `REDERIVED`: the capture cannot record it, because it
   is a statement about the bundle rather than about the run.
3. **A6 is re-derived against `chunks/index.json`, not against the manifests.**
   The four manifest-declared checks are already recorded in the index; re-deriving
   them needs the manifest, and a verifier that reaches for the manifest to check
   the manifest's own blob is checking the wrong thing. What `verify` re-derives is
   the part the index is responsible for — that the file at `stored_path` is the
   bytes the index describes. A blob the index lists with no path is **red, not
   skipped**: that is exactly the `--no-chunk-blobs` bundle, and reporting it as a
   failure is what tells an operator their bundle cannot be materialized.
4. **`materialize` writes *beside* the bundle, never inside it.** The config doc
   said "beside" and the first implementation wrote `<artifact>/materialized`,
   which adds a file `checksums.json` does not list — so the next `verify` fails
   A16 on the bundle this command had just "repaired". The default is now
   `<bundle-dir-name>-materialized` as a sibling, and an explicit `--out` inside
   the artifact warns rather than refuses.
5. **Every bundle document now derives `Deserialize` as well as `Serialize`.**
   `verify` has to read back what `capture` wrote, and mirroring the structs as
   separate read types would let the two drift silently — which is the one class
   of bug a round-trip test cannot see. Three fields could not carry a borrowed
   lifetime out of a document and became owned: `BlobProps.container`,
   `Assertion.id` and `Assertion.assertion`. **No wire format changed** — the JSON
   is byte-identical, and the full suite was green before the sweep.

Two smaller things the milestone forced. `Checksums::a16` reports the *first*
mismatch, which is right for a capture that stops at the first and wrong for a
verifier over 95 files, so it now has `Checksums::findings` returning all of them
and `a16` is defined in terms of it. And `verify` checks the recomputation
against the recorded selfcheck **last**, after the rebuild, because a
disagreement is a finding about the *recording* and a reader who met it before
A18 would go looking for the wrong thing.

The one bug this milestone found in itself, worth keeping: the A6 row first
compared a blob against the index *and* against itself, which always agreed.
Both sides now render the same five fields in the same order, and the test that
caught it (`a_blob_whose_bytes_were_altered_turns_its_row_red`) is the one that
matters.

### M10 — tests
`tests/*` against a local mock server.
**Accept:** `cargo test -p gircap` is green and **touches no network** — the mock
serves synthetic manifests built by the test, exactly as `girpr`'s own suite does
against its own mock. A test asserting a captured artifact has zero failed
selfcheck rows. A test asserting the gzip rule fires on a >threshold body and not
below.
**Commit:** `gircap: M10 — offline integration suite against a local mock`

**Landed.** `tests/common/mod.rs` is a hand-rolled HTTP/1.1 server on
`127.0.0.1:0` serving all three base families from a `World` the test writes — the
four HoYoPlay bodies, both `getBuild`s, the manifest archives and the chunk
blobs, all synthesised, none checked in as a fixture file. `capture_layout.rs`
(18 tests) drives the real **binary** end to end; `matrix.rs` (8) and `schema.rs`
(11) read the bundles those runs wrote back off disk.

**"Touches no network" is proven, not asserted.** Two tests read
`api/requests.ndjson` out of a captured bundle and check that *every* recorded
URL is a loopback address the mock owns — including the manifest and chunk CDN
requests, whose bases come from the `getBuild` body the mock itself wrote. There
was no other address to reach.

**Four real defects the suite found, all now fixed.** Each is a bug that could
not have been found any other way, which is the argument for the milestone.

1. **`--no-chunk-blobs` could never be green.** A blob the index lists but does
   not store produced an A6 row with `expected: measured` /
   `actual: not measured` — i.e. **red**. The same held for A7-A12, which emitted
   `expected: asserted` / `actual: not asserted` as red rows. A declared gap was
   being rendered as a failure, so the flag was effectively an assertion that
   always failed. The design already had the right mechanism — the `skipped` list
   is what makes `Selfcheck::build` answer `incomplete` rather than `fail` — and
   A6 has now joined A7-A12 in it. A metadata-only run is `incomplete`, with zero
   red rows.
2. **`--no-chunk-blobs` also skipped `game/config.ini`.** `write_prelude` was
   inside the "are we assembling?" branch, but `config.ini` and the audio scan
   file come from `getGameConfigs` and `--audio`, not from a chunk. A
   metadata-only bundle had no `game/` at all, so A13 failed on a tree that simply
   had no config in it. The prelude now always runs.
3. **The matrix spelled `coveredby` and `overbudget`.** The status came from
   `format!("{:?}", row.status).to_lowercase()`, which is right for the two
   single-word variants and wrong for the two that matter. A bundle in which one
   class was covered by another wrote a status **its own A17 row rejects** — and
   the run still exited `0`, because the only test that could have seen it
   compared against the same wrong string. `MatrixStatus::id()` now spells each
   variant once, and a unit test pins the four spellings against the schema doc.
4. **`checksums.json` digested the stored bytes of a `.gz` file** while
   `Encoding::Gzip`'s own doc comment promised the decompressed ones. A reader who
   gunzipped a document and ran `md5sum` got a different answer than the document
   gave them. Nothing caught it because A16 re-hashes with the same function that
   wrote the digests, so both sides agreed. `digest_file` now streams a
   `GzDecoder`. **Bundles written before this fail A16 on re-verification**, by
   design — re-capture for a consistent bundle.

Three more, smaller and also fixed: `RequestLog::records()` was documented as
returning records "in `seq` order" and returned completion order, so
`requests.ndjson` had a `seq` column that contradicted its own row order under
`--jobs > 1` (the existing unit test asserted the bug); `--files` named a path
neither manifest carries and nothing reported it, despite a comment promising
"M8 reports it" — the run now carries `forced_files_requested` /
`forced_files_skipped` and warns per path; and `patchmaps/` was never created
when nothing landed in it, so `verify` refused a `--no-chunk-blobs` bundle as
unreadable (exit `1`) instead of reporting the finding a reader wants.

Two additions to the artifact, both because a test asked a question the bundle
could not answer. `MANIFEST.json` gained **`gzip_threshold`**, because the
`--gzip-threshold` help says "the decision is recorded" and it was not recorded
anywhere — a reader holding a bundle could see which files were `.gz` but not
why. It **defaults on read** rather than being required: `schema_version` is still
`1`, so making a newly-added field mandatory would have turned a
backwards-compatible addition into `verify` refusing a perfectly good bundle.
And `MatrixStatus` now has `is_acceptable()` and `selfcheck::MatrixRow::ok_status()`
cross-checked against each other, because two copies of the A17 literal could
disagree and nothing would have said so.

The mock's own design notes are worth keeping, because both were bugs: its
`categories` were originally hardcoded to `[game, en-us]` while `five_classes()`
published only `game`, and it lied — the branch claimed a manifest the build did
not carry and the capture failed at the join for a reason that had nothing to do
with the code under test. They are derived from the builds now. And every body is
rendered by a 40-line order-preserving JSON value rather than by `json!`, because
`serde_json::Value` is a `BTreeMap` and would sort every key — which is invisible
to the tool and fatal to the "stored exactly as served" claim the byte-exactness
test exists to check.

### M11 — first real capture
Run against `hk4e_global` 7.0.0 → 7.1.0, selection pool `game` + `en-us`, all
nine classes.
**Accept:** exit 0; `selfcheck.json` `result: "pass"`; artifact under
`artifacts/hk4e_global/7.0.0__7.1.0/<id>/`; then:
1. `gircap verify` on it, offline, exit 0;
2. `gircap materialize` on it, offline, and the rebuilt files match `game/` and
   `expected/` byte-for-byte;
3. `diff -r game/ expected/` lists exactly the non-`identical` variant files;
4. **DONE** — hand `game/` to `girpr` and confirm it reaches `7.1.0` with
   `SUMMARY download_bytes` equal to each capture's
   `patchmaps/<id>.json → totals.download_bytes`. *(Written when this was BLOCKED
   on `girpr` having no path include/exclude; see §1 and the addendum below. It
   shipped, the run was done by hand against a scratch copy, and M11 is complete.
   Never automated, because `gircap` must not depend on `girpr`.)*
**Commit:** `gircap: M11 - first live capture (end-to-end check blocked on girpr path filters)`

**M11 is complete.** Steps 1–3 landed here; step 4 landed later, once `girpr` shipped
path scoping — see the step-4 addendum at the end of this milestone, which records
the run and the figures.

> *Superseded by M12's re-capture at
> `artifacts/hk4e_global/7.0.0__7.1.0/20261015T092615Z/`, which carries the
> corrected one-sided patchmaps. The figures below are M11's and are kept as the
> record of that run; re-run the steps below against the newer bundle, whose
> `verify` reports 90 checks rather than 83.*

Run: `gircap capture --out artifacts --audio en-us --old-tag 7.0.0 --new-tag
7.1.0`, 27 s, exit `0`, bundle at
`artifacts/hk4e_global/7.0.0__7.1.0/20261014T055348Z/`. 8 captures admitted, 8
classes satisfied, `full_reuse_reordered` `unavailable` with its reason.
38 blobs / 27 612 723 B, 0 A6 failures, 49 requests, 47 591 582 B down, 56
documents written. `selfcheck.json` is `pass`: **115 rows, 115 passed, 0
failed** - A1 6, A2 2, A3 4, A4 8, A5 0, A6 38, A7 7, A8 7, A9 6, A10 4, A11 14,
A12 6, A13 1, A14 1, A15 8, A16 1, A17 1, **A19 1**. A5 is 0 because
`getBuild.stats` reconciled for all four manifests, which is the correct outcome
and not a gap.

1. **`gircap verify` on it, offline: exit 0.** 83 re-derived rows, 0 failed,
   `agrees_with_recording=true`, 8 captures and 62 216 590 B rebuilt from
   `chunks/`, 0 findings.
2. **`gircap materialize` on it, offline: exit 0**, and **14 of 14**
   manifest-derived files reproduced byte-for-byte by SHA-256. The two files it
   does not write are `game/config.ini` and `game/.../audio_lang_14`, which come
   from `getGameConfigs` and `--audio` rather than from a chunk - and the bundle
   still passes A16 afterwards, which is the point of M9's writing beside rather
   than inside.
3. **The variant set is exactly the non-`identical` captures.** `game/` only:
   `config.ini`, `audio_lang_14`, `00664172.blk` (the `removed` capture).
   `expected/` only: `libHttpClient.GDK.dll` (the `added` capture). In both and
   differing: `HoYoChannel.dll`, `unity default resources`, `00981562.blk`,
   `09801749.blk`, `data_revision`. In both and **identical**:
   `browser_assets` - the `identical` capture, and the only file the pair leaves
   untouched. 7 non-identical captures, 7 variant files, plus the 2 tool-written
   ones. No capture is silent and no variant is unaccounted for.
4. **BLOCKED - not skipped.** Re-confirmed against `girpr --help`: the flag set is
   `--game-path`, `--biz`, `--audio`, `--io-threads`, `--purge-after`,
   `--purge-before`, `--check-only`, `--dry-run`, `--json-summary`,
   `--log-level`. Nothing narrows a path, so a run against this 9-file fixture
   would plan from the whole live manifest and fetch ~130 GB - and
   `--purge-after` would then delete most of the fixture. The procedure and the
   exact expected numbers are written into
   [`00-spec.md` &sect;8.1.1](00-spec.md) (the six patchable captures sum to
   **2 373 385 B**), so the check is a comparison to run rather than a procedure
   to reconstruct. It stays manual: `gircap` must not depend on `girpr`.

#### Step-4 addendum, after `girpr` shipped path scoping

This sub-step was BLOCKED when it was written, and the reading above was correct
*then* and wrong now. It is kept as the record of that run rather than edited into
agreement with a tool that had not shipped yet.

`girpr` has `--include` / `--exclude`, so the run became possible. **It was done,
by hand, against a copy of `game/` from `20261015T092615Z`.** All eight captures,
every one exit `0`:

| capture | `download_bytes` | patchmap says | md5 | reused, not downloaded |
|---|---|---|---|---|
| `identical` | 0 | 0 | ok | 1 chunk |
| `zero_reuse_single` | 21 | 21 | ok | - |
| `zero_reuse_multi` | 336 852 | 336 852 | ok | - |
| `partial_same_offset` | 274 426 | 274 426 | ok | 1 chunk |
| `partial_moved_offset` | 1 564 894 | 1 564 894 | ok | 8 chunks |
| `partial_size_changed` | 197 192 | 197 192 | ok | 9 chunks |
| `added` | 114 357 | 114 357 | ok | - |
| `removed` | 0 | 0 | *gone* | - |

**2 487 742 B spent**, which is the sum of the patchmaps' figures to the byte -
and the first time that sum has been *earned* rather than asserted, since M12's
correction to `added` is what made 114 357 a real number rather than a defaulted
zero. `removed` reported `deleted_extra_bytes=1731133`, exactly its `old.size`.
Across all eight runs only the captures' own paths changed: the `added` file
appeared, the `removed` file disappeared, the other seven were modified in place,
and nothing else moved - so the scope held over every run. `config_bumped=0`
throughout and `local_manifest=ok` on every one, the latter being what rules out
the degraded-but-green shape where a run that reused nothing still exits `0`. The
bundle itself was never a working tree and still verifies 90/90.

Assertion 3 needed one correction that is worth keeping: the first attempt
searched the trace log for the chunk ids in `must_not_fetch` and reported 19
violations. All 19 were *reused*, not downloaded - a reused chunk is logged as
`reuse slice md5 ok -> copy chunk=<id>`, so the match has to be on
`chunk downloaded chunk=<id>`. A check that reports violations on correct
behaviour is worse than no check, because it trains you to ignore it.

Two things this needed and that were not written down anywhere before:
`removed` takes `--purge-only` rather than `--max-files 1`, and it is the one
capture whose assertion is a byte count the artifact does *not* carry -
`deleted_extra_bytes` against `old.size`.

**M11 is complete.** The end-to-end check ran, by hand, and passed.

**One assertion was added because M11 ran the thing.** Comparing `game/` against
`expected/` by hand - step 3 - turned out to be checking something nothing
asserted. A14 asks whether `game/` is free of paths the tool under test would
build a plan against, and it cannot see the converse: a capture admitted for
`partial_same_offset` whose two variant files came out byte-identical would pass
A14, pass A7/A8 (each side matches its own declared MD5), and be useless, because
the fixture would hold no evidence of the patch it was taken for. That is
**A19**, and it is the second id this project has added beyond the spec's
seventeen (A18 being `verify`'s closure check). It skips one-sided captures,
whose file is absent from one tree by construction, and it is **skipped** under
`--no-chunk-blobs` for the same reason A6 is: with no assembled sides the check
would pass vacuously, and a green row about nothing is the exact failure mode M10
had to fix twice already.

Two things M10 changed showed up here, which is the argument for running M11 after
M10 rather than before. `MANIFEST.json` carries `gzip_threshold`, and
`checksums.json` re-hashed 95 files / 208 295 423 B with every digest matching -
under the corrected rule, so the bundle is consistent in the sense the schema
documents. And the matrix spells `covered_by`; no row came out `coveredby`. The
eleven bundles written before those fixes are archived under
`artifacts/_pre-m10/` and now fail A16 by design, which is the honest reading of
a digest list that recorded compressed bytes while promising decompressed ones.

---

### M12 - the one-sided patchmaps

Not a planned milestone: a review of the M11 bundle. It found that
`patchmaps/added.json` emitted `"plan": null`, `"totals": null` and
`"expectations": { … "download_bytes": 0 }`, and `patchmaps/removed.json` emitted
no `expectations` at all - while three documents described a different shape and
not one test looked.

**The defect was not the shape. It was that the `0` was false.** The `added`
file's single chunk is 114 357 compressed bytes and `chunks/index.json` lists it
in the `patch_fetch` set, so a consumer repairing that path must download 114 357
bytes. An assertion written as "`SUMMARY download_bytes` equals
`expectations.download_bytes`" would have compared 114 357 against 0 and called a
perfectly correct run a failure. A defaulted `0` standing in for unknown data is
worse than a `null`, because a `null` reads as unrecorded and a `0` reads as a
claim.

1. **`added` gets the plan the schema promised.** Every entry `FETCH` with
   `source_range: null`, a `note` saying why, and `url`/`chunk_id`/
   `compressed_size` populated - so `totals` reads `fetch: 1, reuse: 0, moved: 0,
   download_bytes: 114357`. The writer now *refuses* to emit a capture that has a
   new side but no plan, so the defect cannot return quietly.
2. **`removed` gets an `expectations` of its own shape**: `absent_after: true`,
   `result_path`/`result_md5` null, `download_bytes: 0`, `must_not_fetch: []`.
   That `0` is a *true* value - deleting a file downloads nothing - and it is
   written down rather than left null precisely because a consumer should be able
   to compare against it. `expectations` is now required on every capture.
3. **A20**, a new `verify`-only assertion, re-sums `compressed_size` over each
   plan's `FETCH` entries and recounts `reuse`/`fetch`/`moved`, then checks that
   `expectations.download_bytes` is that figure. A20 immediately turned red on
   M9's own hand-built bundle fixture, whose `partial_same_offset` capture also
   carried `plan: null` - so the fixture was made realistic rather than the check
   loosened.
4. **The two verified-but-unrecorded ids are now named.** `SUMMARY` said
   `checks_derived=A16,A6,A11,A7,A8,A13,A17` while `verify` had been re-deriving
   A18 all along. `REDERIVED` (cross-checkable against the recording) and
   `DERIVED_UNRECORDED` (A18, A20 - statements about the bundle that no capture
   writes) are now separate lists, and `checks_derived=` names all nine. A reader
   who cannot tell what was checked from what was merely recorded will read that
   list at face value.

Three tests were added, and one was proved to fail on the old shape rather than
assumed to: reverting the `added` arm and the writer's guard reproduces
`"plan": null` and the new test reports *"`added` has a plan: every chunk is
fetched whole"*.

**Proved against the real bundle too.** Re-captured live at
`artifacts/hk4e_global/7.0.0__7.1.0/20261015T092615Z/` (17 s, exit `0`, 116
selfcheck rows all green, A16 re-hashed 95 files / 208 289 563 B). `added.json`
now carries a one-entry `FETCH` plan with `compressed_size: 114357` and
`totals.download_bytes: 114357`; `removed.json` carries
`absent_after: true`. `verify` reports **90** checks, all green, nine derived ids
named. With the defect reintroduced into that bundle by hand, A20 reported *A20
`patchmaps/added.json (download_bytes)` expected=114357 actual=0* and the
expectations figure alongside it — and A16 caught the same edit independently
through `MANIFEST.json`'s own digest record. M11's four acceptance steps all still
pass: `verify` exit 0, `materialize` exit 0 with 14 of 14 files byte-for-byte,
and the variant set still exactly the non-`identical` captures.

**Two claims in the review were checked and not adopted *at the time*.** It
reported that the manual step''s reason for avoiding `--purge-after` was stale,
and that `--audio` is refused under a path scope. Both describe behaviour of a
`--include` flag that did not exist: `girpr --help` then offered `--game-path`,
`--biz`, `--audio`, `--io-threads`, `--purge-after`, `--purge-before`,
`--check-only`, `--dry-run`, `--json-summary`, `--log-level` and nothing else,
`purge::keep_set` was config-only with no glob, and `00-spec.md` §8.1 said
*"The blocker is unchanged"*. It also reported that M11 step 4 should be recorded
as run. It was not run and was still blocked. Writing down a pass for a check
that has not happened - and rewriting a correct reason into one that matches code
nobody has written - is the exact failure mode M10 had to fix twice.

> **Superseded by the merge.** `girpr` has since shipped `--include` /
> `--exclude`, and **both claims turned out to be right**. The refusal is real
> (`--audio` under a scope exits 1, naming the scan file), and the cleanup reason
> was indeed stale: `plan::purge::classify_purge_path` tests the scope first and
> keeps anything not visible, so a scoped cleanup cannot reach the rest of the
> fixture. Verified by hand - see the step-4 addendum under M11 and §8.1.3.
>
> The lesson survives the reversal, and it is the opposite of what the note
> wanted: the check that mattered was reading the tool, not the handover. A
> reviewer reading the flag list would have been right; a reviewer trusting the
> note would also have been right - but for the wrong reason, and only once the
> flag existed. Both routes to "no, it does not exist yet" were the right answer
> on the day, and only one of them had evidence behind it.

What *was* taken from that point, and survives the merge too, is the real kernel:
a `removed` path is absent from the new manifest by construction, therefore absent
from any repair plan, so the only lever that can act on it is a cleanup. That is
§8.1.2 of the spec rather than something to be discovered, and it is why `removed`
is the one capture whose run is a `--purge-only` cleanup rather than a repair.
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
| **The end-to-end check cannot run** | the artifact is never validated against the real tool | **Retired.** `girpr` ships `--include`, and the check has been run by hand against a scratch copy of `game/`: all eight captures exit `0`, every `download_bytes` matches its patchmap to the byte (2 487 742 B total, exactly the sum), 19 reused chunks were reused rather than downloaded, and only the captures' own paths moved on the copy. The bundle still verifies 90/90 (spec §8.1.1). `verify`/`materialize` continue to cover the offline half |
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