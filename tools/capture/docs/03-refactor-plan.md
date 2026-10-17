# Refactor & optimisation plan — `gircap`

A ranked, evidence-backed worklist for `tools/capture/`, **ordered from lowest
risk to highest**. Every item names the code it touches, the observable effect
(perf, lines removed, correctness), and the gate that proves it did not break
anything.

Companions: [`00-spec.md`](00-spec.md) (what the tool does),
[`01-artifact-schema.md`](01-artifact-schema.md) (the bytes it writes),
[`02-plan.md`](02-plan.md) (how it was built).

This document does **not** change the spec, the schema, or the milestone story.
It is the plan for making the existing thing cheaper and smaller without
changing what it means.

---

## 0. The baseline this plan is argued from

Measured on this checkout (`capture/refactor`, commit `884eeea`), 2026-10-17.
Re-measure before trusting any number; they are here so a reviewer can check
the reasoning, not so they can skip it.

### After, for comparison

Taken at `16886c8`, **every number below measured by the same script over both
trees** — `Measure-Object -Line` silently skips blank lines, so mixing it with
`.Count` makes the two columns incomparable and produces nonsense (it first
reported production lines exceeding total lines).

| | baseline `884eeea` | now `16886c8` | delta |
|---|---|---|---|
| `src/` production lines, excl. `#[cfg(test)]` mods | 13,410 | 17,022 | **+3,612** |
| of which non-blank, non-comment | 8,823 | 10,045 | **+1,222** |
| comment + blank share of production | 34% | 41% | +7 pp |
| in-module `#[cfg(test)]` lines | 9,252 | 13,602 | +4,350 |
| `tests/` lines | 4,773 | 5,758 | +985 |
| `src/` files | 32 | 42 | +10 |
| tests passing | 452 | **571** | +119 |
| clippy `--all-targets` | clean | **clean** | — |

**This refactor made the crate bigger, not smaller: +8,947 lines, +33%.** Two
thirds of the production growth is comments and blank lines, and the code itself
grew 1,222 lines. Any claim in this plan that a change "removes lines" should be
read as "removes them from one function", not from the crate — several items
added a named helper, a test and a note explaining why, and the note is usually
the bigger half. The one structural change is real though: **`src/run.rs` went
from 1,981 lines to 860**, and the largest file in `src/` is no longer `run.rs`
at all.

### What moved, measured

The numbers §8 asked for, taken on this machine after all bands landed.

    verify, 20261017T000730Z, warm cache    7.92 s  ->  1.86-2.01 s   (4.1x)
    cargo test -p gircap                  151 s    ->  24.4 s          (6.2x)
    capture wall clock, --jobs 1                             33.5 s
                          --jobs 2                             36.9 s
                          --jobs 4                             24.8 s
                          --jobs 8                             16.4 s   (2.0x)
    peak RSS during capture              151-154 MB   at every --jobs
    list --root artifacts                                    0.05 s

The suite got **6.2x faster while carrying 119 more tests**, which was §3.8's
whole point: it had to be cheap enough to run on every band-1 commit or the plan
would be an afterthought. Its invariants all still hold — 0 `#[ignore]`, 0
`thread::sleep`, 0 non-`.rs` fixture files, every capture on `127.0.0.1:0`.

**§3.2's prediction is wrong, and the measurement is why §8 asked for it.**
§3.2 says flat `--jobs` scaling "confirms the `RequestLog` mutex is the ceiling",
and calls that "the single most informative measurement in the list". It is not
flat: `--jobs 8` is 2.0x faster than `--jobs 1`, so the mutex is demonstrably
*not* the binding constraint here. The likely reason is that this workload cannot
express the contention — a capture fetches 38 chunk blobs and a handful of API
documents, so there is very little concurrency to serialise on. **The §3.2 claim
is untested rather than refuted**, and testing it properly needs a capture with
thousands of concurrent chunk fetches, which this fixture is not.

`verify` at 4.1x is the clearest single win in the plan, and §8 predicted exactly
where it would come from: "a 5-second verify on a warm page cache points at
hashing and decompressing, not at the disk". §3.4 (the verifier re-reading what
it just read) and §5.4 (one-pass digests, `spawn_blocking`) are both that fix.

### Shape

| | |
|---|---|
| source files | 39 `.rs`, **26,726 lines** — 32 in `src/` (21,191) + 7 in `tests/` (5,535) |
| largest file | `src/run.rs` — 1,914 lines, of which `capture()` is **887** |
| next largest | `assemble/mod.rs` 1,438 · `http/mod.rs` 1,422 · `select/mod.rs` 1,121 · `manifest/diff.rs` 1,026 · `verify/mod.rs` 995 · `config.rs` 992 |
| functions | 1,104 total; 22 over 80 lines, 6 over 150, **1 over 250** |
| `cargo clippy --all-targets` | **clean** on default lints, no warnings |
| production `unwrap`/`expect`/`panic!` | **12** (8 are `Mutex::lock().expect`), vs 399 inside `#[cfg(test)]` |
| tests | **452 passing, 0 ignored**, 151 s wall |

The codebase is *not* uniformly messy — clippy is clean, panics are almost
entirely quarantined in test modules, and 1,082 of 1,104 functions are under
80 lines. The problems are concentrated, and they are concentrated in exactly
the places that cost the most. A plan that said "this crate needs a rewrite"
would be wrong.

### The data it moves

| quantity | value | source |
|---|---|---|
| files in the `game` manifest | 2,744 | `docs/01-artifact-schema.md:95` |
| chunks in the `game` manifest | **113,610** | `docs/01-artifact-schema.md:262` |
| total chunk bytes in `game` 7.1.0 | **128.3 GB** | `hyp/types.rs:794` |
| manifest archive | 9,007,187 B zstd → 16,827,424 B | `hyp/types.rs:794` |
| largest single chunk | 3,689,476 B compressed | `manifest/mod.rs:792` |
| decoded `game` graph | ≈ 23 MB in RAM, ~200 B per chunk | derived from `manifest/proto.rs:60-75` |
| a default bundle | ≈129 MiB — 97 files, 4 manifests, **38 stored chunks**, 56 request rows | `README.md:43`, measured on `artifacts/hk4e_global/7.0.0__7.1.0/20261016T002107Z` (135,129,448 B) |
| admitted files per capture | 7 in `game/`, 7 in `expected/`, 8 captures | `docs/02-plan.md:287-291` |
| assembled bytes per capture | 62,216,590 B | `docs/02-plan.md:327` |

**Measured wall time, release build** — `gircap verify --artifact` against that
same bundle:

```
7.92 s
  files_checked=96  blobs_checked=38  captures_rebuilt=8  bytes_rebuilt=62,216,590
  checks_total=90  checks_passed=90  checks_failed=0  findings=0  exit_code=0
```

That 7.92 s is the honest denominator for every `verify`-side claim below: it is
~135 MB read and hashed, ~28 MB of blobs read **and decompressed more than
once** (§3.4), and 62 MB rebuilt from them. A capture against the live API is
**not** measured here — it needs network and a ~2 min run — which is why §8
asks for it separately.

### Three verified defects, found while auditing

These are cited throughout because they change how several "safe" items must be
gated. All three were confirmed against the real bundle in `artifacts/`, not
inferred from reading.

1. **`write_atomic` does not exist.** `emit/mod.rs:200` promises that "the
   large artifacts (blobs, game files) go through `tmp/`-and-move in
   `write_atomic`". A grep for `write_atomic` across `src/` returns exactly one
   hit: that comment. The only tmp-and-rename in the crate is the private
   `Store::write` at `blobs/mod.rs:350-366`, which covers blobs. The 22 MB
   `game/*.blk` and every `expected/` file are written with a bare `fs::write`
   at `emit/gamedir.rs:252`.
2. **All 6 `api/` rows have `stored_as: null`.** The six HoYoPlay/getBuild
   bodies *are* written to `api/*.json` (`emit/mod.rs:58`), and
   `docs/01-artifact-schema.md:157` shows the field populated. Chunk rows
   (38/38) and manifest rows (4/4) are populated; probe rows (8/8) are null,
   which is correct — a probe response is deliberately not stored. The cause:
   `hyp::CapturedBody::seq` is hardcoded `0` at `hyp/mod.rs:257` with the
   comment "filled in by the caller", and `hyp::attach_seq` — the function that
   would fill it — has no caller outside its own test.
3. **The request log records headers that were never sent.**
   `http/mod.rs:321-322` fabricates `accept: */*` and `accept-encoding: gzip`;
   the client is built at `http/mod.rs:549-553` with only `.timeout()` and
   `.user_agent()`. The live bundle's `requests.ndjson` records all three
   headers as sent. `requests.ndjson` exists to be auditable
   (`http/mod.rs:3-6`) and is wrong in the one field a reader would use it for.

---

## 1. What "risk" means here

This crate's risk axis is unusual and worth stating before the worklist,
because it is **not** "might it break the build". A bundle is a **fixture other
work depends on**; re-capturing one costs ~2 minutes and 129 MiB, and
`girpr`'s `tests/artifact_bridge.rs` (75 KB) runs against the copies in
`artifacts/`. So:

> **Risk = P(silently changing what a bundle says) × cost of noticing.**

That produces five bands. Each one has a gate; the gate is part of the item, not
an afterthought.

| band | meaning | gate |
|---|---|---|
| **0 — Free** | Nothing observable can change. Dead code, duplicated constants, redundant passes over data already in hand. | `cargo test` |
| **1 — Output-identical** | Internal algorithms and allocation change. The bundle bytes **must** come out identical. | band-0 test suite **+ byte-diff of two loopback captures** (§4) + `gircap verify` on `artifacts/` |
| **2 — Hostile-input hardening** | Output identical for well-formed input; behaviour changes only for inputs today's bundles do not contain (malformed manifests, poisoned locks, unreachable paths, burst failures). | band-1 gate **+ a new test per hardened input** |
| **3 — Artifact-visible** | A **new** capture writes something different. Existing bundles keep verifying but stop representing the tool. | §4 gate, **plus a re-capture** of one reference bundle and a diff of the changed documents |
| **4 — Out of bounds** | Touches the spec, the schema, the assertion ids, or the `girpr` contract. Listed so nobody mistakes them for cleanup. | do not do it as part of this work |

### The band-1/2 gate is not as hard as it sounds

`cargo test` already covers this crate well: **452 tests, none ignored**, and
the integration suites drive a **loopback mock server**
(`--hyp-base`/`--sophon-base`, `cli.rs`), so a full capture runs in-process
with no network. `tests/measured_fixture.rs` alone is 9 tests in 63 s and
exercises the whole pipeline. The missing piece is only a **byte-diff between
two captures of the same fixture**, which §4 specifies. Build it first; it is
~40 lines and it is what makes bands 1 and 2 routine instead of scary.

---

## 2. Band 0 — Free

No observable surface changes. Land as one PR, before anything else.

**≈200 lines out, zero behaviour change.** (~120 of dead code, ~60 of
duplicated constants and helpers, ~20 of provably-redundant passes; most other
rows are single-line edits inside larger functions.)

### 2.1 Dead code and test-only API

Every item was confirmed by exhaustive grep: definition present, zero callers
in `src/` and `tests/`.

