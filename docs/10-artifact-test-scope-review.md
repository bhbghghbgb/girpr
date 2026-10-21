# Review note — the artifact test scope (`fix/artifact-tests`)

A hand-off note on the branch `fix/artifact-tests` (`main..HEAD`, 14 commits), the work being
**"restore artifact bundle testing to the cross-tool claim"**. It is written for the agent that did
the work: every item is a concrete, located fix, not a direction.

**This file is temporary.** It is deleted once the fixes land, exactly like
`docs/07-streaming-zst-decode-review.md` and `tools/capture/docs/05-review-note.md` were. Nothing
should refer to it — not the code, not the documents, not the comments. A finding that needs a
permanent home moves into the document that owns the thing it is about.

---

## 0. What I already verified — do not redo it

Everything below was run against the branch as it stands (`7e80951`), on this machine, with the
bundle at `artifacts/hk4e_global/7.0.0__7.1.0/20261021T111044Z`:

| check | result |
|---|---|
| `cargo test --workspace` | **all green** — 488 gircap unit + 147 girpr unit + 113 always-on integration; 0 failed |
| `cargo clippy --workspace --all-targets` | clean |
| `cargo fmt --all --check` | clean |
| `cargo test -p girpr --test artifact_bridge --release -- --ignored --nocapture` | **6 passed; 0 failed** — **15.1 s** (release) |
| `cargo run -p gircap --release -- verify --artifact artifacts/hk4e_global/7.0.0__7.1.0/20261021T111044Z` | **63/63, exit 0**, `result=pass`, `findings=0` |
| `git grep "09-artifact\|artifact-test-scope"` | **empty** — the fold left no pointer behind |

**The cost, measured against the plan's own "before" column:**

| | plan's "before" | now |
|---|---|---|
| bundle on disk | 696 MB | **97.7 MB** |
| `game/` fixture | 213 MiB | **19.9 MB** |
| `#[ignore]`d cases | 26 | **6** |
| `girpr` runs in the ignored suite | 79 | **12** (6 × print-plan + run) |
| scratch copies | 59 (12.25 GiB) | **6**, one file each (~20 MB) |
| ignored suite, release | 233 s | **15.1 s** |
| always-on cases in this area | 0 | **35** (25 machinery + 7 server + 3 gircap consistency) |

**And the mutation check, which is the property the whole plan turns on.** The plan's M2
acceptance criterion was "every one of them fails when the corresponding branch in `src/repair/` is
broken". I broke four branches and ran both suites against each:

| mutation | caught by |
|---|---|
| `salvage_order`'s tie-break `>` → `>=` (`src/repair/reuse.rs:97`) | `a_tie_goes_to_the_ascending_order` **FAILED** |
| `trim_is_free` always `true` (`src/repair/file.rs:1069`) | `a_source_past_the_end_defers_the_trim` **FAILED** |
| the local reuse map forced empty (`src/repair/manifest.rs:50`) | **7** Tier-1 cases FAILED; the bridge's `reuse_moved` **FAILED** on the fetch-set assertion |
| the skip check disabled (`src/repair/file.rs:621`) | Tier-1 `an_intact_file_is_planned_skipped_and_never_touched` **FAILED**; the bridge correctly **did not** fail |

That last row is the strongest evidence the tier split is right: **the bridge is insensitive to a
broken skip branch** (an internal decision) and **sensitive to a broken reuse map** (a patch fact).
The tree was reverted; `git status` is clean.

**Verdict: the work is good and it met the intention.** The plan's §1 states the intent correctly,
and the implementation follows it through all three tiers — `girpr`'s internal machinery is on
crafted, always-on fixtures; the `#[ignore]`d suite says only the cross-tool claim; the harness and
the bundle's own consistency have their own always-on homes; and `gircap` no longer transcribes
`girpr`'s `SUMMARY` into a foreign schema. **Three things must be fixed before it lands** (P1 and
P2 below), and a tail of stale prose in the documents (P3). Nothing here says "redo the change".

---

