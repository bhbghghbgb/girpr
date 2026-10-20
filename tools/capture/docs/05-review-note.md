# Review note — `capture/re-spec` (rework of the `@tools/capture` artifact bundle)

> **This file is temporary, and it is deleted once the fixes land.**
>
> **Do not refer to it.** Not from the code, not from the documents, not from a
> comment, not from a commit message, not from a doc's "see also". It is a
> hand-off between two passes of the same work, not a project document, and it
> has no place in the shipped spec. Nothing that outlives this file should
> point at it, and nothing should be written that depends on it being there.
>
> If a finding here turns out to need a permanent home, it moves into the
> document that owns the thing it is about — not into a reference to this file.

**Branch:** `capture/re-spec` (HEAD `516d710`) · **Base:** `main` (`f9e6f6a`)
**Scope:** the re-spec of the `gircap` artifact bundle after `girpr`'s repair
spec changed, plus the `girpr` bridge rewrite it forced.
**Reference bundle:** `artifacts/hk4e_global/7.0.0__7.1.0/20261020T100659Z`

This note lists concrete defects with locations and the exact values to use.
Everything marked **[P1]** should be fixed before the branch lands; **[P2]** is a
real defect worth fixing here; **[P3]** is a sweep.

---

## 0. Do not touch these — they are verified correct

So the fixes below do not churn good work:

- The eleven class ids, the admission order, and all eleven `exercises()`
  strings (`tools/capture/src/select/classes.rs:170-251`) match
  `tools/capture/docs/00-spec.md:243-253` verbatim, and the doc's predicates
  match `Class::matches` exactly.
- `Repair` / `SummaryExpectations` field names and serde spellings
  (`tools/capture/src/assemble/mod.rs:275-330`) match
  `tools/capture/docs/01-artifact-schema.md` §13.0 / §13.0a exactly.
- The interop arithmetic holds on all nine captures: source choice (first old
  chunk per identity), the trim's two outcomes, the salvage direction and
  tie-break, `est_download_bytes`, `damaged_chunks`, `kept_bytes`,
  `write_bytes`, `ram_bytes`, `download_bytes` all agree between
  `src/repair/{scan,file,reuse}.rs` and the patchmaps.
- The bundle itself is what the plan asked for: nine captures,
  `trim_deferred` → `covered_by` `moved_ascending`, `moved_mixed` →
  `over_budget` with its reason. README's size figures are correct
  (measured: 254 652 177 B total ≈ 243 MiB; `expected/` 79.4, `game/` 75.8,
  `chunks/` 44.3, `manifests/` 41.4 MiB).
- `docs/06-artifact-bridge-spec.md` §7.3's "not covered" reasons for the
  read-only fallback, the mismatching-chunk second pass and the zero-fill are
  correct and well argued — keep them.

---

## 1. [P1] The class set is not exclusive with `identical`, and it surfaces as a wrong published number

**Where:** `tools/capture/src/select/classes.rs:286-316`.

`Class::Identical` matches `same_md5: true`. The three direction classes and
the three partial classes do **not** exclude it — only
`ZeroReuseSingle`/`ZeroReuseMulti` carry `same_md5: false` (`:267-281`).

The live pool contains one such path. From the bundle's own
`diff/candidates.json`:

```
{"classes":["identical","moved_descending"], "matching_field":"game",
 "path":"GenshinImpact_Data/StreamingAssets/AudioAssets/Music4.pck",
 "nchunks":336, "reused":336, "moved":4, "moved_up":4, "moved_down":0,
 "old_size":398535566, "new_size":398535566, "same_md5":true, "rank":2835}
```

The whole-file MD5 matches on both sides, yet the chunking was re-tiled, so the
file satisfies `moved_descending` too. Three consequences:

1. **The published population is wrong.**
   `tools/capture/docs/00-spec.md:277` says `moved_descending` **139**;
   the bundle's own `diff/matrix.json` says `candidates_considered: 140`
   (and `diff/candidates.json` has 140 such rows: 139 + this one).
   Every other class agrees (1733/121/491/29/321/3/56/282/84/11).