| item | where | note |
|---|---|---|
| `config::Transport` — a shadow of `http::Transport` with the same 3 fields — plus its unused `immediate()` | `config.rs:293-313` | `run.rs:90-94` destructures it straight back into the `http` one. ~22 lines. |
| `CapturePlan/VerifyPlan/MaterializePlan.output` + their 3 parses | `config.rs:221,539,550,367,558,580` | `plan.output` is read by **nothing**; `main.rs:70` uses `common.output`. Deletes 3 fields and the 4× duplicate `--output` parse (`cli.rs:307` does it correctly — the other three are both dead and duplicated). |
| `config::DEFAULT_GZIP_THRESHOLD` | `config.rs:33` | duplicates `cli.rs:89`; the `cli` one is live (the clap default at `cli.rs:137`, the read default at `emit/index.rs:466`). `cli.rs:83-88` states the point of the constant: the flag default and the manifest's *read* default "have to be the same number". `config.rs` holds a second copy of that number, used only by its own tests. |
| `config::DEFAULT_AUDIO` | `config.rs:27` | referenced only by tests; the value that ships is the bare literal `"en-us"` at `cli.rs:129`. Wire it up (value already matches) or delete it. |
| `config::METADATA_ONLY_NOTE` — **and it is wrong** | `config.rs:36-37` | the note reads *"assertions A7-A10 are not provable without chunk blobs"*. The real skip list (`run.rs:800`) is **`["A6","A7","A8","A9","A10","A11","A12","A19"]`** — eight ids, not four. The only test guarding it, `assert!(METADATA_ONLY_NOTE.contains("A7-A10"))` at `config.rs:827`, checks the constant against its own text and cannot catch this. The note is also never surfaced to anyone. Either fix the text and assert it **against `Selfcheck::skipped()`**, or delete it. |
| `AudioSelection::is_some()` | `config.rs:277` | test-only; prod uses `codes()`. |
| `run::exit_with` re-export | `run.rs:1394` | `main.rs` imports `exit` directly. |
| `logging::resolve_log_path` | `logging.rs:241` | the identity function `explicit.map(Path::to_path_buf)`. |
| `logging::user_agent()` | `logging.rs:236` | allocates a `String` for a compile-time constant, once per `Fetcher`. Make it `const`/static. |
| `hyp::document_format()` | `hyp/mod.rs:333` | an identity function. |
| `hyp::attach_seq()` | `hyp/mod.rs:312` | no caller — **this is the cause of defect #2. Deleting it forecloses the band-3 fix; see §6.1.** |
| `Transport.jobs` | `http/mod.rs:261` | never read by `Fetcher`; `run.rs:534` uses `plan.jobs` directly. Either give it a job or drop it. |
| `RetryPolicy::immediate()` | `http/mod.rs:186` | test-only. |
| `Fetched::text()` | `http/mod.rs:474` | zero callers; `Observation::text()` is the one used. |
| `GetBuild::matching_fields()`, `Stats::compressed_bytes()`, `Category::is_audio()` | `hyp/types.rs:561,733,335` | test-only or unused. |
| `Checksums::get` | `emit/index.rs:192` | linear scan over entries, no callers. |
| `Report::to_selfcheck` and `derive(Clone)` on `Report` | `verify/mod.rs:299,209` | `Report` holds `rebuilt: Vec<Pair>` — the whole rebuilt fixture, ≈135 MB. A `Clone` derive on it is a live footgun even though nothing calls it. |
| `Layout::sorted`, `Range::as_std` | `assemble/ranges.rs:199,93` | test-only. |
| `roles_of`, `sets_of` | `assemble/mod.rs:896,904` | zero callers. |
| `Written::all` | `emit/gamedir.rs:169` | zero callers. |
| `materialize::ranges`, `materialize::tree_path` | `verify/materialize.rs:257,252` | test-only. |
| `digest_bytes`, `config_ini_md5` | `emit/index.rs:289`, `emit/gamedir.rs:243` | test-only. |
| `select::primary_class_satisfies_shape` | `select/mod.rs:347` | only caller is `tests/measured_fixture.rs:555`. |
| the `pool` parameter of `candidates_document` | `select/mod.rs:670` | literally `let _ = pool;`. *(Drop the parameter in band 0; actually using it is a band-1 perf item — §3.3.)* |
| `reqwest` features `json` + `stream` | `Cargo.toml:22` | neither is exercised — all decoding is `serde_json::from_slice`, and `bytes_stream`/`chunk`/`text`/`json` have zero occurrences. *(See §6.6: `stream` is wanted in band 2.)* |

### 2.2 Duplicated constants and helpers

| item | where | effect |
|---|---|---|
| `Class::ALL` and `Class::REQUIRED_ORDER` are the same 9 classes declared twice | `classes.rs:99,118` | `REQUIRED_ORDER` is documented "in no particular order" yet is byte-identical. A 10th class must be added in two places. Make it `= ALL`. |
| `9` as a literal in assertions and array types | `classes.rs:99,118,248`, `select/mod.rs:438,938,1044` | `Class::ALL.len()`. |
| `nchunks: 1`, and `reused > 0 && reused < nchunks` written 4× | `classes.rs:180,192,196,201,207` | one `fn is_partial(reused, nchunks)`; `is_partial` is the invariant most likely to drift. |
| 5 hand-rolled hex formatters | `hash.rs:39`, `emit/index.rs:99`, `report.rs:113,179`, `biz.rs:346`, `http/mod.rs:904` | one `hash::hex_bytes`. `emit/index.rs:99` allocates a `String` **per byte** (`b.iter().map(|x| format!("{x:02x}")).collect()`) — 96 allocations per file digest × 4 passes per capture. |
| `logging::init` registry block written twice; `AppendFile::write`/`flush` written twice | `logging.rs:172,183,96,105` | character-for-character duplicates. |
| the "does this assertion pass" row builder, twice | `run.rs:774-789` (A17), `run.rs:829-844` (A16) | one `fn row(id, subject, expected, ok, detail)`. |
| the "selfcheck row failed" log loop, twice — and the two disagree | `run.rs:692-695` omits `expected`; `run.rs:743-751` includes it | same filter, same message, one is a strict subset. Merge, keep the fuller one. |
| the identical sort comparator | `select/mod.rs:386-391` and `:514-521` | byte-for-byte identical, **in the same function**. The comment at `:508-512` records that drift in this pair once already caused a silent 108 B admission under a 100 B ceiling. One `fn cost_then_field_then_path`. |
| `Admission::satisfies` predicate written as a method *and* inlined | `select/mod.rs:338-339, 569-571` | use the method. |
| the `moved` predicate, 3 spellings in prod | `assemble/mod.rs:681,722,819` | one `PlanEntry::is_moved()`. |
| over-budget wording, two strings for one fact | `select/mod.rs:537-540, 580-582` | both land in the artifact (`Rejection.reason`, `MatrixRow.reason`) — **parameterise the wording, do not reword it.** |
| the `expected/` tree name, asserted in one module and written by another | `assemble/mod.rs:856` builds `PatchMap.expectations.result_path`; `emit/gamedir.rs:234,237` writes the tree | rename the tree in `emit` and the recorded expectation silently becomes a lie. One `const`. |
| `plan.per_class.max(1)`, duplicated 200 lines apart | `run.rs:307,508` | the *clamp* is band 2 (§5.5); here just note it so both sites move together. |

### 2.3 Redundant passes over data already in hand

Each of these recomputes a value that is already computed, or scans for a value
that a live index could answer.

| item | where | cost today |
|---|---|---|
| `decode_counts` dedups with `Vec::contains` | `manifest/mod.rs:527,540` | **3.76 M `&str` comparisons** per manifest (2,744² / 2), ×4 manifests. `diff.rs:267-270` explicitly argues O(n²) over these paths is unacceptable — the rule was not applied here. |
| `decode_counts` re-walks what its own loop already walked | `manifest/mod.rs:580` (`e.chunk_bytes()`), `:585-596` (4 more passes) | the preceding loop already has `cursor` for `Σ uncompressed_size` — the comment at `:566-570` says so. |
| `ChunkIndex::build` allocates a `String` key per blob per role | `blobs/mod.rs:79` | the three keys are already pre-seeded at `:72-75`. Use `get_mut` via `Borrow<str>`. |
| `failures()` / `violations()` build a `Vec` to report ≤7 `&'static str` | `props.rs:57-72`, `manifest/mod.rs:135-159` | called from inside a `filter` (`blobs/mod.rs:101`) and a `Display` (`props.rs:77`) — one `Vec` per blob, per format. |
| `diff::score` recomputes `old_bytes` in a separate pass | `diff.rs:405-409` | `index_file` (`:348-354`) is already walking those chunks. |
| `offsets.contains` on a `Vec<i64>` | `diff.rs:398` | the module's own doc (`diff.rs:368-370`) cites a hash appearing **400×** in one file. Sorted + `binary_search`, or a `HashSet`. |
| `assemble::totals` — 5 passes for 5 numbers, 3 already counted | `assemble/mod.rs:815-834` | have `patch_flow` return the counters it computes anyway at `:719-728`. |
| `select` counts `candidates_considered` with 9 separate passes | `select/mod.rs:451` | 9 × 2,921 predicate calls; one pass computes all nine. |
| `classes_in_order` does a linear `contains` inside a 9-iteration filter | `select/mod.rs:683-688` | `Class::parse` already exists and is O(9). |
| `Layout::of` builds `Vec<(i64,i64)>` only to convert it to `Vec<Range>` | `assemble/mod.rs:499-503` → `ranges.rs:137` | two vectors where one will do; `Layout::of` then walks it twice more. |
| `create_dir_all` per blob | `blobs/mod.rs:353` | every blob lands in the same `chunks/blobs/`; one redundant mkdir+stat chain per blob. |
| `report::to_json_line` allocates a `String` per key and clones the event | `report.rs:324,314` | keys are `&'static str` ASCII chosen by this crate — push them directly. |

**Band-0 note on `path_tsv`:** `manifest/dump.rs:192-221` and `:534-565` are two
writers with the same name, purpose and rules. Neither escapes a tab, so one
corrupt path corrupts both files identically. Merging them (§3.5) is band 1
because it must preserve the exact bytes; the `String::from(header)` with no
capacity at `diff.rs:536` and the seven `.to_string()` calls per row are band 0.

### 2.4 Documentation that contradicts the code

Zero build risk, and the highest-leverage band-0 work in this document — not
because the docs are wrong, but because **a wrong field name in
`01-artifact-schema.md` is an invitation to "fix" working code to match the
doc.** Band 3 changes artifact fields; doing that while the schema document
misdescribes them is how a compatible change turns into a breaking one.

Every item below was checked against the source.