## 1. P1 — the shipped bundle still publishes the old girpr-internal wording

**Where:** `tools/capture/src/emit/selfcheck.rs:185-196` (the `WORDING` table),
`tools/capture/tests/capture_layout.rs:1010-1012` (the pin), and every shipped
`verify/selfcheck.json`.

**What it is.** The A21/A22/A23 **checks** were rewritten to the patch space — the reuse boundary,
the moved boundary, the size boundary — and the rows' `expected`/`actual` fields carry the new
text. But the `WORDING` table, which is what fills the row's `assertion` field, was never
updated. So every shipped bundle says this:

```
A21  assertion: "the bundle covers both trim outcomes: at least one eager and at least one deferred"
     expected:   "2 with nothing reusable, 3 with something"
     actual:     "2 with nothing reusable, 3 with something"
```

```
A22  assertion: "the bundle covers both write directions: at least one ascending and at least one descending"
     expected:   "2 in place, 1 moved"
```

```
A23  assertion: "the bundle covers a file that grows: at least one capture with growth_bytes > 0"
     expected:   "2 grow, 2 shrink"
```

**Why it matters.** This is the exact leak the branch exists to remove: `girpr`'s own vocabulary —
`eager`/`deferred`, `ascending`/`descending`, `growth_bytes` — published in the bundle, in a row
whose own `expected` field says something different. A reader of `selfcheck.json` is told the
bundle covers "trim outcomes" when the check is about the reuse boundary; the row contradicts
itself. It also contradicts `tools/capture/docs/00-spec.md:577-579`, which states the new wording.

**And it is pinned by a test**, so CI will not catch it:
`tools/capture/tests/capture_layout.rs:1010-1012` asserts the three old strings verbatim.

**Fix.** Three strings, plus the pin:

```rust
// tools/capture/src/emit/selfcheck.rs:185-196
(
    "A21",
    "the bundle covers both sides of the reuse boundary: at least one capture with nothing \
     reusable and at least one with something",
),
(
    "A22",
    "the bundle covers both sides of the moved boundary: at least one capture whose reused \
     chunks are in place and at least one whose reused chunks moved",
),
(
    "A23",
    "the bundle covers both sides of the size boundary: at least one capture whose file grows \
     and at least one whose file shrinks",
),
```

Use the wording from `tools/capture/docs/00-spec.md:577-579` **verbatim** — that table is the
specification, and the two must not be able to drift.

Then update `tools/capture/tests/capture_layout.rs:1010-1012` to the same three strings.

**And then make them unable to drift.** `coverage()` (`selfcheck.rs:246`) builds its rows with
`Assertion::row(...)`, which fills the row's `assertion` field from `wording(id)` and leaves
`expected`/`actual` for the check's own numbers — so the published claim and the published
measurement are two different sentences, and nothing checks that the claim is *about* the thing
that was measured. Add a guard in `selfcheck.rs`'s own tests:

```rust
// next to a_bundle_without_both_sides_of_the_reuse_boundary_fails_a21
#[test]
fn the_coverage_rows_are_about_the_patch_space_and_not_the_repair_spec() {
    // The wording table is what the bundle publishes. It has to describe the
    // patch space; the retired classes (`trim_eager`, `moved_ascending`, the
    // `growth_bytes` counter) are the repair tool's own vocabulary and must
    // not come back through the wording.
    for (id, forbidden) in [
        ("A21", ["trim", "eager", "deferred"]),
        ("A22", ["direction", "ascending", "descending"]),
        ("A23", ["growth_bytes"]),
    ] {
        let w = wording(id).to_lowercase();
        for word in forbidden {
            assert!(
                !w.contains(word),
                "{id} still names the repair spec's own vocabulary: {word}"
            );
        }
    }
}
```

The existing pin in `capture_layout.rs` already covers the "the wording changed" case; this one
covers the "the wording changed back to the repair spec's vocabulary" case, which is the one that
actually happened.

---

## 2. P2 — a shipped spec makes a claim the tree cannot back