2. **The instrument does not reproduce the crate's rules.**
   `tools/capture/tools/case_coverage.py:227` short-circuits on MD5 equality,
   so it never scores such a path — which is why it says 139 while the Rust
   says 140. The plan (§8) required the instrument to reproduce the crate's
   rules exactly.
3. **Latent A19 trip-wire.** `tools/capture/src/emit/selfcheck.rs:519` uses
   `should_differ = !a.primary_class.is_identical()`. A file with equal MD5s
   admitted for a content class would fail A19 with *"is unchanged but its
   class says it should differ"* → capture exits `3`. Latent only because
   `Music4.pck` costs 785 MB (rank 2835) and can never be the cheapest
   `moved_descending` candidate — but the class set is supposed to be a
   partition, and a class is "a statement about the content" while a skipped
   file makes no content decision at all.

**Fix:** add `same_md5: false` to `KeptInPlace`, `TrimEager`, `TrimDeferred`,
`MovedAscending`, `MovedDescending`, `MovedMixed` — matching what the
zero-reuse classes already do. That makes the numbers 139 in the crate, in the
instrument and in the doc, and removes the A19 trip-wire.

**Consequence to plan for:** the bundle's `diff/matrix.json` and
`diff/candidates.json` then disagree with the crate (140 vs 139) and must be
regenerated. The nine patchmaps are unaffected (`Music4.pck` is not captured),
but `diff/` is part of the artifact, so this is a re-capture step — or, if you
prefer the smaller blast radius, keep the Rust as-is and fix only the doc
(140) and the instrument. **Decide explicitly and say which in the commit
message**; the recommendation is the predicate fix, because it also fixes
consequences 2 and 3.

---

## 2. [P1] `docs/06-artifact-bridge-spec.md:606` claims a bridge case that does not exist

> | `--max-ram-bytes <small>` | **the same case, with a budget that cannot hold the payload** | a file that cannot get its reservation degrades to the ordered tier instead of failing |

There is no such case. The only `--max-ram-bytes` values the bridge ever passes
are `1 GiB` (`tests/artifact_bridge.rs:1033`) and `0` (`:1909`).

Those are **different branches** in girpr (`src/repair/file.rs:627-649`):

```rust
Some(0) => (Tier::Ordered, None),        // "stage nothing at all" — a MODE
_ if payload == 0 => (Tier::Staged, None),
_ => if ctx.summary.reserve_ram_bytes(payload) { Staged }
     else { /* "the moving bytes do not fit the RAM budget -> ordered tier" */ Ordered }
```

The "reservation refused → degrade to the ordered tier" branch
(`src/repair/file.rs:641-647`) is **never executed by any bridge case**. The
plan required it; `docs/06` asserts it is covered.

**Fix (preferred):** add the case. `--max-ram-bytes 0` and a *refused*
reservation look identical from the outside, so the observable is the log line
`src/repair/file.rs:641-646` — `"the moving bytes do not fit the RAM budget ->
ordered tier"`. The bridge already writes a TRACE log file
(`base_args(..., &scratch.log_file())`, and `downloaded_chunks(log)` at
`tests/artifact_bridge.rs:866` already parses it), so the new test can assert
that line is present, that the run exits `0`, and that the file still hashes to
`expectations.result_md5`. `moved_ascending` (payload 5 892 269) is the right
subject; a budget of e.g. `1024` refuses the reservation.

**Fix (fallback):** if you decide not to add the case, change the `docs/06:606`
row to say **not covered**, and add it to the "Why three of them are not covered
here" paragraph — which currently says *three* are not covered while in fact
**four** are (this row plus the three already listed).

---

## 3. [P1] The trim's outcome — the branch's headline change — is asserted nowhere

**Where:** `tests/artifact_bridge/server.rs:447-451` parses
`repair.trim`, `repair.direction`, `repair.keeps_ascending`,
`repair.keeps_descending`, `repair.growth_bytes` and
`summary.ram_bytes_ordered`; **`tests/artifact_bridge.rs` reads none of them**
(the only `.trim` in the driver is `l.trim()` on a stdout line).