| doc claim | reality |
|---|---|
| `01-artifact-schema.md:412-413` — `"verified": { …, "size_matches": true }` | **`size_matches` does not exist.** The real keys are `decoded_size_matches`, `md5_compressed_matches`, `md5_decompressed_matches`, `raw_size_matches` (`blobs/props.rs:34,40`; pinned by `tests/schema.rs:240-249`). The example also omits `decode_error`, which `tests/schema.rs:222` requires. **Highest priority in this table.** |
| `01-artifact-schema.md:367-368` — the `candidates.json` `columns` array | **missing `matching_field` and `rank`** (`select/mod.rs:673-677`; asserted `matrix.rs:216,376`). The doc contradicts itself — its own row example at `:373` carries `"rank": 1`. |
| `01-artifact-schema.md` §1 tree | **omits `diff/matrix.json`**, which `emit/mod.rs:184` writes and `measured_fixture.rs:596` references. It is in no doc tree and no test's expected-path list (`capture_layout.rs:539-572`). |
| `00-spec.md:499` — `--require-class` "default: all required" | the implementation requires **nothing** by default (`cli.rs:120`, `config.rs:660`). The reversal is recorded only in `02-plan.md:197-203` and is what `matrix.rs:316` asserts. `00-spec.md` §7 was never updated. |
| `00-spec.md:289` — `gircap materialize --offline` | **no `--offline` flag exists** (`cli.rs:186-197`). `00-spec.md:510` lists the command correctly, so §4 and §7 disagree. |
| `01-artifact-schema.md:362` — "unless `--max-candidate-rows` trims it" | **no such flag** anywhere in `src/`. |
| `00-spec.md:411` — "exits non-zero if any of **A1–A14** fails" | the table immediately above (`:393-409`) lists **A1–A17**, and A15/A16/A17 are recorded by `capture` too. The exit-code table at `:519-523` maps `3` to "an assertion failed" with no A-range — and never mentions `result: "incomplete"`, which a `--no-chunk-blobs` bundle exits `0` with (`capture_layout.rs:911,925`). |
| `00-spec.md:3` — "**Status:** spec (design agreed, nothing built yet)" | the crate is complete through M13 with 452 tests. **The single most misleading line in the doc set.** |
| `00-spec.md:204` — cheapest `zero_reuse_multi` candidate is `Resources/unity default resources` | the fixture uses `StreamingAssets/unity default resources` (`measured_fixture.rs:103,466`). |
| `02-plan.md` §2 module tree (`:56-105`) | **omits `src/hash.rs` and `src/run.rs`** — the latter is the largest file in the crate. The other 31 files match. |
| `02-plan.md` §2 tests tree (`:100-104`) and §4 (`:664-673`) | **3 of 7 test files undocumented**: `measured_fixture.rs` (28 tests), `offline_bundle.rs` (9), `cli_contract.rs` (19). |
| `02-plan.md:398-400` — "capture_layout.rs (18 tests) … schema.rs (11)" | actual **22** and **14**. |
| `02-plan.md:109-110` — dependency list | missing **`tracing-appender`** (`Cargo.toml:34`). |
| `02-plan.md:675-679` — "a **stored decoded manifest fixture**" | there is no stored fixture; `measured_fixture.rs:10-13` explicitly rejects one in favour of a synthetic corpus. The doc describes a strategy the code declined. |
| `02-plan.md` §3 M0-M4 (`:139-217`) | written in future tense with `**Accept:**` / `**Commit:**` pairs. M0-M4 all landed; only M5-M12 carry a `**Landed.**` addendum, so **half the plan reads as unstarted work.** |
| stale bundle figures, three ways | `01-artifact-schema.md:100` says `chunks_blobs: 41` (elsewhere 38); `:600` says `file_count: 96` (elsewhere 95); `:622,634` says `total: 71` (elsewhere 115, and "one A6 row per stored blob" = 38). |

Two more are **coverage gaps rather than doc errors**, and both are places where
a document asserts a guarantee that no test enforces — band 2, because each
needs a new fixture:

| doc claim | reality |
|---|---|
| `01-artifact-schema.md:638-642` — "a capture's `selfcheck.json` never contains an `A18` row" | **zero test coverage.** `capture_layout.rs:189-199` only checks `ok != true`. |
| `00-spec.md:436` — "A19 … **which nothing else in the suite does**" | **no test in `tests/` asserts A19 at all** — presence, count or content. It appears only in `src/run.rs:800,1540,1551` and `src/emit/selfcheck.rs:157,195`. The claim is that A19 is uniquely covered; the suite does not cover it. |

---

## 3. Band 1 — Output-identical

Internal algorithms and allocation change. The bundle bytes must not.
**These are where the performance is.** Ordered by risk within the band:
mechanical dedup first, hot-path allocation second, structural change last.

### 3.1 The four full-tree digest passes → two

**The single largest win in this document.**

A capture close-out walks and digests every file in the bundle **four times**:

| pass | site | work |
|---|---|---|
| 1 | `run.rs:794` → `manifest_file_list` (`run.rs:1754-1771`) | walk + md5 + sha256 every file |
| 2 | `run.rs:821` → `write_checksums` → `Checksums::scan` (`emit/index.rs:169-189`) | every file, again |
| 3 | `run.rs:822` → `Checksums::a16` → `findings` (`emit/index.rs:222-266`, re-digest at `:259`) | every file, a third time |
| 4 | `run.rs:848` → `write_checksums` again | every file, a fourth time |

Over a 129 MiB bundle that is ~500 MiB of md5+sha256 that only needs to happen
twice — and pass 3 re-derives digests that pass 2 computed and is holding in
memory. Also note `run.rs:1754-1771` is `Checksums::scan` re-implemented, with
`encoding_of` inlined from `emit/index.rs:277-283`; if that ever learns a third
encoding, `MANIFEST.json` silently keeps recording `identity`.

- Add `Checksums::scan_with(root, exclude)` (`emit/index.rs:169`) returning
  `(Checksums, findings)` from one read.
- Replace `manifest_file_list` with it, using the exclude it already needs.
- Collapse `run.rs:821/822` and the unconditional second `write_checksums` at
  `run.rs:848` into the two passes that are genuinely independent.

*Effect:* ~50% of capture close-out I/O. *Risk:* low — the A16 semantics
("re-hashing matches") are preserved as long as pass 3's independence from
pass 2 is kept. Note `run.rs:825`'s `if !selfcheck.by_id().contains_key("A16")`
is always true; the "conditional" is not conditional.

### 3.2 The chunk-store and diff hot paths

Measured against 113,610 chunks and 2,744 paths.

| item | where | cost today |
|---|---|---|
| **`RequestLog::set_stored_as` is O(n²) under a process-wide mutex** | `http/mod.rs:421-426`, called once per chunk at `blobs/mod.rs:301` | it takes the global `Mutex` then does `records.iter_mut().find(\|r\| r.seq == seq)`. At 113,610 records that is a **~23 MB memory walk per call, holding the lock**. On `--jobs 8` the 8 workers fully serialise on exactly this line, destroying the concurrency `blobs/mod.rs:196` bought. Fix: index by `seq` — `reserve()` at `:355-359` already produces dense `1..N` ids, so a `Vec<Option<_>>` slot or a `BTreeMap<u64, usize>` is exact. |
| `diff` allocates a `String` key on **every** chunk lookup | `diff.rs:356-364` (`into_key`), used at `:351,395,457` | **~454,000 `String` allocations per `game` field diff.** `(&str, i64)` is already `Hash + Eq`; keying directly deletes `KeyExt` outright. |
| the old-manifest per-file chunk index is **built twice** | `diff.rs:271-275` vs `:431-435` | 113,610 inserts × 2 for identical data, from identical inputs, in the same call tree. `old_by_path` is right there in scope — pass it as a parameter. **≈40% of `diff` runtime.** |
| `build_md5_path_index` builds values nobody reads | `diff.rs:371-381` | `HashMap<&str, HashSet<&str>>` over all 113,610 old chunks; the only consumer is `contains_key` at `:451`. The `HashSet`s are never read — up to 113,610 allocations discarded. A `HashSet<&str>` is 100% of what is needed. |
| `retcode` — one field, five parsers | `http/mod.rs:710`, `:510`, `probe.rs:157`, `hyp/mod.rs:77`, `biz.rs:270` | the same `getBuild` bytes are deserialised 4-5× per run with no cache. *(Unify only `http/mod.rs:710` and `:510`; the other three carry deliberately different defaults — `-1` at `probe.rs:157`, `i64::MIN` at `hyp/mod.rs:77`, and a hand-walk of `/data/game_branches` at `biz.rs:270` — so unifying them is a behaviour change, not a refactor.)* |
| `RequestRecord` is deep-cloned only to read a `u64` | `http/mod.rs:663`, `:628` | `self.log.push(record.clone())` then `Fetched{body, record}`; the caller uses **only** `.seq()` (`blobs/mod.rs:301`, `manifest/mod.rs:499`). ~25 heap allocations per chunk, discarded. Return the seq. |
| the header map is rebuilt per record | `http/mod.rs:880` | 3 `String`s + a `BTreeMap` per request. Build once on the `Fetcher`. |
| `header_map` lowercases names the `http` crate already lowercased | `http/mod.rs:898` | `HeaderName::as_str()` is guaranteed lowercase. One wasted `String` per response header × ~8 × 113,610. |
| `Report` writes with `println!` per record and no `BufWriter` | `report.rs:383-391` | `println!` re-locks stdout internally. Irrelevant for the 1-record commands; `list` is the only unbounded producer. `exit_with` (`exit.rs:108-112`) already flushes. |

### 3.3 Selection and assembly

| item | where | cost today |
|---|---|---|
| `select`'s want loop rescans the whole pool per class | `select/mod.rs:469-481` | `pool.iter().find` restarting from index 0 each `while` iteration, with an O(A) and an O(W) `String` scan inside the predicate. **≈420,000 `String` comparisons** at the default `--per-class 1` over a 2,921-entry pool, and `--per-class` has no upper bound (`cli.rs:111`). The pool is already sorted cheapest-first at `:386-391`: keep a **cursor** and hoist a `HashSet<&str>` of pool paths. |
| `file_pairs` linear-scans 2,744 entries per admitted path | `blobs/mod.rs:389-393` | O(A × n) per side, twice per path. **The index that removes it already exists**: `diff` builds `old_by_path` at `diff.rs:271` and `new_paths` at `:280`, and `run.rs:414` throws the map away that `run.rs:282` just built. Pass `&[FieldDiff]`. |
| `retention` is computed twice per pair, and clones to compare | `chunks.rs:289-293,310,394` vs `run.rs:417` | `Identity::of` does `c.uncompressed_md5.clone()` — a 32-byte allocation **per new chunk, purely to be compared and dropped** — then `Vec::contains` does O(old) tuple compares. `Identity` is already `Hash + Eq` (`chunks.rs:72`). Thread the `Retention` `run.rs:417` already built instead of recomputing. |
| `Retention.reuse` is a `Vec<String>` scanned linearly per chunk | `chunks.rs:242,397` | **O(new_chunks²) per file** with 49-byte string compares. |
| `wants` deep-clones every `Want` per spawn | `blobs/mod.rs:207,226` | each `Want` carries a full `SophonChunk` (4 `String`s) plus an **unbounded** `references: Vec<Reference>` — and each `Reference` carries 4 more `String`s (`chunks.rs:98,100,116,118`). `Arc<Want>`, or take it out of the vec. |
| `references` is scanned linearly on every insert | `chunks.rs:369-372` | insertion is O(R) for a chunk shared by R files → O(R²) per chunk, with 4 allocations each. |
| `retention` pushes 5 clones of the same 49-byte id per chunk | `chunks.rs:300,309,311,313,317` | stored ~8× per want. |
| `build_map` deep-copies the entire plan | `assemble/mod.rs:889` | `plan.map(<[PlanEntry]>::to_vec)` — ~5 `String`s per entry, ~300 allocations per capture — then drops the original at `:612`. It only ever *reads* the plan (`:815-834`, `:860-867`); take it by value. |
| the assembler re-derives content identity that `blobs` already defines | `assemble/mod.rs:639-650` vs `chunks.rs:73-95` | two implementations of "is this new chunk's identity in this old file" — M6's A12 rule and M7's reuse rule. `assemble` also clones a 32-byte `String` per chunk as a `BTreeMap` key; `Identity` exists for this and is already `Hash`. |
| `candidates_document` discards the pool it is handed | `select/mod.rs:670` | rebuilds the same ordered list via `by_cost` (`diff.rs:579-593`), which clones every `PathScore` *and* its field `String` — ~5,500 allocations per capture — then discards it. It also derives `Shape` from `s.same_md5` (`:645-649`) while `candidates()` derives it from `row.state` (`:365-370`): **two sources of truth for one field.** |
| `diff::aggregate` computed twice per capture | `run.rs:492` and `:885` | one O(rows) walk + two sorted `Vec<String>`. Hoist next to `diffs`. |
| `flat` iterated 5 times for 4 sums | `run.rs:864-882` | one fold. |
| `rows.clone()` on every capture | `run.rs:817` | clones the whole assertion vector only so `rows.len()` stays readable at `:925-926`. |

### 3.4 The verifier re-reads and re-decompresses everything it just read

The crate's stated philosophy is "re-derive, don't trust" (`docs/00-spec.md §8`),
which is right. But it over-applies it to its own in-process work:

| item | where | cost today |
|---|---|---|
| every blob is read **3×** and zstd-decoded **2×** per `verify` | `verify/mod.rs:451` reads + `:495` decompresses; `assemble/mod.rs:455-456` reads + decompresses **the same blob again** | `ChunkSource::decoded` caches nothing *by design* (`assemble/mod.rs:437-442`), which is right for capture (rebuilds a file three times and wants the page cache, not three resident copies) and wrong for verify, where `check_blobs` has just read and decompressed the identical blob and thrown it away. An opt-in memo, or folding `check_blobs` into the rebuild, removes one full read+decode of every blob. |
| `chunks_for` + a per-call size map are O(N·B·R) | `verify/materialize.rs:46-83, 105-109` | `chunks_for` linearly scans **every blob's entire `referenced_by` vector** per side and allocates a fresh `Vec<i64>` per blob even when the answer is "no match"; `:105-109` rebuilds a `BTreeMap` of **every blob's size on every `rebuild` call**, and `rebuild` runs twice per capture. One `BTreeMap<(tag,capture,path,role), Vec<(blob,offset)>>` built once in `rebuild_all` (`:168`) → O(B·R). |
| two large buffers alive at once per comparison | `verify/mod.rs:560-561` | `fs::read` of the shipped file **while** the rebuilt copy of the same size is alive → 2× file size. *(Peak `Report.rebuilt` holding the whole tree is documented and deliberate at `:206-208` — leave it, but note the `Clone` removal in §2.1.)* |
| `ChunkIndex::get` is a linear scan of a **sorted** `Vec` | `blobs/mod.rs:93-95` | `blobs` is "ordered by id" (`blobs/mod.rs:47`), built from `BTreeMap::into_values()` (`chunks.rs:405`). `binary_search_by_key` is free. Only tests call it today, but it is `pub`. *(The right fix — and proof of the shape — already exists: `assemble::ChunkSource::from_index` rebuilds a `BTreeMap` at `assemble/mod.rs:406-428`, at the cost of cloning every id and md5 out of every blob. Have `ChunkSource` borrow instead.)* |

### 3.5 Six ways to write a file, four ways to write a JSON document

| item | where |
|---|---|
| **6 divergent copies of "write bytes into the bundle"** | `emit/mod.rs:201`, `emit/gamedir.rs:247`, `manifest/dump.rs:223`, `http/mod.rs:453`, `run.rs:1196` (inline), `logging.rs:222` — governing **23 call sites**. Two are byte-identical. One `fsx::write_rel`; ≈34 lines. |
| **4 copies of "pretty-JSON to a bundle document"** | `emit/mod.rs:189-194` (1 helper, 9 call sites — good), vs `manifest/dump.rs:100-101`, `:142-143`, and `hyp::render_json` (`hyp/mod.rs:328`). Plus `emit/mod.rs:61-74` repeats its own 7-line block twice. |
| **2 copies of `paths_tsv`** | `manifest/dump.rs:192-221` and `:534-565` — same header, same LF-only rule, same hand-rolled quoting. One `TsvWriter`. **Must preserve byte-for-byte** — add a test that both produce the same bytes for a fixed input before deleting either. |
| `to_string_pretty` then `format!("{x}\n")` | `emit/mod.rs:192`, `dump.rs:101,147` | `format!` allocates a second full copy of the whole document to append one byte. |
| `IndexedBlob`-from-`Want`, 6 copies | prod `blobs/mod.rs:260-268, 313-319, 338-346`; tests `:528,574,645` | ≈40 lines. |
| `assemble_side` ≡ `materialize::rebuild` | `assemble/mod.rs:499-538` vs `verify/materialize.rs:110-143` | the same tiling algorithm, ≈35 lines, with **two different error strings** — `assemble/mod.rs:515` "does not fit a {} B buffer" vs `materialize.rs:133` "is outside the file". Both strings land in the artifact: **parameterise the wording, do not unify it.** One `fn tile(ids, size, src) -> Result<(Vec<u8>, Layout), (String, String)>`. |
| A13 `config.ini` check implemented twice | `run.rs:1432-1456` and `verify/mod.rs:722-756` | same read → same `parse_game_version` → same verdict, ~25 lines each, different subjects. One shared `fn check_config_ini(root, tag)`. Same wording caveat. |
| `verify` and `materialize` share a 25-line tail | `run.rs:1097-1140` ↔ `:1213-1245` | one `fn finish(report, summary)`. ≈20 lines. |
| the `write_err` mapping hand-inlined 6× | `run.rs:464,476,496,510,551,631` | `write_err(&dir, e)` already exists at `run.rs:1401` and is used 12 times. All six callees return `io::Result`. **≈40 lines, the cheapest win in this document.** |

### 3.6 Memory: the full-decode write

`manifest/dump.rs` builds one of the largest documents in the bundle and holds
**~57 MB** doing it:

- `dump.rs:99` → `dump.rs:181` `entries: c.entries.clone()` — a deep clone of
  the entire ~23 MB / 113,610-chunk struct graph, for a document that is
  serialised immediately and dropped.
- `dump.rs:100` `to_string_pretty` → a ~27 MB `String`.
- `dump.rs:116` `hash::gzip` → another ~5-7 MB compressed copy **while the 27 MB
  String is still alive**.

Peak ≈ 23 + 27 + 7 = **~57 MB per manifest**, repeated for both game manifests
and each audio manifest (`run.rs:474-488`).

- `FullDecode<'a> { entries: &'a [SophonChunkFile] }` (`dump.rs:170-184`)
  removes the 23 MB clone. **Band 1** — serialisation is unaffected by borrowing.
- Streaming the JSON through `BufWriter<GzEncoder<File>>` removes the other 34 MB
  **but may change the gzip bytes**, which `checksums.json` records. → **band 3
  (§6.4)** unless a test proves the deflate output is identical for a fixed input.

Same shape, smaller: `dump.rs:193-204` builds a `Vec<(String,i64,String,usize,bool)>`
with two `String` clones per entry, sorts, renders to a `String` with a guessed
capacity, and hands `.as_bytes()` to `write_file` — ~200 KB of tuple Vec plus a
~250 KB String alive at once. Sort `Vec<&SophonChunkFile>` (zero clones) and
stream.

And `blobs/props.rs:163` materialises **every** decoded chunk solely to md5 it
and take its length, then drops it. The module doc (`props.rs:19-21`) accepts
"tens of megabytes of peak RSS", but at `--jobs 8` with a 3,689,476 B max chunk
that is **~30 MB of decoded buffers** concurrent with 8 raw bodies. A
`zstd::stream::read::Decoder` + `io::copy` into a counting+md5 writer gives both
numbers with O(1) extra memory. *(Band 1 — the numbers are the same, only the
allocation profile changes.)*

### 3.7 The structural one: split `capture()`

`run.rs::capture` is **887 lines** (`run.rs:85-971`) and does **11 distinct
jobs** — 22 `tracing::` calls and 31 `map_err(` closures inside that span.
`run.rs`'s module doc calls itself "command dispatch: the four entry points"
(`run.rs:1-5`); roughly **700 of its 1,707 production lines are not dispatch.**

The work is already delimited by numbered phase comments (`run.rs:121, 157,
182, 459`). The natural split:

| phase | lines | job | home |
|---|---|---|---|
| 1 | 86-119 | endpoints / transport / fetcher | `run` ✓ |
| 2 | 121-154 | metadata + `validate()` | `hyp` |
| 3 | 157-180 | tag probe | `http::probe` ✓ |
| 4 | 182-212 | manifest fetch loop | `manifest` |
| 5 | 214-277 | negative-probe planning | `http::probe` |
| 6 | 279-395 | diff → score → select → budget → `--require-class` | `select` |
| 7 | 397-457 | chunk-set planning, A12, byte ceiling | `blobs` |
| 8 | 459-556 | tree writes | `emit` |
| 9 | 558-713 | assembly loop + patchmap/gamedir writes | `assemble` + `emit` |
| 10 | 715-862 | closing assertions, MANIFEST.json, 2-pass convergence | `emit::selfcheck` |
| 11 | 863-970 | 80-key summary construction | `report` |

Plus `closing_assertions` (199 lines, `run.rs:1410-1608`) and
`api_and_manifest_assertions` (131 lines, `run.rs:1778-1908`) → `emit::selfcheck`;
`ManifestInput` + `build_manifest` (140 lines, `run.rs:1611-1751`) → `emit::index`;
`verify_manifest` (40 lines) → `manifest`. `capture` drops to ~40 lines of
orchestration. **≈600 lines move out of `run.rs`.**

**Why this is last in the band, and why it needs the §4 gate.** It is the
single highest-variance *mechanical* change in this document: it touches the
orchestrator, it has no unit tests of its own (coverage comes from the loopback
integration suites), and a mistake silently changes bundle content rather than
failing a build. Zero intended behaviour change, real chance of accidental
change. Do it **after** the perf work, so the byte-diff harness is already
trusted, and **never in the same commit** as anything else.

Two things to fix while it is open:

- `build_manifest`'s doc says "Every field is something the run already knew"
  (`run.rs:1644`) but `files` is `Vec::new()` at `:1748`, filled in afterwards
  at `:794`, and `bytes_on_disk` recomputed at `:795`. Two-phase construction
  the doc denies exists.
- 112 `.set(` calls across four `Record::summary(...)` builders
  (`run.rs:887-968, 1098-1129, 1214-1234, 1290-1312`).

### 3.8 The test suite is part of the plan, not beside it

§4 makes `cargo test` the gate for every band-1 commit, and there are ~12 of
them. The suite takes **151 s**, and most of that is redundant. Speeding it up
is a **prerequisite** for this plan being practical rather than an afterthought —
so it lands in band 1, where it is provably behaviour-free. Land it *early*
(see §9: step 2).

#### What is already right — do not lose it

| | |
|---|---|
| **no network, ever** | every capture points at loopback (`127.0.0.1:0`, `tests/common/mod.rs:422`) via `--hyp-base`/`--sophon-base`; `cli_contract.rs:34-43` uses the dead port `http://127.0.0.1:1` with `--retries 1 --timeout 2`. No packet leaves the host. |
| **no `#[ignore]`** | 452 tests, **0 ignored**. Nothing is silently skipped. |
| **no sleeps, no `Duration`, no retry loops** | the only timing-adjacent tokens are the `--retries 1` flag and its comment (`cli_contract.rs:32,39`). |
| **no checked-in fixtures** | `tests/` is 100% `.rs`; every fixture is generated at test time. §4's byte-diff harness has a mock to build on for exactly this reason. |

This is a better test posture than most crates have, and it is what makes bands
1-3 reviewable at all. Everything below is about *redundancy*, not coverage.

#### ~60 process spawns, and the suite knows it

| file | spawns/run | note |
|---|---|---|
| `tests/capture_layout.rs` | **30** | 14× `capture_ok()` + 9× `capture()` + 7× `run()` (`:116`) |
| `tests/cli_contract.rs` | ~22 | `:60`, `:372` |
| `tests/matrix.rs` | **10** | `:28`; `a_budget_turns_a_satisfiable_class_into_over_budget` (`:272-317`) runs the *identical* world **twice** (`:277`, `:283`) purely to derive `half` at runtime |
| `tests/schema.rs` | **2** | memoised via `OnceLock` (`:44`, `:75`) |

Each `Mock::start` rebuilds the whole world: `World::five_classes()`
(`common/mod.rs:228-293`) + `World::archive()` (`:329-364`, prost-encode + zstd
per field) + `build_table()` (`:643-871`, **229 lines** of JSON rendering).