**Where:** `docs/06-artifact-bridge-spec.md:44-48`.

> **The claim is not specific to one version pair.** It has been checked against `hk4e_global`
> 7.0.0 → 7.1.0 and against 6.7.0 → 7.1.0: two bundles, two different sets of files and two
> different sets of numbers, and the same six assertions green on both.

`artifacts/` is gitignored, and no 6.7.0 bundle is retained, so **a reader cannot reproduce or
check this**. The bundle that *is* present carries `api/tag_probe.json`, which does show `6.7.0`
was probed and served — so the check was probably done — but "probably done" is not what a shipped
spec should say.

The plan's M9 ("the second Tier 2 pass") was marked **optional**, and the plan was explicit that
its *account* is `git log`'s job while only the *after* facts survive. This sentence is a
consequence the plan did not anticipate: an optional milestone's result was promoted into a
permanent claim.

**Fix — one of three, in order of preference:**

1. **Retain the second bundle** somewhere the tree can point at (a release artifact, a
   `docs/06` note naming the path), and say where.
2. **State it as a historical observation** rather than a standing property, and give the command
   that reproduces it:

   ```text
   **The claim is not specific to one version pair.** It was checked against
   `hk4e_global` 7.0.0 → 7.1.0 and against 6.7.0 → 7.1.0 — two bundles, two different sets of
   files and two different sets of numbers, and the same six assertions green on both. The
   second bundle is not kept in the tree (`artifacts/` is gitignored); reproduce it with
   `cargo run -p gircap -- capture --old-tag 6.7.0 --new-tag 7.1.0 --out artifacts`, then point
   the bridge at it with `GIRPR_ARTIFACT_BUNDLE`.
   ```

3. **Drop the sentence.** The claim as stated in §1.1 — "`girpr`'s output is identical to what
   another repair tool computes" — does not need the second pair to be true.

What is **not** acceptable is leaving a claim in a shipped document that nothing in the tree can
confirm or refute.

---

## 3. P2 — a consuming-procedure assertion the bridge does not make

**Where:** `tools/capture/docs/00-spec.md:922` versus `tests/artifact_bridge.rs:572-578`.

`00-spec.md`'s §8.1.1 table lists five rows. The fifth is:

> | - | for `removed`: the path is gone, and `deleted_extra_bytes` equals `old.size` | `SUMMARY` + the tree diff |

The bridge asserts **only the first half** — that the path is gone:

```rust
// tests/artifact_bridge.rs:572-578
if capture.absent_after {
    assert!(
        !target.exists(),
        "{}: {} must be gone after the cleanup",
        capture.id, capture.path
    );
}
```

`deleted_extra_bytes` is never compared against `old.size`, and `download_bytes == 0` is never
asserted for `removed` either — assertion 3 is skipped because `totals` is `null` for that
capture, so `capture.download_bytes` is `None`.

**Why it matters.** `docs/06` §7 lists exactly **three** assertions plus the preconditions and no
`removed`-specific row, so the two shipped documents disagree with each other. The `00-spec.md`
procedure is the one a human follows by hand, and it tells them to check a figure the tool never
checks — so a human doing it by hand will find a number that does not match anything the bundle
states.

**Fix — one of two:**