So D5 is the one content decision that has a class and no assertion. Splitting
`partial_size_changed` into `trim_eager` + `trim_deferred` — the whole point of
the class-set change — buys the bundle nothing observable. The eager/deferred
branch shows up only in the log (`src/repair/file.rs:596-609`):

```
"trimming the dead tail"                                     // eager
"a reuse candidate reaches past the target's end: the trim is deferred ..."  // deferred
```

**Fix:** assert the trim, and close the loop with the artifact:

- for `trim_eager`: the log contains `"trimming the dead tail"` and **not** the
  deferred line;
- for `trim_deferred` (covered by `moved_ascending`'s capture — its old size
  11 914 387 > new 11 912 007): the log contains the deferred line and **not**
  `"trimming the dead tail"`;
- in both, cross-check the log against the patchmap's `repair.trim`, so the
  artifact and the tool are shown to agree.

Note the deferred line is `debug`-level; the bridge's log file is always-TRACE,
so it is captured. If you assert on `repair.trim` instead, the artifact and the
tool are still not shown to agree — do both.

While you are there, `docs/06-artifact-bridge-spec.md:557-558` ("`ram_bytes` is
`0`, **against the patchmap's `ram_bytes_ordered`**") and `:584-585` ("the
bridge asserts that the two agree") describe cross-checks the code does not
perform — only 4 of the 11 `repair` fields are compared today. Either make
them true or soften the wording.

---

## 4. [P2] A17's change altered `gircap capture`'s exit-code contract; it is not documented where the contract lives

**Where:** `tools/capture/src/select/mod.rs:137-162` (`MatrixStatus::is_acceptable`),
`tools/capture/src/emit/selfcheck.rs:177-180` and `:759-770`,
`tools/capture/src/emit/index.rs:1181-1205`.

A17 now accepts `over_budget`. Before this branch, a class the budget refused
failed A17 → the capture exited `3` ("the artifact is **not** trustworthy").
Now it exits `0`.

The plan (§3/C4) said *"A17 (every class is `captured` or `unavailable`) is
unchanged"*, but §2.1/§4 required `moved_mixed` to be `over_budget` in a
**successful** capture — the plan contradicted itself, and this is the
resolution. The reasoning in the code is good; what is missing is the
documentation.

**Fixes:**

- `tools/capture/src/emit/closeout.rs:171` and
  `tools/capture/src/verify/mod.rs:407` still say the A17 row's expected text
  is `"every class captured | covered_by | unavailable"` while the verdict
  accepts all four. Make the row text say
  `captured | covered_by | over_budget | unavailable` so the row does not
  contradict itself.
- Add a sentence to `tools/capture/docs/00-spec.md` §7 *Exit codes* and to
  `tools/capture/README.md`: a class the budget refused is now recorded as
  `over_budget` with its reason and the run exits `0`; `--require-class` is
  the lever that turns it into exit `4`. The information is not lost
  (`SUMMARY captures_over_budget=`, the matrix row's reason) but the guard rail
  is: a capture that silently dropped most of its classes now exits `0`.
- If you want the old guard rail back, the alternative is to keep A17 strict and
  let `over_budget` be an accepted *non-fatal* row that still fails A17 only
  when the class was `--require-class`d. That is a bigger change; documenting
  the current behaviour is the minimum.

---

## 5. [P2] `gircap` compares chunk identities case-sensitively; `girpr` compares them case-insensitively

**gircap (case-sensitive):** `tools/capture/src/manifest/diff.rs:71`
(`ChunkKey = (&str, i64)` in a `HashMap`),
`tools/capture/src/assemble/mod.rs:965-971` (`old_by_identity`) and
`:1287-1296` (`identity_sources`).

**girpr (case-insensitive):** `src/repair/reuse.rs:162`
(`md5.eq_ignore_ascii_case(&c.uncompressed_md5)`),
`src/repair/plan/mod.rs:161`, `src/repair/file.rs:1123`.

This is the same comparison in both tools: in the bridge, girpr's "local"
manifest is the served 7.0.0 manifest, so girpr compares the old manifest's
md5s against the new manifest's md5s — exactly the pair `gircap` compares.
On a pair that spells the same md5 in different cases, `gircap` would call a
chunk *fetched* where `girpr` calls it *reused*, and the bundle would publish
the wrong numbers.

I checked the live pair: **0** disagreements, so nothing is wrong with the
current bundle. But the two tools disagree **by construction**, and the
instrument (`tools/capture/tools/case_coverage.py:91`) already uses
`.lower()`.

**Fix:** normalise the md5 when it enters the identity key (lower-case it at
the point the key is built) in the three `gircap` places above, and keep
`girpr` as it is. Add a test with a mixed-case md5 pair.

---

## 6. [P2] `case_coverage.py`'s "outcome" section does not reproduce the artifact's `repair` block

`tools/capture/tools/case_coverage.py:337-341` counts a **kept** chunk toward
`asc_keeps`/`desc_keeps`; the artifact does not
(`tools/capture/src/assemble/mod.rs`, `repair_of` `continue`s on a kept chunk).
And `:365` always prints a direction, where the artifact prints `n/a` for the
0–0 tie. The tool disagrees with the artifact for **six of the nine**
captures.

The artifact is the one that matches `girpr`: `src/repair/reuse.rs:92-112`
(`salvage_order`) runs over the **damaged** chunks only, so kept chunks are not
counted, and a 0–0 tie is "ascending" but there is nothing to order.

**Fix:** align the tool with the artifact (skip kept chunks in the salvage
counts; print `n/a` when both counts are 0), and — while you are there — add a
gate so this cannot drift again: a check that runs
`tools/capture/tools/case_coverage.py <bundle>` and diffs its outcome table
against the bundle's own `patchmaps/*.json` `repair` blocks. That single gate
would have caught §1's population drift as well.

---

## 7. [P2] Three mangled lines — a statement joined onto a signature line

Three lines in the branch are botched by a botched edit: a statement sits on
the same line as the signature it belongs to.

- `tools/capture/src/assemble/mod.rs:1137` — `) -> Result<PatchMap, String> {    let side = |r: &FileResult| Side {`
- `tools/capture/src/assemble/mod.rs:2429` — `fn decisions_serialise_the_way_the_schema_doc_writes_them() {        assert_eq!(`
- `src/repair/file.rs:378` — `pub(super) fn resolve_mode(...) -> Mode {    if requested == RepairMode::Temp {`

**Fix:** split those three lines by hand, so the statement starts on its own
line under its signature.

**Do not run `cargo fmt` on this branch, and do not reformat anything else.**
`main` already fails `cargo fmt --all -- --check` (193 diffs), so formatting is
not a gate here — and running it turns a three-line fix into a 26-file diff
that then has to be read line by line. Hand-edit the three lines above and
leave the rest of the branch's formatting exactly as it is.

---

## 8. [P3] The docs around the new text still carry the **old** bundle's figures

The new material (§4.1's class table, §13.0, §13.0a) is accurate. The
surrounding text is not — it was written for the 8-capture bundle
`20261017T123335Z`. Please sweep these.

**`tools/capture/docs/00-spec.md`**

| line | wrong | right |
|---|---|---|
| 277 | `moved_descending` **139** | see §1 — 139 after the predicate fix, 140 as the artifact stands |
| 301 | `zero_reuse_single` — "— (grows)" | "— (same size)"; the artifact says `growth_bytes: 0` (old = new = 8) |
| 309-312 | "`moved_ascending` is the only capture that exercises the **moving-bytes** path at all, the only one that reaches the ordered tier's rule" | its own table shows `moved_descending` moves 27 chunks / 38 150 367 B with `direction: descending`; only "the only one that reaches the **deferred** trim" is true |
| 220 | "§6 of `girpr`'s bridge spec" | §7.3 (*The flag axes and the fault injections*) — and §7.3 exists only on this branch |
| 374 | the pre-existing bold pseudo-heading "**Selection pool = `game` ∪ the selected audio fields.**" now reads as part of "#### What the bundle cannot cover" because the new subsections were inserted above it | move it back above the new subsections |
| 391 | `record covered_by=<that capture>` | the code puts the covering capture's **id** in `capture_id` and its **path** in `covered_by` (`select/mod.rs:709-717`; artifact: `capture_id: "moved_ascending"`, `covered_by: "…/09801749.blk"`) |
| 635, 766 | "skips A6–A12 and A19" | also A21–A23 — `selfcheck::SKIPPED_WITHOUT_CHUNK_BLOBS` is `["A6".."A12","A19","A21","A22","A23"]` (`emit/selfcheck.rs:672-674`) |
| 681-687 | "the first seven are statements about the API bodies, the manifests and the diff" | the first **eight** (A1-A5, A9, A10, A12); the list before "or A21-A23" holds eleven ids |
| 904 | "a single run over all eight sums to the same total" | nine |
| 966 | "the other seven" | the other eight (`game/` holds eight capture paths; `added` is absent) |
| 1046-1047 | "The six that carry `totals` sum to **2 373 385 B**; adding `added` makes it **2 487 742 B**" | **seven** captures carry `totals` (4 853 560 B) and the nine total **4 967 917 B** — the same document says 4 967 917 B at line 1020 |

**`tools/capture/docs/01-artifact-schema.md`** — the worked example is the old
bundle throughout:

| line | wrong | right |
|---|---|---|
| 23-25 | "97 files, 8 captures, 38 chunk blobs, 56 request rows, 116 selfcheck rows" | **133 files, 9 captures, 71 chunk blobs, 93 request rows, 159 selfcheck rows** |
| 97 | capture id `20261017T123335Z` | `20261020T100659Z` |
| 119-124 | `files_captured: 8, chunks_blobs: 38, bytes_downloaded: 47592933, bytes_on_disk: 208239065` | **9, 71, 66 287 734, 327 720 240** |
| 161-166 | "`file_count: 96` for a 97-file bundle"; "≈199 MB against ≈129 MB on disk" | `file_count: 132` for a 133-file bundle; **≈328 MB against ≈243 MiB** |
| 190-191 | "56 rows - 6 `api`, 4 `manifest`, 38 `chunk`, 8 `probe`" | **93 rows - 6 api, 4 manifest, 75 chunk, 8 probe** |
| 550 | "38 blobs for the reference capture" | **71** |
| 681, 693 | `"write_bytes": 713245` | **714245** (690 719 + 23 526) |
| 906, 918-921 | `"total": 116`; "A1 6, A2 2, A3 4, A4 8, A6 38, A7 7, A8 7, A9 6, A10 4, A11 14, A12 7, A13 1, A14 1, A15 8, A16 1, A17 1, A19 1" | **159**: A1 6, A2 2, A3 4, A4 8, A6 71, A7 8, A8 8, A9 7, A10 5, A11 16, A12 8, A13 1, A14 1, A15 8, A16 1, A17 1, A19 1, **A21 1, A22 1, A23 1** |
| 126-134 | the example matrix rows | `"covered_by": "moved_ascending"` is a **path** in the real artifact; the `reason` strings differ ("cheapest candidate already captured for another class" / "the cheapest candidate is captured for another class"); `moved_mixed`'s real row is `reason: "the cheapest candidate costs 243016539 B and the budget could not fund it"`, `candidates_considered: 3` |
| 739, 762, 797-807 | "The block is absent for a capture with no new side" | the writer emits `"repair": null` and `"summary": null` — and §13.1's own `removed` example uses `"new": null, "plan": null, "totals": null`, so the document contradicts its own convention |
| 7 vs 679-695 | "keys in the order written here" | the example puts `repair` before `expectations`; `PatchMap`'s field order (`assemble/mod.rs:132-160`) and the artifact put `expectations` first |
| 932 | "skips A6–A12 and A19" | also A21–A23 |
| §13.0a | the title says "the `SUMMARY` keys, per flag" | girpr's SUMMARY has **one** RAM key, `ram_bytes` (`src/report.rs:412, 483-485`); `ram_bytes_ordered` / `ram_bytes_staged` exist only in the capture's `SummaryExpectations`. A consumer cannot compare them to a key that is never emitted |

**Other files**

- `tools/capture/docs/02-plan.md:93` — "the nine class predicates" → eleven.
  `:244` — "a default capture is ~130 MB" → ≈243 MiB (contradicts the README).
- `tools/capture/docs/03-design-notes.md:18` — "~130 MiB" → ≈243 MiB.
  `:46-48` — "~130 MiB on disk: 97 files, 4 manifests, 38 stored chunks, 56
  request rows" → **243 MiB, 133 files, 71 stored chunks, 93 request rows**;
  "8 captures, 7 in `game/` and 7 in `expected/`" → **9 captures, 8 and 8**;
  "assembled bytes per capture | 62 216 590 B" → **162 668 990 B**.
- `tools/capture/docs/04-bundle-differ.md:182-192` — the mock column is the
  old 5-capture mock ("58 files", "patchmaps 5", "chunk blobs 10"); this branch
  grew the mock (`tests/common/mod.rs:215-236` adds `eager.blk`,
  `deferred.blk`, `grow.blk`) so it now produces **8 patchmaps** and the
  58-document arithmetic no longer holds. The reference column is the old
  bundle too.
- `docs/06-artifact-bridge-spec.md:98` — "~2 minutes" → the suite takes
  **~10 minutes** (592 s for 25 cases). `:318-320` — "the `manifests/`
  directory is ~40 MB of the ~130 MB bundle; across eight runs" → the bundle
  is **243 MiB** and the suite is 25 cases. `:446` — "the other seven" → the
  other eight. `:447` — "~32 MB per run" → the `game/` fixture is **76 MB**.
  `:480` — assertion 5 says "the **nine** capture paths are pairwise distinct";
  the code asserts distinctness only, and "nine" is a property of this bundle.
  `:484` — assertion 9 says the discard mode downloads "the whole of it"; for
  `identical` the discard mode **skips** the file (the skip check precedes the
  delete, `src/repair/file.rs:549-552` vs `562-570`), so nothing is
  downloaded.

---

## 9. [P3] Stale class ids in the test fixtures

- `tools/capture/tests/offline_bundle.rs:37` — `const CAPTURE: &str =
  "partial_same_offset";` while the same file's patchmap carries
  `classes: vec![Class::KeptInPlace]` (`:235`). The fixture id contradicts its
  own class list.
- `tools/capture/tests/measured_fixture.rs:62, 72, 75, 194, 390, 916, 1114,
  1360` — `partial_same_offset` / `partial_moved_offset` /
  `full_reuse_reordered` in fixture ids and comments. Lines 73-75 assert a fact
  about the spec ("`docs/00-spec.md` §4.1 records that class as having no
  candidate") that is now false — `full_reuse_reordered` was removed from the
  class set.
- `tools/capture/tools/case_coverage.py:63-73, 166-173` keeps the old ids
  **on purpose** (the script measures the old set too). Leave it, but say so
  in `tools/capture/tools/README.md`, which currently does not mention that the
  script carries both class sets.

---

## 10. [P3] Smaller items

- `tools/capture/src/emit/selfcheck.rs:955` — the comment reads "Twenty-one
  rather than **twenty** is the point of asserting the length"; the original
  said "Eighteen rather than seventeen". It should be "twenty-one rather than
  seventeen" (17 spec ids + A19 = 18 before this branch).
- `tools/capture/docs/01-artifact-schema.md` §13.0 — "A tie goes to ascending,
  as it does in the tool" — but the artifact reports `direction: "n/a"` for the
  0–0 tie (six of nine captures), which is the tie the sentence is about;
  `girpr`'s `salvage_order` would say "ascending" there.
- `tools/capture/src/lib.rs:81` — `SCHEMA_VERSION` stayed `1` although the
  class-id rename renames files under `patchmaps/`. Defensible under the
  document's own rule ("bumped only for a breaking change to the bundle's byte
  layout"), but the rename *is* breaking for a consumer that hard-codes an id.
  Either bump it or record the decision.
- `docs/06-artifact-bridge-spec.md:608` — the row is titled "the resume paths"
  (plural) but only the in-place half is covered; the temp resume-by-length
  path (`src/repair/file.rs:1116-1119`) is not exercised by any case.

---

## 11. Gates to re-run before handing back

```powershell
cargo test -p gircap
cargo test -p girpr
cargo run -p gircap -- verify --artifact artifacts\hk4e_global\7.0.0__7.1.0\20261020T100659Z
#   -> 130/130, exit 0; selfcheck 159/159, pass
cargo test -p girpr --test artifact_bridge -- --ignored --nocapture
#   -> 25 passed, 0 failed   (~10 min)
```

Formatting is **not** a gate. `main` already fails `cargo fmt --all -- --check`,
and reformatting this branch produces a diff far larger than the change it
accompanies. Hand-edit the three mangled lines in §7 and leave the rest of the
branch's formatting alone.

Current numbers to beat: gircap 481 lib + 8 + 29 + 19 + 8 + 28 + 9 + 15;
girpr 137 lib + 7 cli_contract + 66 repair_offline; bridge 25.

**Careful with a stale build.** During this review a stale
`target/debug/deps/artifact_bridge-*.exe` was served by cargo and reported
"22 tests" for a suite that has 25 — the three flag-axis cases
(`the_ordered_tier_downloads_exactly_the_patchmap_total`,
`the_discard_mode_downloads_the_whole_file_and_still_produces_it`,
`the_in_place_path_re_verifies_and_continues`) were missing from the binary.
If a count looks off, delete the test binary and rebuild before believing it.

---

## 12. The plan, for the record

The plan (`docs/05-case-coverage-plan.md` @ `04f4b74`, folded into the docs by
`516d710`) was good work: the D1-D9 case space with "can a class force it?",
the explicit backwards-compatibility decision *with its reason* (a class id **is**
a patchmap's file name), the admission-order insight with the cost of getting
it wrong, `covered_by` designed with its edge case, `moved_mixed` expected to be
unfunded with "the bundle records the truth instead of pretending", the manual
capture/promotion steps kept out of the commit list because `artifacts/` is
gitignored, and the fold commit's "What the plan could not have said" section
recording the three findings the plan could not have predicted.

Four things it got wrong, worth fixing in the record so the next plan does not
repeat them:

1. **It contradicted itself and did not notice.** §3/C4 ("A17 … is unchanged")
   vs §2.1/§4 (`moved_mixed` must be `over_budget` in a *successful* capture).
   The plan should have named the conflict and decided it in advance.
2. **It under-scoped a behaviour change on the `girpr` side.** C5 asked only
   for a deferred-trim log line. What the work actually found is that girpr's
   eager trim was **dead code** — the local-reuse-map rule could never be
   satisfied — and `trim_is_free` was changed to the plan's sources
   (`src/repair/file.rs:973-984`). That changes the shipped repair's peak-disk
   behaviour; it was not in the plan, not budgeted, and is recorded only in the
   fold commit's message. Note `docs/02-repair-chunk-spec.md:248` already said
   *"no reuse candidate has `r + s > size`"* — the implementation had drifted
   from its own spec, so the plan should have listed "audit the trim against
   docs/02" as a step.
3. **C1's gate was not the gate it promised.** It promised "the populations must
   not move" over the live pool; the implemented test is a synthetic
   three-pair fixture, and *nothing* measures the live populations against the
   published table — which is how the 139/140 drift in §1 survived.
4. **§8's "the instrument must reproduce exactly" was never gated.** A test that
   runs `case_coverage.py` and diffs it against the bundle's own patchmaps
   would have caught §1 and §6 at the source.