**The clearest waste is self-documented.** `tests/schema.rs:37-42` justifies its
`OnceLock` in as many words — *"five tests paying for five captures makes the
suite slow for no extra coverage"* — and `tests/capture_layout.rs` then runs **14
identical `World::five_classes()` captures** across 22 tests. Apply the pattern
`schema.rs` already uses. This is the largest single contributor to the 151 s,
and it is why `measured_fixture.rs` (9 tests) takes 63 s.

#### Dead and vacuous test code

| item | where |
|---|---|
| **A tautological assertion** | `tests/offline_bundle.rs:463` — `assert!(!r.findings.is_empty() \|\| r.findings.is_empty());` asserts nothing under any input. |
| `Build::chunks()` | `tests/common/mod.rs:178-188` — never called; `build_table` re-implements it inline at `:761-770`. |
| `World::by_tag()` | `tests/common/mod.rs:307-315` — never called; `Mock` answers `getBuild` by direct map lookup at `:1004`. |
| ~30 more unreachable `pub` items | `tests/common/mod.rs` — masked by `#![allow(dead_code)]` at `:35`, so the compiler will never report them. The only `use` in the whole suite is `use common::{Mock, World};`. |
| `tests/measured_fixture.rs:1284-1307` | `the_measured_patch_reuses_one_chunk_and_fetches_two` re-checks `download_bytes == 274_426`, already asserted at `:314` and again at `:388`. |
| `tests/offline_bundle.rs:652-664` | `the_two_sides_of_the_fixture_really_do_differ` asserts properties of its **own hand-built bytes**, not of the crate. |

#### Duplicated helpers — one of which is a latent bug

| helper | copies | note |
|---|---|---|
| `percent_decode()` | `tests/common/mod.rs:1039-1061`, `tests/capture_layout.rs:95-113` | **the two are not equivalent.** The `common` copy handles `b'+' => ' '` (`:1050-1053`); the `capture_layout` copy is **missing that arm** — and it is the one that parses the tool's `SUMMARY` line (`:87,878,900,969,986`). `capture_layout.rs` already declares `mod common;` (`:14`). Latent bug, not just duplication. |
| `bin()` | `capture_layout.rs:22`, `schema.rs:16`, `matrix.rs:18`, `cli_contract.rs:19` | **3 of 4 hardcode `.exe`** and break on non-Windows. `cli_contract.rs:26` is the only correct one: `format!("gircap{}", std::env::consts::EXE_SUFFIX)`. |
| `run()` (process spawn) | `capture_layout.rs:115`, `schema.rs:25`, `matrix.rs:27`, `cli_contract.rs:59` | `schema.rs` and `matrix.rs` are byte-identical. |
| `collect()` (recursive dir walk) | `capture_layout.rs:775`, `schema.rs:431` | byte-identical. |
| `find_bundle()` | `schema.rs:101`, `matrix.rs:112`, `capture_layout.rs:144`+`:153` | **three** recursive implementations, all different. |
| `struct Scratch` (temp-dir RAII) | `capture_layout.rs:36`, `matrix.rs:40` | identical except the prefix string. |
| `fn json()`, `struct Run` | `schema.rs:124`/`matrix.rs:131`; `capture_layout.rs:64`/`cli_contract.rs:53` | identical / near-identical. |
| `gunzip()`, `md5_hex` | `capture_layout.rs:423` and `common/mod.rs:1084` | **the same crate in the same repo reimplements inflate and md5** that `gircap::hash` already exports (used at `schema.rs:332`, `offline_bundle.rs:40`). |
| the deterministic fake-chunk body | `measured_fixture.rs:1164-1169`, `offline_bundle.rs:35-37` | **character-for-character the same expression**; `common/mod.rs:74` is a third variant. |
| the `HoYoChannel.dll` measured fixture | `measured_fixture.rs:917`, `:1193`, `offline_bundle.rs:87` | modelled **three times** with the same offsets (0/64/192) and the same four chunk-id prefixes. |
| `SophonChunkFile` builders | `measured_fixture.rs:43`, `:904`, `:1175` | `:1175` duplicates `:904` **inside the same file**, and the two disagree: `:911` hardcodes `md5: "ff".repeat(16)` while `:1186` computes the real md5. |

#### Harness hazards

| item | where | note |
|---|---|---|
| **Temp dirs keyed on bare PID** | `schema.rs:45`, `schema.rs:77`, `cli_contract.rs:215,278,349,367` | `capture_layout.rs:42` and `matrix.rs:46` correctly use `{name}-{pid}-{thread-id}`; these use pid alone. Two concurrent `cargo test` invocations with recycled PIDs — or two test binaries — can share a directory. |
| `std::mem::forget(mock)` | `schema.rs:64,96` | leaks the thread **and** the `TcpListener` for the rest of the binary's life. |
| the heaviest single loop | `measured_fixture.rs:671-710` | `for cap in (0..8_000_000).step_by(7_919)` = **1,010 iterations**, each a full `select::select` + `rejected` scan. Legitimate (it is the budget sweep) and most of that file's 63 s. |
| ~21 assertions duplicated across files | e.g. "9 matrix rows" at `capture_layout.rs:810` and `matrix.rs:165`; `selection_pool == "game,en-us"` at `capture_layout.rs:852` and `matrix.rs:373` | the worst is #15: `capture_layout.rs:725-733` asserts `verify` re-derives 9 ids, `offline_bundle.rs:454-462` asserts a **different** id set for the same report, and the two are never reconciled. |
| wire-format coupling | **≈60-65 of 100 tests**; `capture_layout.rs` 21/22, `schema.rs` 13/14, `matrix.rs` **8/8** | ~228 raw `["key"]` JSON accesses. `tests/schema.rs:183-249` pins four exact key sets, so one rename breaks three tests at once. **This is load-bearing** — it is why band 3 needs a re-capture *and* a test review, and why the wrong key names in §2.4 matter. |

---

## 4. Band 1 gate — the byte-diff harness

Build this **before** band 1, not after. ~40 lines, in `tests/`, and it is the
thing that makes bands 1-2 routine.

```powershell
# capture the same fixture twice, against the loopback mock
gircap capture --out $env:TEMP\gircap-diff --hyp-base $MOCK --sophon-base $MOCK `
    --biz hk4e_global --old-tag 7.0.0 --new-tag 7.1.0
# ...repeat into a second directory, then diff the trees with the
#    non-reproducible fields normalised: capture_id, ts, elapsed_ms,
#    date/expires/age response headers, and any absolute path.
```

Normalisation is required, and it is worth being explicit about **why**:
`api/requests.ndjson` embeds `ts` and `elapsed_ms` per row
(`http/mod.rs:794-806`), so **no two captures of the same input are ever
byte-identical**. `checksums.json` covers that file (A16). So the gate cannot
be "byte-identical bundle" — it is "identical after normalising the fields that
were never deterministic", which is a stricter and more meaningful claim than
it first appears.

Then, per band-1 commit:

```powershell
cargo test -p gircap                                     # 452 tests, 0 ignored
gircap verify --artifact artifacts\hk4e_global\7.0.0__7.1.0\<id>   # must exit 0
# plus: the two normalised capture trees are identical
```

**Also do, once, before band 1:** `cargo test -p girpr --test artifact_bridge
-- --ignored` against the existing bundles. It is the consumer contract, it is
`#[ignore]`d by design (expensive), and it is the only check that knows whether
a refactor broke something the schema does not describe.

---

## 5. Band 2 — Hostile-input hardening

Output is identical for well-formed input. Behaviour changes only for inputs
**today's bundles do not contain**. Each item needs a new test that feeds the
bad input — that is the difference between band 2 and band 3.

### 5.1 Panics on the data path

The crate's panic surface is remarkably small — **12 production sites**, 8 of
them `Mutex::lock().expect`. Fix all 12; it is cheap and it is the difference
between "a panic" and "a finding".

| site | reachability | fix |
|---|---|---|
| `http/mod.rs:247,253,356,365,375,396,407,422` (8×) | `std::sync::Mutex` poisoning is **genuinely reachable** — a panic in any task holding the `RequestLog` lock poisons it for every other task, and the lock sits inside the concurrent chunk fetcher. The crate already *tolerates* panicking tasks (`blobs/mod.rs:217`, "a chunk fetch task panicked"), so the two policies contradict each other. | `lock().unwrap_or_else(\|e\| e.into_inner())` — a poisoned request log is still a perfectly good log for provenance. Removes 8 panics and aligns with the existing task-panic policy. |
| `blobs/mod.rs:549` `unwrap_or_else(\|\| panic!("{role} names a missing blob {id}"))` | **reachable from a hand-edited `chunks/index.json`.** `verify/materialize.rs:69-76` deliberately turns a bad index into a `String` error; this panic sits one layer below it. | return `Err`. *This is the crate's one genuinely exploitable panic.* |
| `run.rs:378` `.expect("every required class has a matrix row")` | provably unreachable — `missing` and the detail map use the identical predicate on the identical slice — but it is a panic in a function taking user-supplied `--require-class`. | `filter_map` removes the argument entirely. |
| `emit/index.rs:124` `strip_prefix(root).expect("under the root")` | unreachable in practice (`read_dir` builds its entries by joining the rooted path it was given). | `io::Error`, matching every other function in `walk`/`scan`/`findings`. |
| `blobs/mod.rs:352` `final_path.parent().expect(...)` | effectively unreachable. | `unwrap_or(root)`. |
| `manifest/dump.rs:275` `to_value(m).expect("ManifestMeta is all owned data")` | unreachable — all scalars, `String`s and `Vec`s. | `io::Error::other`, which is strictly better in a write path. |

**No `spawn_blocking`, no `catch_unwind`, and no `anyhow` anywhere in the
crate.** Everything is `Result<T, String>` or `Result<T, io::Error>`, re-wrapped
through `ApiError::shape` at ~10 sites in `run.rs`, which is why `Failure::detail`
is always `format!("{what}: {e}")`. That is coherent — but it means there are
**three** failure constructors (`Failure::usage`, `Failure::api`, and the free
`apierr`/`usage`/`assertion`/`write_err`) where `assertion()` duplicates
`Failure::api`'s struct literal with a different code. Adding an exit code means
editing three places.

### 5.2 Arithmetic on manifest-controlled `i64`

`SophonChunkFile`/`SophonChunk` carry `size`, `offset`, `uncompressed_size`,
`compressed_size` as **`i64`** straight off the wire (`manifest/proto.rs:32,66,68,70`).
Three sites treat them as trusted:

| site | problem |
|---|---|
| `manifest/mod.rs:559` `cursor = c.offset + c.uncompressed_size` | **overflow.** A manifest declaring `offset = i64::MAX` panics in debug/test builds and silently wraps in release. `cargo test` itself would abort. Use `checked_add`. This counter is a *measurement*; it must not panic on the thing it measures. |
| `manifest/mod.rs:551` `if c.compressed_size as u64 > max_chunk` | a **negative** `compressed_size` becomes ~1.8e19 and is written into the artifact as `max_chunk_compressed_bytes` — the field documented as "what bounds the main tool's whole-chunk buffering" (`manifest/mod.rs:174-178`). |
| `manifest/mod.rs:593-595` `.map(\|e\| e.size as u64)` | same, unguarded, and it flows into `reconcile` (`:623-627`) as a bogus mismatch. `diff.rs:484,489` and `blobs/chunks.rs:421` already do `.max(0) as u64` correctly — the fix is to copy them. |
| `assemble/mod.rs:512,633` `vec![0u8; entry.size.max(0) as usize]` | guarded against negative, **not against absurd**. `size = 2^40` attempts a 1 TB allocation → abort (not a panic, so not catchable). A named `MAX_ASSEMBLY_BYTES` guard turns a hostile-manifest abort into the tool's normal "this is a finding" error path. `verify/materialize.rs:122` has the same shape. |

### 5.3 Concurrency and blocking — the real performance story