- **Make the bridge assert it** (the better fix — the figure is a cross-tool fact: the bytes the
  cleanup deleted are the old file's size, and the patchmap states `old.size`):

  ```rust
  // tests/artifact_bridge.rs, in the `absent_after` branch
  if capture.absent_after {
      assert!(!target.exists(), ...);
      assert_eq!(
          run.num("summary", "deleted_extra_bytes"),
          capture.old_size,
          "{}: the cleanup must delete exactly the old side's bytes",
          capture.id
      );
      assert_eq!(run.num("summary", "download_bytes"), 0, "...");
  }
  ```

  Note that `Capture.old_size` is already read by `tests/artifact_bridge/bundle.rs:41` and is
  currently unused by the driver — this is what it is for.

- **Or drop the row from `00-spec.md:922`** and let `docs/06` §7 stand as the single statement of
  what is asserted.

Either way the two documents must say the same thing.

---

## 4. P3 — stale prose and numbers that survived the fold

All of these are cheap and none of them changes behaviour. Grouped by file.

### `tools/capture/docs/01-artifact-schema.md` — the reference numbers are half-updated

The header at line 23 was updated and the body was not, so the document now contradicts itself and
the bundle it describes:

| line | says | the bundle actually has |
|---|---|---|
| 196 | "In the reference capture: **277** rows" | **43** (`api/requests.ndjson`) |
| 816 | `"file_count": 96, "total_bytes": 208307799` | **77** (a 78-file bundle) — and `total_bytes` is the new figure |
| 851 | `"counts": { "total": 354, "passed": 354 }` | **93** |
| 863-867 | "In the reference capture it is 354: A1 6, A2 2, … **A6 259** …" | 93, and the paragraph contradicts *itself* ("25 stored chunks produce 25 A6 rows" vs "A6 259") |

Line 167 already says the right thing (`file_count: 77` for a 78-file bundle), which is why the
document reads as if it were half-converted rather than wholly stale.

### `tools/capture/docs/04-bundle-differ.md:191`

```
| | mock | reference (`20261021T111044Z`) |
| patchmaps/ | 8 | 10 |
```

The reference bundle has **6** patchmaps. The rows above and below it in the same table (`files`
79 → 78, `chunk blobs` 22 → 25) were updated; this one was missed.

### `tools/capture/docs/00-spec.md`

- **:1026** — "Ten reused chunks across the **four** captures that declare them". The reuse counts
  are `identical` 1, `reuse_in_place` 1, `reuse_moved` 8 — **ten** chunks across **three**
  captures. `no_reuse` declares none, and `added`/`removed` have no plan.
- **§4.1's published predicate table (around lines 248-255)** — the table states
  `reuse_in_place` = `0 < reused < nchunks`, `moved == 0` and `reuse_moved` = `moved > 0`, but
  `tools/capture/src/select/classes.rs:220-256` additionally requires `same_md5: false` for
  `NoReuse`, `ReuseInPlace` **and** `ReuseMoved`. The carve-out is documented at
  `classes.rs:204-219` — it is what keeps the retiled `Music4.pck` out of the direction classes
  and makes the class set a partition — but the published table omits it. A reader implementing the
  published table would put `Music4.pck` in `reuse_moved` and break the "the class set is a
  partition" claim the very next paragraph makes. **Add the carve-out to the table.**

### `tools/capture/src/select/`

- **`mod.rs:399-404`** — `fn considered_counts(pool: &[SizedCandidate]) -> [u64; 11]` with
  "One walk of the pool instead of eleven. The eleven numbers are the same ones the per-class
  `filter().count()` produced." The array is sized for the old 11-class set; only indices 0..5 are
  ever used. Make it `[u64; Class::ALL.len()]` and fix the comment.
- **`mod.rs:783-784`** — `REPOOL_NOTE` says "the spec records this class as absent for 7.0.0 ->
  7.1.0". Under the new class set **all six** classes have candidates for that pair, so the note
  is false wherever it can be attached.
- **`require.rs:67-69`** — "Requiring all **eleven** by default would make *every* run of a pair
  like 7.0.0 → 7.1.0 exit `4`, because that pair genuinely lacks a `reuse_moved` file". Doubly
  false now: there are six classes, and that pair **does** have a `reuse_moved` file (it is one of
  the six captures).
- **`classes.rs:62-64, 86-87`** — `Shape::Scored { old_size, new_size }` are carried but **no
  predicate reads them any more**. They are only read by the tests' table at
  `classes.rs:312-332`. Either drop them or say what they are for.

### `tools/capture/src/verify/mod.rs`

- **:430-431** — "The bundle's **coverage**: whether it shows both trim outcomes, both write
  directions and a file that grows." The check is about the reuse boundary, the moved boundary and
  the size boundary.
- **:440** — "and 'this bundle does not cover both trims' is a thing it said."
- **:453** — the stderr message "the bundle does not cover the **repair spec's decisions**". It is
  the **patch space**, not the repair spec's decisions — the repair spec's decisions are
  `girpr`'s and are not what a bundle covers.
- **:38-43** — the module doc's list of what `verify` cannot re-derive does not mention
  A21-A23.

### `tools/capture/tests/`

- **`case_coverage.rs:3-19`** — the module header still says the gate compares "the script's
  outcome table | the patchmaps' `repair` blocks" (a block that no longer exists — the test
  compares against `totals`), and describes a drift ("it counted a *kept* chunk toward the two
  salvage counts… it always named a write direction") that is no longer a drift, because the
  bundle publishes neither.
- **`measured_fixture.rs:94`** — a literal `` `r`n `` was left in a comment by a mechanical
  replacement:

  ```rust
  // common + changed, single chunk, nothing reusable -> no shape: a one-chunk`r`n        // file with nothing reusable is a degenerate patch
  ```

### `tools/capture/src/manifest/diff.rs` — the last of girpr's vocabulary in the bundle

`moved_up` (:90), `moved_down` (:93) and `old_max_end` (:101) are still published in
`diff/candidates.json` and `diff/paths.tsv`, and they are framed in girpr's terms:

- `moved_up`'s doc: "destination. **The descending write order keeps these alive.**"
- `old_max_end`'s doc: "**The trim test's input.** The repair releases the dead tail *before* …"

The **values** are facts about the patch (which chunks moved, how far the old side reaches), and
the class set no longer uses them. But the **framing** is girpr's, and the framing is what a
reader of the document takes away. The bundle is not supposed to know what a "write order" is.

Either rename and re-frame them as patch facts (`moved_later` / `moved_earlier` / `old_max_end`,
with the doc saying what a consumer can conclude from them), or drop them from the published
columns. Computing them is fine — the plan explicitly allows that — **publishing** girpr's
vocabulary is what the branch set out to stop.

### `tools/capture/tools/case_coverage.py` — computes against a bundle that no longer says it

`outcome_rows()` (:338-469) still computes `trim` (eager/deferred/none), `direction`
(ascending/descending/n/a), `keeps_ascending`, `keeps_descending`, `payload` and `growth` from a
bundle that **publishes none of them** — so these values are now compared against nothing. Its
docstring at :341 points at `assemble::repair_of` **in the crate**, a function this branch deleted
(`grep -r repair_of tools/capture/src` → nothing).

The instrument is a dev tool, not a published document, so this is lower priority than the rest —
but it is now dead work with a dead pointer, and either it should read the bundle's new shape or
it should say plainly that it is computing its own expectation.

### `gircap list` skips a v1 bundle instead of refusing it

`tools/capture/src/verify/mod.rs:866-877` — `gircap list` walks the root and, for a directory with
a `MANIFEST.json` that does not open, logs a warning and **skips** it. `verify` and `materialize`
both refuse a v1 bundle with a good message (`verify/mod.rs:145-155`), but `list` makes it
disappear from the index instead. A user with one v1 bundle and one v2 bundle sees only the v2
one, with no explanation beyond a `warn` line.

**And there is no test for the v1 refusal** — `grep -r "schema version 1" tools/capture/tests`
finds nothing, and `verify/mod.rs`'s own tests cover only "not a directory" and "no
`MANIFEST.json`". The schema bump exists precisely so that a v1 bundle can be refused, and that
one behaviour is untested.

```rust
// tools/capture/tests/ — a new test, or a case in the existing verify tests
#[test]
fn a_v1_bundle_is_refused_with_the_command_that_makes_a_current_one() {
    // Write a MANIFEST.json with "schema_version": 1 and nothing else.
    // `gircap verify --artifact <dir>` must fail, and the message must name
    // `cargo run -p gircap -- capture --out <dir>`.
}
```

---

## 5. The plan — recovered from history, and reviewed

The plan was folded at `e3358d6` and is recoverable:
`git show 1f803eb:docs/09-artifact-test-scope-plan.md` (912 lines).

### What the plan got right

It diagnosed the drift correctly and, more importantly, **diagnosed its cause** rather than its
symptom. The sharpest sentence in it is the one about `girpr` reporting **one** `ram_bytes` key
while the bundle stated it **twice**, once per flag — "that is not a bundle fact. That is `girpr`'s
CLI encoded in a foreign schema." That is the whole bug in one line, and the fix follows from it.

Its rule of thumb is the right one and the tree follows it exactly:

> **A test belongs in Tier 1 if another repair tool could plausibly do it differently. It belongs
> in Tier 2 if the answer must be the same whatever the implementation.**

Its **cost model** was the part I most expected to be wrong and was not. The "before" column
(696 MB, 79 runs, 12.25 GiB of copies, 233 s) is the plan author's own measurement on the author's
machine — I did **not** re-measure `main`, so I can only report that the "after" column was met or
beaten: 97.7 MB, 12 runs, ~20 MiB of copies, **15.1 s**.

Its **migration map** (§4) accounted for every one of the 26 old cases, and the tree accounts for
every one of them too:

| the plan's map | where it landed |
|---|---|
| 11 `capture_test!` cases (plus `one_capture_patches_to_the_new_version`, 12 in all) | 6 bridge cases (the class set shrank) |
| 7 harness cases | `tests/artifact_server.rs`, 7 cases, on a synthetic mini-bundle |
| 2 bundle-consistency cases + 1 discovery case | `tools/capture/tests/bundle_consistency.rs`, 3 cases |
| 4 `girpr`-only cases (`the_ordered_tier…`, `a_ram_budget…`, `the_discard_mode…`, `the_in_place_path_re_verifies…`) | `tests/repair_machinery.rs` |
| the driver assertions that were not cross-tool facts | `tests/repair_machinery.rs`: the live/settled figure pair, `local_manifest` ok/unavailable, the scope containment, the trace log's download records against the server's request log, and the path-with-a-space glob |

Its **fold discipline** held: `git grep "09-artifact\|artifact-test-scope"` is empty, the README's
docs table has no row for the deleted plan, and no source comment points into it. That is the
house pattern (`be5fc4b`, `516d710`, `818f8ee`, `12c9531`) applied correctly.

### What the plan missed

**One thing, and it is the same thing as P2.** M9 — "the second Tier 2 pass" — was marked
*optional*, but the plan also said, in §14, that only the **after** facts survive the fold and that
"the measurements, the rejected alternatives and the per-case migration map are the *argument* for
the change, not a description of the tool". Those two rules collide on an optional milestone whose
result is a *claim about the tool*: the plan's fold rule pushed a "we checked it twice" fact into
the shipped spec, where it became a permanent property of the tool that nothing in the tree can
back. §2 of this note is the fix.

The plan also did not anticipate that a **stale string table** could survive the schema change —
`WORDING` is not a schema field, so nothing in the schema work touched it, and the test that
pinned it pinned the wrong text (P1). A plan step for "the wording table is part of the contract,
not a label" would have caught it.

---

## 6. Housekeeping

- **Delete this file** when the fixes land, in its own commit, exactly like
  `docs/07-streaming-zst-decode-review.md` was deleted at `13c522c`. Nothing should refer to it
  afterwards — not the code, not the documents, not the comments.
- **Do not cite this note** from a commit message, a source comment or a document. A finding that
  needs a permanent home moves into the document that owns it: §1 into `tools/capture/src/emit/selfcheck.rs`
  and `tools/capture/docs/00-spec.md`, §2 into `docs/06-artifact-bridge-spec.md`, §3 into
  `tools/capture/docs/00-spec.md` and `docs/06-artifact-bridge-spec.md`, §4 into the files named in
  it.
- The bundle under `artifacts/hk4e_global/7.0.0__7.1.0/20261021T111044Z` is **gitignored** and
  will not travel with the branch. Anyone picking this up needs to re-capture it before running the
  ignored suite.