The perf work in §3 is allocation. These are the structural wins.

| item | where | why |
|---|---|---|
| **`std::thread::sleep` inside `async fn get`** | `http/mod.rs:232-238` (`Sleep` is a *sync* trait), called at `:656-659` | with `--retries 10` the schedule is `0.5+1+2+4+8+10+10+10+10 = 55.5 s` of **thread-blocked** time per failing request, on a tokio worker, inside a `JoinSet`-spawned task (`blobs/mod.rs:203,222`). On a 4-core box with `--jobs 64`, four workers can be asleep simultaneously and the run stalls. `tokio::time::sleep` is available (`tokio` has `full`) and unused. Make the `Sleep` trait async; `RecordingSleep` (`:241`) is the test seam and must follow. |
| **6 fully serial RTTs where 2 suffice** | `hyp/mod.rs:274-287` | `configs`, `branches`, `deprecated`, `packages`, `build_new`, `build_old` are six consecutive `.await`s. Only `branches` is a true dependency (`package_id`); `build_new`/`build_old` are independent of each other; the other three are independent of everything. This is the *entire* network cost of a `--no-chunk-blobs` metadata run. |
| 7 serial tag probes | `http/probe.rs:133-185` | one `getBuild?tag=` per tag, serial. `diff_tags` is unbounded by contract (`hyp/types.rs:307-309`). `seq` is already reserved at issue time (`http/mod.rs:746`), so row order is preserved by design. |
| 7 serial negative probes | `http/probe.rs:439-466` | 7 RTTs for ~2 KB of 404 XML. |
| manifests fetched strictly serially | `run.rs:202-212` | `for field { for tag { manifest::capture().await } }` — `game` alone is 2 × 9 MB, one after the other. |

All four are the same shape. The crate has **no** `Semaphore`, `FuturesUnordered`
or `buffer_unordered` anywhere — and `futures` is not a dependency
(`Cargo.toml:18-35`). The pattern to copy already exists: the "prime N, refill
on completion" `JoinSet` loop at `blobs/mod.rs:196-228`. One
`Fetcher::map_concurrently(items, jobs, f)` built on it covers all four, and
gives `Transport.jobs` (`http/mod.rs:261`), currently a dead field, an actual job.

*Why this is band 2 and not band 3:* with every request succeeding, the same
**set** of rows is produced with the same `seq` values regardless of concurrency,
and `requests.ndjson` is not byte-reproducible anyway (§4). Only a
**partial-failure** run records a different sequence. Gate it with the loopback
mock so the row set can be diffed, and diff it on a run that deliberately fails
one probe.

### 5.4 Hashing: no incremental API, so every buffer is read twice

`hash.rs` exposes only whole-buffer one-shots (`md5_hex`, `sha256_hex`,
`gzip`, `zstd_decompress`). The crate needs **both** MD5 (manifest/CDN language)
and SHA-256 (`checksums.json`) over the same buffer, and every call site
therefore makes two full passes:

- `blobs/props.rs:160` + `:170` — md5 **and** sha256 over every raw chunk blob,
  then `:163-164` a full zstd decode of that same buffer and an md5 of the
  decoded bytes. At 113,610 chunks that is **≈128 GB read twice for the digests
  alone, plus ≈128 GB decoded** — on tokio worker threads, with no
  `spawn_blocking` anywhere in the crate.
- `manifest/mod.rs:473` + `:474` — the same over the 9 MB archive, plus `:427`
  over the 16.8 MB decompressed form.

**The crate has already solved this — in the wrong place.**
`emit/index.rs:80-107` `digest_file` feeds `io::copy` through a `Tee` writer
into both an `md5::Context` and a `Sha256::new()` in **one pass**, constant
memory, never resident, with a 100-line comment explaining why. It reimplements
what `hash.rs` claims to centralise (`hash.rs:1-7`: "the helpers live here
rather than being re-derived four times"), and reimplements `hex` again at
`emit/index.rs:99`.

Move `DualDigest` into `hash.rs`, have `digest_file` call it, and use it from the
blob path. Then wrap the decode+hash in `spawn_blocking`. Digests are unchanged;
only scheduling is.

### 5.5 Other band-2 hardening and behaviour changes

**Status: all sixteen rows closed.** `e0c7f9a` (row 10) was the last open one.

| row | outcome |
|---|---|
| 2, 3, 4, 5, 7, 11, 12, 14, 15, 16 | fixed, one commit each |
| 8, 9 | fixed in `59a1c72`, but both were **partly already handled** - see the annotations on the rows and the commit message |
| 10 | fixed in `e0c7f9a` (streamed ndjson; the clone-under-lock and the whole-document `String` are both gone) |
| 1 | **already satisfied** when reached: `hash::md5_sha256` is one `DualDigest` pass inside `spawn_blocking` since §5.4 |
| 6 | **decided, not outstanding**: `memoizing` exists and `verify` uses it; `capture`'s opt-out is recorded in the method's doc with the measurements behind it |

Rows 2 and 3 are annotated in place where this plan was wrong, not corrected
silently: Windows `fs::rename` *does* replace, so row 2's "re-capture would be
impossible" was never true; and row 3's fix had to distinguish `kind` values,
which would be breaking rather than additive. Row 16's `is_fatal` turned out to
be consulted at `emit/closeout.rs:280` already, with a test that proves it.

**Row 9's `bytes::Bytes` was declined.** All three benefits it promised were
already gone before it was reached - the `to_vec()` died in §6.6.2, and neither
`Fetched` nor `Observation` derives `Clone`, so there were no clones to make
free. Changing the field types would touch every consumer to buy a refcount
nothing uses.

**A stale reference bundle — and a gate claim in `e0c7f9a` that was wrong.**
`api/requests.ndjson` is one of the six documents that always differ between runs
(`ts`, `elapsed_ms`, CDN headers, `etag`), so the scratch band-1 comparison
reports it as "expected" whether it is right or wrong. When `7771323` filled in
the six null `api/` rows' `stored_as` — a correct, declared band-3 fix — **both
reference bundles went stale**: they carry 42 marked rows where the tool now
writes 48. Nothing failed, because `stored_as` is a claim about where a body was
written rather than a digest, and `verify` does not re-derive it.

`e0c7f9a`'s commit message concluded from this that "the band-1 gate cannot
detect a change to this file at all", and named a new checked-in field-level
test as the missing gate. **Both halves of that are wrong**, and the correction
is worth more than the claim was:

- `tests/bundle_diff.rs` already normalises `ts` and `elapsed_ms` and compares
  `api/requests.ndjson` **byte-exactly** against them blanked. Being in
  `DERIVED_PATHS` only exempts the *cascade* documents that record this file's
  digest; `normalize_doc` falls through to the plain `normalize` for the file
  itself, so every other field — `stored_as` included — is compared.
- And a checked-in gate already catches exactly the case that was thought
  missing: `tests/capture_layout.rs`'s
  `the_six_api_rows_record_where_their_body_was_written`, added with `7771323`.

Verified rather than argued. Deleting `log.set_stored_as(body.seq, &rel)` from
`emit::write_api` — the single line that fix turns on:

    bundle_diff        8 passed   MUTANT SURVIVES
    capture_layout     FAILED     left: []  right: [six api/*.json paths]

So a gate does catch it, and names both sides of the failure. The residual gap is
narrower than claimed: **`bundle_diff` cannot catch a *uniform* change to a file,
because it compares two captures of the same binary against each other.** A drift
that both runs share — which is exactly what `stored_as` becoming non-null was —
is invisible to it by construction. `capture_layout` covers the one field this
refactor touched, but that is a per-field gate and nothing generalises it to the
other fields, or to `MANIFEST.json` and `README.md`.

### The re-capture band 3 owed

§6's preamble says these items change what a **new** capture writes and that "a
re-capture is part of the work". `7771323` is the one that moved a *document's
content* rather than only a digest, so it is the one that made the stored bundles
stop representing the tool. Done: **`20261017T123335Z`** is now the current
reference, and all three bundles verify.

| bundle | `stored_as` marked | status |
|---|---|---|
| `20261016T002107Z` | 42 | valid, historical |
| `20261017T000730Z` | 42 | valid, historical |
| **`20261017T123335Z`** | **48** (6 `api/`, 38 `chunk/`, 4 `manifest/`) | **current reference** |

All three: `verify` exit 0, `checks_total=90 checks_passed=90 findings=0`, and
`list --root artifacts` reports `bundles=3 bundles_passing=3`. The two older
bundles are *kept*, not deleted: they verify because their digests are
self-consistent, which is exactly why nothing noticed they had drifted, and
`SCHEMA_VERSION` stays at `1` because every change was additive.

Against `20261017T000730Z` the new capture is **91 of 97 byte-identical**, with
only the six known-volatile documents differing — and every blob and every
assembled file identical, which is the part that has to hold:

    game/ 9/9   expected/ 7/7   patchmaps/ 8/8   chunks/ 39/39
    diff/ 5/5   manifests/ 16/16

`api/` is 7 of 9: the two that move are `requests.ndjson` and
`error_shapes.json`, both volatile for the recorded reasons.

**`artifacts/` is gitignored** (`.gitignore:3`), so promoting a bundle is a
filesystem action and adds nothing to a commit. A reviewer sees this only as a
diff to the numbers in this section.

### One trap worth writing down

PowerShell's `Copy-Item` **preserves `LastWriteTime`**. Restoring a mutated file
with it therefore leaves the source file *older* than the binary cargo last built
from the mutation, cargo considers the target fresh, and `cargo build --release`
reports `Finished in 0.50s` without compiling anything.

That happened here, and the symptom was a capture that looked fine — exit 0, 116
selfcheck rows passing, `verify` 90/90 — while carrying **0** `api/` marks
instead of 6. Every digest was correct, because the mutant was in
`emit::write_api`, not in anything `verify` re-derives. Only the field-level
comparison caught it.

The lesson is the same one as the §5.5 row 8 gate: **a mutation check that does
not confirm the binary was rebuilt proves nothing.** After restoring a file from
a backup, force the rebuild — `(Get-Item f.rs).LastWriteTime = Get-Date` — or
delete the binary. And when a capture's *content* disagrees with what the source
says it should be, rebuild before investigating anything else.

| item | where | change |
|---|---|---|
| `bytes_in` is **0 for every non-2xx** | `http/mod.rs:794-806` | the record is built, then `resp` is dropped **without its body being read** (`:808`; the read is on the success path at `:810`). A CDN 429/503 with a 4 KB envelope is transferred and counted as 0. The module doc (`http/mod.rs:20`) sells `bytes_in` as "the only way to price a manifest without re-downloading it", and it under-reports exactly the retried requests that cost the most. Well-formed runs are unaffected → band 2. *(The one-pass digest work in §5.4 pairs with this: hash the error body too.)* |
| Windows `rename` fails when the destination exists | `blobs/mod.rs:359-363` | ~~`fs::rename` on Windows fails if the target exists, so **re-capturing into an existing bundle directory fails on every single blob** — then backfills all of them with "fetched but not stored" (`:338`). This is a real, reproducible bug on the platform this repo develops on. Remove the destination first, or use a per-task unique temp suffix.~~ **CORRECTED — not a bug.** `std::fs::rename` calls `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, so the destination is replaced. Verified on rustc 1.98.1 and pinned by `blobs::tests::renaming_a_blob_over_an_existing_one_replaces_it`. Acting on the original text would have deleted the `tmp/`-and-rename step that keeps a half-written blob from ever appearing at its final path. Bundle directories are also `<timestamp>` to the second, so re-capturing into an existing one cannot happen through the CLI either. |
| two `Want`s sharing a `chunk_id` race on one temp filename | `blobs/mod.rs:200-209, 354-357` | the temp name is derived from the chunk id, so two tasks would write the same `.tmp` interleaved and one `rename` would publish a half-written file. Ids are unique today by construction; nothing enforces it. Per-task suffix, or `Arc<Want>` + a uniqueness guard. |
| `--per-class 0` is silently clamped to 1 | `run.rs:307` and `:508` | `config.rs:348` copies `per_class` with **no validation**, unlike `--jobs`/`--retries`/`--timeout` which are range-checked in `validate_transport` (`config.rs:376-404`). A user passing `0` gets an undocumented clamp instead of a usage error. Validate once in `capture_plan`. |
| `--require-class` is validated **after the whole network capture** | `run.rs:359` → `required_classes` (`:1001-1006`) | `config.rs:354` keeps the raw `Vec<String>` and parsing happens at `run.rs:359` — *after* both manifests per field, the diff and the full selection. A typo therefore burns an entire capture before being reported, contradicting the stated design intent at `config.rs:316-320` ("everything is validated before a socket opens"). Move `Class::parse` into `capture_plan`; store `Vec<Class>`. |
| `ChunkSource` caches nothing, by design | `assemble/mod.rs:437-442` | a chunk shared between old and new **is** the same physical blob (`chunks.rs:326-330`: "Keyed by `chunk_id` alone, because that is the content address") and is read+decoded+md5'd **twice** (`assemble_side(old)` then `assemble_side(new)`); a FETCH chunk twice more. ~30-35% of assembly is recomputed. A bounded memo of the current capture's decoded chunks is the cheap fix — the existing comment reasons about *holding three copies*, which a bounded LRU avoids. |
| `PatchOutcome` keeps a 22 MB buffer for one `memcmp` | `assemble/mod.rs:235, 610` | `Assembled` holds `old.bytes`, `new.bytes` **and** `patch.bytes` at once — ~68 MB peak per capture, contradicting the plan's own "peak residency at one file" (`docs/02-plan.md:302-305`). Compare-and-drop immediately. *(Keeping only a digest would weaken **A9**, which is specified as a *byte*-equality assertion (`docs/00-spec.md:401`) — that is a band-4 doc change, not a refactor. Do not do it here.)* |
| `decoded(...).into_owned()` copies every fetched byte twice | `assemble/mod.rs:660,711,729`; `hash.rs:121-122` | `zstd::decode_all` grows a `Vec::new()` with no capacity hint, then `out.extend_from_slice(&body)` copies it again. Write into a caller-supplied slice sized from `BlobProps::uncompressed_size` — which is **not currently carried into `IndexEntry`** (`assemble/mod.rs:50-53`), so add it. |
| `Fetched.body: Vec<u8>` forces a full copy of every response | `http/mod.rs:826` (`b.to_vec()` after `resp.bytes()`), `:613` | `bytes::Bytes` is already one contiguous, refcounted buffer. Storing it would delete the copy, halve peak memory per body, and make `Fetched`/`Observation` clones free. Touches every consumer → band 2, not band 1. |
| `std::fs` on the async path + whole-ndjson `String` | `http/mod.rs:453-460`, reached from `run.rs:715` | `to_ndjson()` materialises every record into one `String`, then `std::fs::write` — on a tokio worker, inside `async fn capture`. At 113,610 records (~200 B each) that is ~75-100 MB built on top of a full deep clone of every record (`http/mod.rs:371-390`, which clones **while holding the lock**). `tokio::fs` + `BufWriter`, streaming `records()` in chunks. |
| no response-size ceiling | `http/mod.rs:549-553` | the client has only `.timeout()` and `.user_agent()`. Combined with `bytes()` at `:613`/`:810`, a hostile or broken origin streams unbounded into RAM. |
| `Endpoints::owned` uses `Box::leak` | `biz.rs:197-202` | called once per run, so the leak is bounded and intentional — but nothing says so, and it silently accumulates if anyone calls it in a loop. `Cow<'a, str>` or a doc warning. |
| `verify` holds `raw` **and** its decompression alive | `verify/mod.rs:491-503` | `describe_bytes` keeps both while computing four digests. |
| `probe::with_param` is a second URL grammar | `probe.rs:479-495` | `split('&')` → `join("&")` with **no percent-encoding**, coexisting with `biz.rs:328-350` (`push_pair` + `enc`) that every real URL goes through. |
| `ErrorShape.retryable` is a hardcoded `false` claiming to be the policy | `probe.rs:464` | its doc (`:291-297`) says it *is* `RetryPolicy::should_retry_status` "stated in the artifact". It is not computed from it; it is currently true only because `observe` ignores retries entirely. Compute it, or fix the doc. |
| two "measure what the API does" paths with different retry behaviour | `probe.rs:153` (retries) vs `:440` (never retries) | both record `kind: "probe"`, so a reader cannot tell them apart in `requests.ndjson`. Give them distinct kinds, or document it. |
| `selfcheck::is_fatal` is never consulted | `emit/selfcheck.rs:176-197` | its doc claims it decides whether a run ends; `run.rs:702-712` hard-codes the decision instead. *Fixing the doc is band 1; **wiring it is band 3** — it changes which failures end a run.* |

---

## 6. Band 3 — Artifact-visible

These change what a **new** capture writes. Existing bundles keep verifying —
they are read, not re-derived — but they stop representing the tool, so a
re-capture is part of the work.

**Before starting band 3, settle the `schema_version` question.** `README.md:79-84`
commits to additive changes leaving bundles readable and the version bumping
"only for a breaking change to the bundle's byte layout". Every item below is
additive or a fix to a value that is currently *wrong*, so `SCHEMA_VERSION`
stays at `1` and **all eight reference bundles stay valid**. That is the cheap
outcome; confirm it rather than assuming it.

### 6.1 Fill in `stored_as` for the six `api/` rows — verified defect #2

`docs/01-artifact-schema.md:157` shows the field populated. Measured on the real
bundle: **6/6 `api` rows are null**, while `chunk` is 38/38 and `manifest` 4/4.
Probe rows are 8/8 null, which is correct.

Cause: `hyp::CapturedBody::seq` is hardcoded `0` (`hyp/mod.rs:257`) with the
comment "filled in by the caller", and `hyp::attach_seq` (`:312-324`) — the only
thing that could fill it — has no production caller. `emit::write_api`
(`emit/mod.rs:57-60`) writes the six bodies and never touches `stored_as`.

Wire `attach_seq` into the capture path and set `stored_as` to `api/<file>.json`
as each body is written. **This is why `attach_seq` is listed in §2.1 as
"deleting it forecloses the fix" — check before deleting.**

*Effect:* every `api` row gains the path it already has on disk. *Verify:*
re-capture one bundle, confirm `stored_as` is non-null on exactly the 6 `api`
rows and that nothing else moved.

### 6.2 Send the headers the log claims were sent — verified defect #3

`http/mod.rs:321-322` fabricates `accept: */*` and `accept-encoding: gzip`;
the client at `:549-553` sets neither. Move them to `ClientBuilder::default_headers`
so the record and the wire agree by construction.

**This changes the wire.** A real `Accept-Encoding: gzip` on a CloudFront-fronted
API may change the bytes that come back (today the bundle stores identity-encoded
bodies). If that happens, `bytes_in` and every body digest change → a genuine
layout change → `SCHEMA_VERSION` and a full re-capture. **Test this against the
live API once, deliberately, before committing to it**, and be ready to drop the
`accept-encoding` half and keep only `accept`.

### 6.3 Record a blob that was fetched but is not a zstd frame

`blobs/mod.rs:293`: `BlobProps::measure(...).map_err(|e| e.to_string())?` — a blob
that **was fetched** but has no zstd frame (`props.rs:149-159` returns `Err`)
makes `Store::one` bail through `?`, so the bytes are **never written**, and the
slot is backfilled at `:242` with `unfilled()` whose `decode_error` reads
**"fetched but not stored"** (`:338`). The real reason is discarded and the
bundle claims the fetch failed when it succeeded.

This inverts the module's stated purpose — `blobs/mod.rs:13-16`: *"a capture that
hit a truncated response is the evidence this tool exists to keep"*. Use
`unwrap_or_else` and record the blob with `decode_error` set. Also fix the
neighbouring conflation at `:217`, where a panicked task is labelled
"fetched but not stored".

*Effect:* a bundle containing an unframed blob gains a chunk row and an A6 row
that do not exist today. Today's bundles have no such blob, so they stay valid.

### 6.4 Stream the manifest JSON through the gzip encoder

The remaining ~34 MB of §3.6. `serde_json::to_writer(BufWriter<GzEncoder<File>>)`
instead of `to_string_pretty` → `gzip` → `fs::write`.

Band 3 because flate2's deflate stream is not guaranteed identical when the
input is fed in different chunk sizes, and `manifests/*.json.gz` bytes are
recorded in `checksums.json`. **Try it in band 1 with a test that both paths
produce identical gzip bytes for a fixed input; if that test fails, it lands
here** and the changed digests are the honest cost.

### 6.5 Unify the skip-set and wire `is_fatal`

Two sources of truth for the same fact:

- `emit/selfcheck.rs:316-324` `Selfcheck::skipped()` **derives** the missing ids
  from `WORDING` — and has **no production caller** (only `:386`).
- `emit/selfcheck.rs:266-270` `Selfcheck::build(.., skipped: &[&str])` **takes**
  them as a parameter.
- `run.rs:799-803` hard-codes `vec!["A6","A7","A8","A9","A10","A11","A12","A19"]`.

**First**: assert the hard-coded list equals `Selfcheck::skipped()`. If they
already agree, unifying is band 1. If they disagree, the recorded set changes
and the `selfcheck.json` rows change → band 3, and the spec (`docs/00-spec.md
§7.2`) may need a look to see which is right.

`is_fatal` (`emit/selfcheck.rs:176-197`) is documented as deciding whether a run
ends and is called from nowhere; `run.rs:702-712` makes that decision by hand.
Wiring it changes which failures terminate a capture.

### 6.6 Consider, and probably reject

- **One-pass `md5 + sha256` over blobs, with `spawn_blocking`** (§5.4) — the
  digests are unchanged, so this is band 2 in effect. Listed here only because it
  is the one change that could plausibly alter a recorded value if the two
  digests are ever computed over different inputs. Gate on a blob whose
  `compressed_md5` and `sha256` disagree.
- **Streaming chunk blobs straight to disk** with the `stream` feature
  (currently dead config, §2.1) — hash-incrementally-while-writing. It deletes
  both the 1.13 MB buffer *and* the `tmp/` round-trip, but the digests must come
  out identical, and it changes the failure behaviour of a partial fetch. Band 2
  with a strict gate, or band 3 if the tmp-file discipline shows up in the
  artifact (it does not today).
- **`ApiError` is the wrong type to carry an exit code.** `ApiError::Shape`
  covers ~55 conditions with nothing to do with the network — local invariants
  (`hyp/mod.rs:157,168,279`), writer errors (`run.rs:467,480,499`), budget
  ceilings (`run.rs:421,448`), assembly (`run.rs:673`). So `ApiError::exit_code()`
  (`http/mod.rs:124-127`) claims "the code **every** API failure maps to" and is
  called **only from tests**; production picks the code at the call site
  (`run.rs:1338` → Network, `run.rs:1356` → AssertionFailed) for the same type.
  **Fixing the doc is band 1. Splitting the enum is a design change → band 4.**

---

## 7. Band 4 — Out of bounds

Not in this plan. Listed so they are not mistaken for cleanup.

| | why |
|---|---|
| Renaming, renumbering or removing **A1-A20** | Spec'd at `docs/00-spec.md:393-444`. A18/A20 are `verify`-only; seven are `REDERIVED` and cross-checked against the capture's own recording (`docs/00-spec.md:459-473`). The ids are a published contract. |
| Changing `SCHEMA_VERSION` or any document's required fields | `docs/01-artifact-schema.md`; a reader must reject an unknown value rather than guess (`lib.rs:71-76`). |
| Anything that breaks `girpr`'s `tests/artifact_bridge.rs` | 75 KB, `#[ignore]`d, run by hand. It pins matrix↔patchmap agreement in **both** directions (`:1531-1572`), patchmap arithmetic (`:1069-1144`), `expected/` ground truth, and that every `download_bytes` matches its patchmap to the byte. **This is the consumer contract.** |
| Any dependency on `girpr`, or a `Command` against its binary | The independence invariant (`docs/00-spec.md §2`; `Cargo.toml:7-8`). A grep over `tools/capture/src/` must keep returning doc comments only. |
| Turning on `clippy::pedantic` crate-wide | Clippy is **clean** today, which is worth preserving. Enabling pedantic in one commit rewrites the whole crate and would drown the band-0/1 work. If wanted, adopt lint groups one at a time. |
| Release-profile tuning (LTO, `codegen-units`, `panic=abort`) | Untested for a tool whose output *is* a fixture, and the perf headroom is in the algorithms (§3), not in codegen. **Measure `verify`/`capture` wall time first** — see §8. |
| `rustfmt`/`clippy --fix` sweeps over untouched files | Same reason. Band 0 is already ~200 lines of deliberate removal; a reformat makes the band-1 diff unreviewable. |

---

## 8. Measurements to take before optimising further

`verify` is already measured (**7.92 s**, §0). `capture` is **not** — it needs
network and a ~2 min run, which is why the capture-side performance claims in
§3.1, §3.2, §3.3 and §5.3 are still argued from code reading. Take these before
acting on them:

```powershell
# capture-side baseline, release build. Pin the pair: an unpinned capture
# follows whatever upstream has moved to and measures nothing.
Measure-Command { .\target\release\gircap.exe capture --out $env:TEMP\g `
    --biz hk4e_global --old-tag 7.0.0 --new-tag 7.1.0 }
# metadata-only, to isolate the serial-RTT cost of capture_all (§5.3)
Measure-Command { .\target\release\gircap.exe capture --out $env:TEMP\g `
    --hyp-base $MOCK --sophon-base $MOCK --biz hk4e_global --no-chunk-blobs }
# jobs scaling, to test the RequestLog-mutex ceiling
1..8 | ForEach-Object { Measure-Command { ... --jobs $_ } }
Measure-Command { .\target\release\gircap.exe list --root artifacts }
```

Then re-run after each band and record the numbers in §0 so the next reader can
see what moved. Three specific things to watch:

- **`verify` re-reads and re-decompresses every blob.** 7.92 s over 135 MB is
  ~17 MB/s of *useful* work; §3.4 says ~28 MB of that is read and decoded twice.
  If `verify` is I/O-bound rather than CPU-bound, §3.4 is worth more than §3.2;
  if CPU-bound, §5.4 (one-pass digests + `spawn_blocking`) is. A 5-second `verify`
  on a warm page cache points at hashing and decompressing, not at the disk.
- **`--jobs 8` vs `--jobs 1`.** If scaling is flat, the `RequestLog` mutex
  (§3.2) is confirmed as the ceiling and nothing else in the concurrency work
  matters until it is fixed. This is the single most informative measurement in
  the list, because §3.2 predicts it will be flat.
- **the `4 → 2` digest-pass collapse** (§3.1), measured on the real 135 MiB
  bundle. Four full-tree md5+sha256 passes over 135 MB is ~540 MB of hashing
  that should be ~270 MB; at release-build md5+sha256 throughput that is on the
  order of a second or two, and it is the most predictable win in band 1.

Peak RSS during `capture` is worth recording too — the §3.6 57 MB and §5.3
concurrency claims are all peak-memory claims, and RSS is the only number that
settles them.

---

## 9. Suggested sequencing

Dependencies, cheapest first. Each arrow means "the next item wants the
previous one".

```
 0. Band-0 sweep (one PR)                    no deps
 1. §4 byte-diff harness                      no deps — DO THIS SECOND
 2. §3.8 test suite: dedup, OnceLock, PIDs    0, 1   (makes the gate cheap)
 3. §3.1 four digest passes -> two            1
 4. §3.2 diff/manifest hot paths              1
 5. §3.3 select/assemble hot paths            1, 3
 6. §3.5 the six write helpers + write_err    0, 1   (tools the rest use)
 7. §3.4 verify read/decode collapse          1, 4
 8. §3.6 memory: FullDecode borrow, tsv       1
 9. §5.4 hash::DualDigest + spawn_blocking    4, 7
10. §5.3 concurrency (map_concurrently)       4, 7   (needs §3.2's mutex fix)
11. §5.1/§5.2 hardening (panics, i64)         1      (each needs its own test)
12. §3.7 split capture()                     1-11   (alone, last)
--- above this line: bundle bytes must not move ---
13. §6.1 stored_as for api rows               1, 4
14. §6.2 real request headers                 0      (test against live API first)
15. §6.3 unframed blob recorded               0
16. §6.5 skip-set / is_fatal                  0      (check agreement first)
17. §6.4 streaming gzip                       8, 13   (only if the byte test fails)
--- above this line: re-capture one reference bundle ---
```

Steps 0-12 remove roughly **550 lines** outright (~200 band-0 dead code and
duplication, ~215 of write-path duplication in §3.5, ~130 in the test suite) and
**relocate ~600 more** out of `run.rs` into the modules that own them. They
capture the large majority of the achievable speedup, with **zero change to what
a bundle says**. Steps 13-17 are worth doing, but each one costs a re-capture
(~2 min, 129 MiB) and a careful read of `docs/01-artifact-schema.md` to confirm
the field is additive.

Step 2 is early on purpose: it is behaviour-free, it is the difference between a
70 s and a 151 s gate, and there are twelve gate cycles ahead of it.

**One item per commit, each with its own gate result in the message.** The
whole plan is only trustworthy if each step is independently revertable — that
is what makes a high-risk band-3 change acceptable at all.

---

---

## 10. Looked at and deliberately left alone

So nobody re-audits them.

| | |
|---|---|
| **`biz.rs` (530 lines)** | all `const fn` tables and `match` over a 2-3 variant enum: `Biz::resolve` (`:113-143`), `Host::hyp_base`/`sophon_base` (`:40-55`), `HypEndpoint::path` (`:77-84`). **Zero per-call table construction, zero repeated lookups.** The only note is `Endpoints::owned`'s `Box::leak` (§5.5). Needs nothing. |
| **`exit.rs` (150 lines)** | three `const fn` tables over a 5-variant enum, a `const ALL` pinned by a test, and a redundant `From<ExitCode> for i32`. Cleanest file in the crate. |
| **`select/budget.rs` (239 lines)** | **zero magic numbers.** `saturating_add` (`:63`), `charge` saturates (`:72`), a rejected candidate is never charged (`:120-126`), and the tie-break is documented as the caller's (`:109-113`). Use it as the model for §2.2. |
| **`report.rs` serialization** | there is **no `serde_json::Value` construction** — `to_json_line` (`:310-330`) hand-rolls the object to keep field order, which is the right call (`preserve_order` would be a workspace-wide feature unification hazard). `Val::text` allocations are real but the data plane is 1 record per command. Only the missing `BufWriter` matters, and only for `list` (§3.2). |
| **`assemble/ranges.rs` merge** | **already a single forward pass** (`:136-191`); there is no sort-then-merge to improve. `Layout::sorted` (`:199`) is the sort, and it is dead (§2.1). |
| **`assemble/mod.rs` byte movement** | **no per-byte loops anywhere.** `copy_from_slice` per chunk (`:529`) and a calloc-backed `vec![0u8; size]` (`:512`) are already optimal. The waste is the *number of buffers* (§5.5), not the copying. |
| **`assemble_side`'s three-way `buffer` / `Layout::of` / `zip`** | correct. Only `assemble_side` ≡ `materialize::rebuild` is duplication (§3.5). |
| **`prost` decode** | sound. `manifest/mod.rs:436-441` decodes the wrapper and immediately moves out `.chuncks`, so the outer struct is not retained. The damage from per-chunk `String` allocation is confined to the three serialisation boundaries in `dump.rs` (§3.6), not the decoder. |
| **Chunk-store keying** | `BTreeMap<String, Want>` keyed by `chunk.id` gives id-uniqueness *and* the deterministic order `docs/02-plan.md` demands (`chunks.rs:332-334`, satisfied by `into_values()` at `:405`). Shared chunks correctly merge into one `Want` with multiple `Source`s. Only `ChunkIndex::get` is wrong (§3.4). |
| **The "re-derive, don't trust" philosophy** | `docs/00-spec.md §8`. Correct, and the reason `verify` is trustworthy. §3.4 is not an argument against it — it is against re-deriving the same blob twice *within one command*, after `check_blobs` already had it in hand. |
| **`scan-and-verify` independence (A16)** | keep pass 3 independent of pass 2. That is what makes A16 an assertion rather than a tautology (`emit/index.rs:198-199`). §3.1 collapses 4 passes to 2, not 2 to 1. |

---

## 11. Completion audit

Checked against the tree at `c5c20e5`, section by section, rather than from the
commit log. Every row is a measurement or a grep, not a recollection.

| § | requirement | evidence |
|---|---|---|
| 2.1-2.4 | band-0 sweep | no dead code left; `src/` grew by 10 files that are the §3.7 split, and `run.rs` 1,981 → 860 |
| 3.1-3.6 | output-identical | 91 of 97 byte-identical on every real capture; `game/ 9/9 expected/ 7/7 patchmaps/ 8/8 chunks/ 39/39 diff/ 5/5 manifests/ 16/16` |
| 3.7 | split `capture()` | 1,981 → 860 lines in `run.rs`; it is no longer the largest file in `src/` |
| 3.8 | cheap enough to gate every commit | suite **151 s → 24.4 s** with 119 more tests; 0 `#[ignore]`, 0 sleeps, 0 non-`.rs` fixtures, all captures on `127.0.0.1:0` |
| 4 | the gate itself | `tests/bundle_diff.rs`, 8 tests, two mock captures compared byte-wise |
| 5.1-5.4 | band-2 hardening | landed, one commit each |
| 5.5 | the 16-row table | closed — see the status table in §5.5 |
| 6.1-6.6 | band-3 changes | landed; the re-capture §6 owes is promoted (`20261017T123335Z`) |
| 7 | out of bounds | `SCHEMA_VERSION` is `1`; 0 non-comment mentions of `girpr` in `src/`; no `pedantic`, no `[profile]` tuning, no `rustfmt.toml`; A1-A20 unchanged (A99 is a pre-existing test fixture for an undescribed assertion, present in the baseline) |
| 8 | measurements | taken and recorded in §0 |
| 10 | left alone | `exit.rs` and `select/budget.rs` untouched; `biz.rs` gained only the `Box::leak` note §10 asked for; `report.rs`'s hand-rolled `to_json_line` and absence of `serde_json::Value` intact |

Full gates at `c5c20e5`:

    cargo clippy -p gircap --all-targets    clean
    cargo test -p gircap                     571 passed, 0 failed
    cargo test --release -p gircap --test bundle_diff        8 passed
    cargo test --release -p gircap --test schema            14 passed
    cargo test --release -p gircap --test capture_layout    24 passed
    gircap verify, each of the three bundles under artifacts/
                                            exit 0, checks_total=90
                                            checks_passed=90 findings=0

**What is not claimed.** §3.2's flat-scaling prediction is *refuted but untested
at scale* — the fixture fetches 38 blobs and cannot express the contention. The
residual gate gap in §5.5 (`bundle_diff` cannot see a drift both its own runs
share) is real and is not closed by anything checked in.

