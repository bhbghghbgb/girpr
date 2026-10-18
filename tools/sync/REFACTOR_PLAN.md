# `girsync` refactor / optimization plan

**Scope:** `tools/sync/` (crate `girsync`, binaries `girsync` + `sled2redb`).
**Goal:** remove real cost and real duplication, ordered so that every step is
independently safe to land. Nothing here changes the CLI surface, the stdout
record vocabulary, the exit codes, or the documented semantics in `README.md`
unless a step explicitly says so.

**Baseline measured before writing this** (Windows, `debug` profile, synthetic
trees of identical files so nothing is copied):

| command | 2 000 files | 5 000 files |
| --- | --- | --- |
| `sync --dry-run` (default, case-insensitive) | 1.36 s | 6.42 s |
| `sync --dry-run --case-sensitive` | — | 1.04 s |
| `compare` (case-insensitive) | — | 1.43 s |

258 tests across 19 targets, all passing. `cargo clippy -p girsync --all-targets`
is clean except one `empty_line_after_doc_comments` warning in
`tests/rw_threads/main.rs`.

---

## How risk is ordered here

**Risk = chance of changing an observable behaviour that a test or a user could
notice.** Not "how hard the edit is". A mechanical change to a load-bearing
invariant is high risk; a large mechanical change to a leaf is low risk.

Three tiers, and the ordering inside each tier is the ordering of value:

| tier | meaning | how to land it |
| --- | --- | --- |
| **§1 Low** | behaviour-preserving by construction, or provably dead | one commit each, green tests is sufficient |
| **§2 Medium** | behaviour-preserving in intent, but touches a shared path or a performance property | one commit each, **plus** the named test added *before* the change |
| **§3 High** | changes something a user or a downstream reader can observe | needs the named characterisation test written and **committed first**, and a README update in the same change |

---

## §1 Low risk

### 1.1 Drop the dead `LiveEnt::abs` field — `src/scan.rs:15`

`LiveEnt` carries `abs: PathBuf`, constructed at `scan.rs:73` and `scan.rs:83`.
Nothing reads it — `rg '\.abs\b'` across `src/` and `tests/` returns only the
declaration and the two constructions. Every consumer re-derives the path with
`root.join(rel)` (`phase_c.rs:120`, `apply.rs:384`) or works in rel-path space
(`phase_a.rs`, `build_plan`).

On a 200 k-file tree this is 200 k `PathBuf` allocations (several MB of string
data) built for nobody. Deleting the field, the two literals, and the `PathBuf`
import is the whole change.

**Verify:** `cargo build -p girsync` + the existing suite. There is no test to
add, because there is no behaviour to characterise.

---

### 1.2 Fix the one clippy warning — `tests/rw_threads/main.rs:143`

An empty line after a doc comment. Mechanical, zero risk, keeps
`clippy --all-targets` at zero so a real warning is visible later.

---

### 1.3 Hoist the repeated `ScanMode` literals in `compare` — `src/commands/compare.rs`

`compare.rs` builds the same two `ScanMode { no_trust_cached_hashes, dry_run }`
values **four times** (lines 84, 98, 151, 163). `commands/sync/mod.rs:166-171`
already does this correctly with a `scan_mode` closure. `compare` is the outlier
and it is the one that can drift: a fourth field added to `ScanMode` has four
sites to update here versus one in `sync`.

Adopt `sync`'s shape verbatim:

```rust
let scan_mode = |no_trust_cached_hashes| ScanMode { no_trust_cached_hashes, dry_run: common.dry_run };
let src_mode = scan_mode(trust.no_trust_src);
let dst_mode = scan_mode(trust.no_trust_dst);
```

Then the four literals become `src_mode` / `dst_mode`. Pure substitution — the
values are provably identical today, and the compiler enforces they stay so.

---

### 1.4 Stop allocating two `String`s per `hash_file` call — `src/hash.rs:64-72`

```rust
let mut md5ctx  = if algos.contains(&"md5".to_string())    { ... }
let mut sha2ctx = if algos.contains(&"sha256".to_string()) { ... }
```

`Vec::contains` takes `&String`, so each call allocates a fresh `String` purely
to compare it against borrowed data. This is on the hottest path in the crate —
once per file per phase C, and twice per copied file in `copy_one`.

```rust
let wants = |a: &str| algos.iter().any(|x| x == a);
let mut md5ctx  = wants("md5").then(md5::Context::new);
let mut sha2ctx = wants("sha256").then(sha2::Sha256::new);
```

Also move the `algos.is_empty()` early return **above** the context
construction: today an empty request builds two contexts and then throws them
away. Behaviour is identical (the function already returns an empty map), but
the ordering is now honest about what the flag means.

---

### 1.5 Drop the `abs`-adjacent double `to_str`/`replace` in the walk filter — `src/scan.rs:35-48`

`walk_live`'s `filter_entry` closure does a full `strip_prefix` + `to_str` +
`replace('\\', "/")` — i.e. one `String` allocation — **per visited entry**,
purely to test `is_cache_rel`. Then the loop body at `scan.rs:55-63` immediately
recomputes the exact same `rel` for every entry that survives.

The filter only ever needs to know *whether the first path component starts with
`girpr-cache`*; it never needs the whole normalised string. Compute the rel
once per entry and reuse it for both the prune decision and the emitted
`LiveEnt`. This removes one `String` allocation per entry on a 200 k-file tree.

Low risk because the filter's *predicate* is unchanged — only the number of
times it is computed changes. Keep `is_cache_rel` exactly as it is; the
refactor is purely about not computing its argument twice.

---

## §2 Medium risk

### 2.1 **The rename pass is O(n²) — `src/commands/sync/rename.rs:43-53`**

This is the one measured performance defect in the crate, and it is large.

```rust
for (lk, srel) in &by_lower {
    if let Some(drel) = dm.keys().find(|k| k.to_lowercase() == *lk && *k != *srel) {
```

For every src path this linearly scans **all** dst paths, and allocates a
lowercase `String` for each comparison. Cost is `|src| × |dst|` string
allocations. Measured, case-insensitive `sync --dry-run`, synthetic identical trees:

| files | time |
| --- | --- |
| 500 | 0.18 s |
| 1 000 | 0.46 s |
| 2 000 | 1.27 s |
| 4 000 | 4.20 s |

Each doubling roughly **quadruples** — textbook `O(n²)`. The same tree with
`--case-sensitive` (which skips the rename pass entirely) runs 5 000 files in
1.04 s against 6.42 s for the default. The rename pass is ~5× of the run's
wall time and it grows without bound; at 100 k files this is the difference
between seconds and hours.

**Fix:** build the dst lowercase index once, exactly as `src` already is at
`rename.rs:31-34`, and do a hash lookup instead of a scan:

```rust
let mut dlow: HashMap<&str, &String> = HashMap::new();
for k in dm.keys() { dlow.insert(&k.to_lowercase(), k); }
// ... per src key:
if let Some(drel) = dlow.get(lk.as_str()).copied().filter(|d| *d != *srel) { ... }
```

This turns `O(n·m)` into `O(n + m)` and removes the per-comparison allocation.
The result set is **identical**: `find` returns the first match in `HashMap`
iteration order and the index returns the same key, because within one side
mixed-case collisions are already fatal (`scan.rs:112` `check_mixed_case`), so
each lowercase key has exactly one dst path under it. That invariant is what
makes the swap safe, and it is why this is medium rather than low.

**Land with:** a test asserting the rename set is exactly the set of paths whose
lowercase key matches src's with a different casing — stated as a set, not a
transcript, per the project's no-goldens rule. Add a case with **zero** renames
and a case with several, since the current unit test only covers one.

---

### 2.2 `diff_maps` rebuilds the lowercase index it could be handed — `src/diff/mod.rs:259-298`

The insensitive branch allocates a lowercase `String` per key for **both** sides
(`slow`, `dlow`), then allocates a third `HashSet<String>` of all lowercased keys
(`mod.rs:269-272`) — so every path is lowercased and cloned three times before a
single pair is compared.

The `keys` set can be eliminated entirely: iterate `slow` then the `dlow` keys
*not* present in `slow`, which is the same union without the third allocation.
`slow` and `dlow` are also nearly always the same key set in practice (the
undecided set dominates), so a single pass over the smaller map plus a lookup
into the larger does the same work with one index instead of three.

Do this **after** 2.1 lands and is green, and land it separately — the two
changes touch adjacent reasoning and conflating them makes a regression
ambiguous.

---

### 2.3 `plan_pairs` duplicates the pairing rule `diff_maps` already owns — `src/planner/pairs.rs:70-101`

`shared_pairs` re-derives "which paths both sides hold, paired by exact key when
sensitive and by lowercase when not" — the exact rule `diff_maps` pairs by. The
module doc is explicit that the two **must not** diverge, and the only thing
enforcing that is a comment plus parallel test coverage.

That is a real structural risk, and it is currently load-bearing on the sync
path: `sync` calls `plan_pairs(..., common.case_sensitive, ...)` and then
`diff_maps(..., true, ...)` with a *deliberately different* flag (`mod.rs:196-294`,
and the long comment at `sync/mod.rs:189-195` explaining why). Any
"deduplicate these" refactor that unifies the pairing will silently break case-
only handling, which `tests/sync.rs` covers with exactly one test.

**Recommendation: do not refactor this.** Instead, make the invariant
structurally obvious rather than duplicated — have `shared_pairs` take the
`CasePairing` enum (`Exact` | `Lowercase`) that the two call sites already
imply, so the asymmetry between "plan pairs as on disk" and "diff pairs
post-rename" is a value at the call site rather than a bare `true`/`false` with
a paragraph of comment. Pure clarity, no behaviour change, and it removes the
most dangerous thing in the codebase: a bare boolean that must differ between two
adjacent calls.

---

### 2.4 `Record::fields_json()` is built on every emit, including unfiltered — `src/report/mod.rs:99`

```rust
pub fn emit(&self, rec: Record) {
    debug!(event = %rec.event(), fields = %rec.fields_json(), "report");
    println!("{}", self.render(&rec));
}
```

`rec.event()` does `to_ascii_lowercase()` (an allocation) and `rec.fields_json()`
builds a whole `serde_json::Map` and serialises it to a `String` — **on every
record, regardless of whether debug logging is enabled**. `tracing` only decides
whether to *write* the event after the arguments are evaluated, so at the default
`--log-level info` this is pure waste.

On a run with `--show-identical` over 200 k files this is 200 k needless JSON
serialisations plus 200 k lowercase conversions, all discarded.

**Fix:** gate the argument construction on whether debug is actually enabled.

```rust
use tracing::level_filters::STATIC_MAX_LEVEL;
use tracing::Level;
pub fn emit(&self, rec: Record) {
    if Level::DEBUG <= STATIC_MAX_LEVEL && tracing::enabled!(Level::DEBUG) {
        debug!(event = %rec.event(), fields = %rec.fields_json(), "report");
    }
    println!("{}", self.render(&rec));
}
```

`Report::emit` is the crate's single stdout writer, so this is one place, and
the `"record is logged as well as printed"` promise is preserved whenever debug
is actually on.

**Land with:** a test that emits at `--log-level debug` and asserts the log
event still carries the fields, so the gate cannot be read as "logging was
dropped".

---

### 2.5 `prune_dst_cache` re-walks dst after phase A already walked it — `src/commands/sync/apply.rs:352-373`

The final apply phase calls `walk_live(self.dst, ...)` to rebuild a
`HashSet<String>` of present paths. `dm` (the post-rename dst map) is already
in scope on `Applier` and already holds exactly the filtered, depth-capped set
of dst paths — the module doc at `plan.rs:186-188` states this explicitly as the
reason `rmdir` is read rather than derived.

The walk is *not* redundant in one respect: phase A's map predates this run's
own writes, so it does not know about files `copy_one` created. But those are
all `plan.copy` paths, and `plan` is also in scope. So the present set is
derivable as `dm.keys() ∪ plan.copy` — no second full walk.

That removes a complete filesystem traversal of dst from every real `sync`. On a
large game tree that walk is a meaningful fraction of the run.

**Risk:** if the derivation is wrong, `prune_dst_cache` deletes live cache rows,
which manifests as a *later* `compare-self` reporting phantom drift — a delayed,
misattributed failure. That is why this is medium and not low.

**Land with:** a test that syncs a tree where `copy_one` creates files in a
directory `mkdir` also created, asserts the new paths' rows survive in dst's
cache, and asserts a genuinely absent path's row is dropped. Test the assertion
against the cache, not against `pruned`.

---

## §3 High risk

**§3.1 is decided** (see below); 3.2 and 3.3 are still open but need no
decision, only a reason to bother. They sit in this tier because of *where* they
are — the durability boundary the README calls out — not because they change
anything a user can observe, which is what the tier otherwise means. §3.2 says
so of itself: "behaviour-preserving but it adds a cache API and touches the
durability path, so it lands medium-to-high."

### 3.1 `copy_one` re-reads src and dst to verify, after already having streamed both — `apply.rs:483-484`

`copy_one` streams src→dst, then calls `hash_file(&s, algos)` and
`hash_file(&d, algos)` — two more full reads. On a cold sync with verification
that is **3 reads + 1 write** per copied file.

This changes the meaning of verify-after-copy from "re-read both sides and
compare" to something weaker. On a live working directory that is a real
difference, not a theoretical one, so it needs a decision rather than a
judgement call. **Decided — see below.**

#### The fix this section originally proposed would not have worked

The earlier draft of this section said the digests "could be computed *during*
the stream copy in the same pass". That is not a weaker verification — it is
**no** verification. Both digests would come out of one loop over one buffer,
so they are equal by construction and `verify hash mismatch` (`apply.rs:487`)
can never fire. The check would assert `x == x` and read as a guarantee.

So the choice is not between "safe" and "risky". Of the two candidate shapes,
one is a real check and the other is not a check at all:

| shape | I/O per copied file | what `verify hash mismatch` means |
| --- | --- | --- |
| today (unchanged) | 3 reads + 1 write | compares the bytes on disk now against the bytes on disk a moment ago |
| **chosen** | **2 reads + 1 write** | compares a digest of the stream read against a digest of the bytes actually on disk |
| full single pass | 1 read + 1 write | nothing — the two digests are the same value |

#### Decision: stream-hash src, re-read dst

`hash_file`'s streaming loop is already there (`hash.rs:62-101`, a 512 KB buffer
feeding both contexts per chunk), so hashing src as the copy streams is a
mechanical substitution for `std::io::copy`. **dst is still re-read and still
hashed.**

That drops the src re-read and keeps every guarantee that matters:

- the two digests still come from two independent reads, so the comparison is
  real;
- `fout.sync_all()` runs before the re-read, so the bytes being hashed are the
  bytes that reached the platter — the write path still catches silent on-disk
  corruption, which is the failure a single pass structurally cannot see and the
  one this tool most needs to see on a large tree on a suspect disk;
- it drops only "src changed between the stream read and the check". That race
  is unwinnable anyway, and phase A's stat catches it on the next run.

**Lands with:** a README update in the same change, plus these four sites, which
all state the double-read as fact and would otherwise become false —
`cli.rs:177` (ships in `--help`), `rw/mod.rs:8`, `apply.rs:227`, and the
`predrop_cache_entries` note at `apply.rs:159-163`, which already lists "reuses
a digest across a content change" among the changes that would make its
pre-drop load-bearing. This change does not cross that line: the digest is still
computed from a real read of dst, never carried over from a previous copy.

### 3.2 `effective/phase_a` loads the whole cache twice in insensitive mode — `phase_a.rs:98-113` and `222-225`

In the default (case-insensitive) mode, `scan_stat_only` calls `cache.load_all()`
up front for the alternate-casing index, then calls `w.load_all()` again at
`phase_a.rs:222` for the prune set. Two full decodes of the entire `entries`
table per side per run.

The prune set only needs the **key set**, not the decoded `FileRec` values —
`phase_a.rs:226-231` uses `.keys()` and nothing else. A `load_keys()` that
iterates without decoding would cut the second pass to roughly a third of the
cost and a twentieth of the allocation.

This is behaviour-preserving but it adds a cache API and touches the durability
path, so it lands medium-to-high. Worth doing on a large tree; not worth doing
for tidiness.

### 3.3 Phase A holds the whole `live` walk and the whole `live_set` in memory — `phase_a.rs:77-86`

`walk_live` returns `Vec<LiveEnt>` for the entire tree, and `scan_stat_only`
immediately builds a second `HashSet<String>` of every included rel —
duplicating every path string. On a 200 k-file tree that is a second full copy
of the path set, alive for the whole phase.

`live_set` exists so the prune loop can ask "is this row still live?" in O(1)
without re-running the globs. That is the right trade, but the duplication is
avoidable: the set could hold `&str` borrowed from `live` rather than owned
`String`s, which removes the copy entirely and keeps the O(1) lookup.

**Not quite the one-line change it looks like.** `phase_a.rs:122` is
`for e in live` — by value — so a `live_set` borrowing from `live` will not
borrow-check until that becomes `for e in &live`. Both lookups change shape
with it (`contains(&e.rel)` → `contains(e.rel.as_str())`). `live` is not used
after the loop (`total_live` is taken at line 97), so nothing else moves; but
this is a loop-and-signature change in the durability boundary, which is what
puts it here rather than in §1.

Low-risk in isolation, but it changes a signature in the module the README calls
out as the durability boundary, so it belongs with 3.2 rather than in §1.

---

## Explicitly *not* doing

Naming these because each looks like an opportunity and is not:

- **Unifying `scan_stat_only` and `load_record_side_from`.** They are not two
  variants of one walk — one reads a directory, one reads a table — and
  `effective/record.rs:1-7` already says so. Merging them would make the
  record path look swappable with the disk path, which is precisely the mistake
  that path exists to prevent.
- **Making `side`/`dst` derive from a trait instead of `match`.** Two variants,
  one dispatch site (`effective/side.rs:29-38`). A trait here would be
  ceremony with no second implementor.
- **Any change to the `--rw-threads*` family, `RwLimiter`, or the
  src→dst lock order.** `rw/gate.rs:124-131` is the one place in the crate
  where a wrong edit can *hang* a run rather than fail one. The current tests
  (`gate.rs` unit tests + `tests/rw_threads/limiter.rs`) are good; leave them
  alone.
- **Touching `plan_pairs`'s coverage check or `pair_verdict`'s
  `unverifiable` clause.** These two together close the last path by which a run
  can report a confident answer about content nobody read. They are correct,
  heavily tested, and the failure mode of a regression is a *false clean* —
  the worst outcome the tool can produce.

---

## Progress log

**§1 and §2 are done. §3 is untouched** — one decision now made, and two steps
that never needed one. Ten commits, one per step, plus a repo-wide
`cargo fmt`. Suite green at every
one: **258 → 275 tests** (274 run, 1 `#[ignore]`d — see the §2.5 note),
`cargo clippy -p girsync --all-targets` clean.

| step | commit | what changed | tests |
| --- | --- | --- | --- |
| §1.2 | `1ec7661` | `drop_caches`' orphaned doc comment moved to `limiter.rs`, where the function went in `4d3fbfe` | — |
| §1.1 | `1ec7661` | `LiveEnt::abs` deleted — written twice, read nowhere | +2 |
| §1.5 | `1ec7661` | walk filter's double `rel` derivation removed | (the +2) |
| §1.3 | `0022e97` | `compare`'s four `ScanMode` literals hoisted | — |
| §1.4 | `53dd207` | `hash_file`'s two per-call `String` allocations removed | — |
| §2.1 | `5349be6` | rename pass indexed by lowercase key — **4.3× at 8 000 files** | +5 |
| §2.4 | `3252072` | `Report::emit`'s log payload built only when debug is on | +1 |
| §2.2 | `58992d4` | `diff_maps` walks two indices instead of three | +5 |
| §2.3 | `2175be4` | pairing rule is a `CasePairing` value, not a bare `bool` | — |
| §2.5 | `e3faa7f`, fixed in `see below` | `prune_dst_cache` derives the present set — **~1 s saved at 20 k files** | +4 |
| — | `84e992e` | repo-wide `cargo fmt` (see the note at the bottom) | — |

### After the merge with `main`

`sync/refactor` merged `main` at `098da51`. Three of main's commits touch this
crate — `bfd6c9f`, `2cf1673`, `8702234`, all scratch-directory hygiene for
tests. **None of them touch `phase_a.rs`, `copy_one`, or the cache API**, so §1,
§2 and §3 are all unaffected by the merge and the numbers above still hold:
275 tests, 274 run, 1 `#[ignore]`d.

They did leave two claims on this page untrue, both now fixed:

- `2cf1673` made `seeded()` hand back the `TempDir` guard instead of a
  `PathBuf`, and left both callers in `cache/db.rs` binding `dir` without using
  it. That is two `unused_variable` warnings, so "`clippy --all-targets` clean"
  above stopped being true. They are `_dir` now, not `_` — the binding's
  lifetime is the cleanup, and a bare `_` would drop the guard on the spot.
- `8702234` appended `impl AsRef<Path>` to `testtemp.rs` without a trailing
  newline, so "`cargo fmt --check` … the tree is clean now" stopped being true
  one commit later.

That is the whole of it: three one-line fixes, no behaviour touched, and a
reminder that a merge can invalidate this page's own housekeeping claims without
invalidating any of its findings.

### §2.5 — found in review, fixed

`e3faa7f` shipped a present set that only ever *added* to `dm`, which is a
**superset** of what is on disk. A review agent found it; I reproduced it before
touching anything, and confirmed the new test passes on the pre-§2.5 commit
(`2175be4`) and fails on mine.

**Trigger (a), real and fixed.** `copy_one` clears a blocking dst directory
*recursively*. `Plan::rmdir` deliberately excludes that subtree (`plan.rs:191-204`)
because the directory is gone by the time the removal pass runs — so `prune_dst_cache`
is the only place that can drop those rows, and a purely additive set kept them.

```
src: x (file)      dst: x/sub/f.txt

before the fix:  rows after the run: [x, x/sub]      <- stale
after the fix:   rows after the run: [x]
```

Fixed by **subtracting** what the run destroys, by path prefix, computed from
`dm`: a copy whose dst is a directory wipes everything under `rel/`. The prefix
test is strictly-longer-than, so the copied path keeps its own row. Pinned by
`rows_for_a_subtree_a_copy_wiped_are_dropped`, at two levels of depth.

**Trigger (b), not fixed, and now stated rather than silent.** `--case-sensitive`
on a case-insensitive filesystem: `mkdir Sub` and a later `rmdir sub` name the
same directory, so the rmdir takes the freshly created tree with it. Deriving
cannot see that without case-folding the prune against the plan — in the one mode
that deliberately does not case-fold. Recorded as
`rows_for_a_directory_created_and_removed_in_one_run_are_dropped`, marked
`#[ignore]` with the reason, and written into `README.md`. The rows are stale
until the next run's phase A prunes them, and `compare-self` builds a *cold*
cache so it cannot be misled. `main`'s walk dropped them as a side effect of
asking the filesystem, not by deciding to.

Impact of both: **cache hygiene only** — no wrong record, verdict or exit code.

### The other finding from that review: a §2.4 test gap, now closed

`a_record_is_logged_with_its_fields_at_debug_level` only ran
`--log-level debug --log-file`, which does **not** discriminate between the gate
actually used and a `STATIC_MAX_LEVEL`-only gate — I checked both in debug and in
release, and a weakened gate passes it. The case that does discriminate is
`--log-file` at the **default** `--log-level`, where only the always-`trace` file
layer wants the event. Added as
`a_record_reaches_the_file_log_at_the_default_log_level`.

That investigation also corrected a claim in my own §2.4 doc comment: I had
written that `STATIC_MAX_LEVEL` folds the branch away in a release build. It does
not — `tracing` is depended on without a `release_max_level_*` feature, so the
ceiling is `TRACE` in every profile and that term is currently just a cheaper
test ahead of the call. The comment now says so.

### Deviations from the plan, and why

**§1.5 — mechanism changed, not just frequency.** The plan suggested computing
the rel once in the `filter_entry` predicate and handing it to the loop body. A
`FnMut(&DirEntry) -> bool` cannot return anything, so that needs a `Cell` to
smuggle it through, which makes correctness depend on walkdir calling the
predicate exactly once per yielded entry — an implementation detail, not a
documented contract. Instead the predicate is gone: the prune moved into the
loop body as a documented `skip_current_dir` call, walkdir's other idiom for
exactly this, and the rel is computed once in one place. `is_cache_rel` is
untouched. Two tests were added rather than none, because this changes the
*mechanism* of pruning: the surviving rel set (cache files, cache dirs and
everything beneath them all gone), and that `max_depth` still bounds the walk
on its own.

**§2.1 — `case_renames` extracted.** The decision moved out of
`rename_to_src_casing` into a function that touches no filesystem or cache, so
the rename set can be stated as a set rather than read off stdout. All five new
tests were run against the **previous** implementation before the swap and pass
there too — they characterise existing behaviour, they do not restate the new
code.

**§2.3 — went past the plan's wording, on purpose.** The plan says to give
`shared_pairs` the enum. That alone would change nothing observable, because the
risk it names is the asymmetry *between two adjacent calls*, and the second call
is `diff_maps`. Both now take `CasePairing`, so `sync` reads
`plan_pairs(…, CasePairing::from(common.case_sensitive), …)` followed by
`diff_maps(…, CasePairing::Exact, …)` — the disagreement is in the values.
`compare` and `compare-self` pass one value to both, which is what makes the
difference between the commands visible rather than incidental. Pure clarity;
no behaviour differs.

**§2.5 — the plan's derivation was wrong twice.** `dm.keys() ∪ plan.copy` deletes
the cache row for every directory the run just created, because `mkdir`'s paths
are in neither list; the fixture written for this step caught it before it
landed. The fix for that (add `plan.mkdir` and `plan.fix_dirs`) was still one
direction only, and the review caught the other. The set is now both directions.

### Measurements

§2.1, synthetic identical trees, `sync --dry-run`, before/after binaries:

| files | before | after | speedup |
| --- | --- | --- | --- |
| 1 000 | 3.44 s | 1.85 s | 1.9× |
| 2 000 | 2.40 s | 1.32 s | 1.8× |
| 4 000 | 5.46 s | 2.58 s | 2.1× |
| 8 000 | 17.35 s | 4.04 s | 4.3× |

The 1 000/2 000 rows are noisy (filesystem cache). The 4 000→8 000 step is the
signal: the old pass roughly triples per doubling, the new one grows ~1.6×, and
the gap widens with n.

§2.5, converged sync over 20 000 identical files, best of 3: **6.35 s → 5.38 s**
before the review fix, **6.60 s → 5.57 s** after it. The saving survives; the
prefix subtraction costs nothing measurable.

Two steps whose effect is real but small, reported honestly rather than dressed
up:

- **§2.4**: 20 000 `IDENTICAL` records via `--show-identical --hash-all-of none`
  went 4.99 s → 4.92 s (1.02×). The gate is verified to fire (probed: 0 times at
  the default level, 3 times at `--log-level debug` for a 3-record run), but the
  wasted serialisation is dwarfed by the walk around it.
- **§2.2**: `compare` over 20 000 identical files went 5.28 s → 5.14 s. One fewer
  allocation per path, not a speedup worth quoting.

### Note on `cargo fmt`

`cargo fmt --check` was **not** clean on this repo at `6d96b1e` — 16 files drifted.
This refactor formatted only the files it touched, and `84e992e` then took the
rest, so the tree was clean when §1 and §2 finished. That commit is mechanical
and unrelated to the refactor; review it as such. It stopped being clean again
when `main` landed `8702234` — see "After the merge with `main`" above.

---

## Where §3 stands

One of the three needed a decision from whoever owns the CLI contract. Two did
not, and the earlier draft of this section said "both" while listing three —
which is the sentence that kept §3.2 and §3.3 sitting here.

**§3.1 — decided, not started.** Stream-hash src during the copy, **re-read and
re-hash dst**. 2 reads + 1 write instead of 3 + 1. See §3.1 above for why the
fix this document originally proposed would have been a tautology, and for the
four sites that state the double-read as fact and have to change with it.

**§3.2 — no decision needed, only a measurement.** `load_keys()` on `CacheDb`
and `CacheWrite`, so phase A's prune pass iterates the `entries` table without
decoding every value. Behaviour-preserving. The only question is whether a large
tree is actually slow enough to be worth adding a cache API on the durability
path for.

**§3.3 — no decision needed, and not free.** `live_set` holds `&str` borrowed
from `live`. See §3.3 above: it needs `for e in &live`, so it is a loop change
rather than the one-liner it reads as. Paired with §3.2, both being in the
durability boundary.

---

## Suggested order

1. §1.2 clippy warning (trivial, do it first so §1 stays visible)
2. §1.1 dead `abs` field
3. §1.3 `ScanMode` hoist in `compare`
4. §1.4 `hash_file` allocations
5. §1.5 walk-filter double `rel`
6. **§2.1 rename pass O(n²)** — the one with real numbers behind it
7. §2.4 `fields_json` gate
8. §2.2 `diff_maps` index
9. §2.3 pairing-rule clarity (no behaviour change, do it for the reader)
10. §2.5 `prune_dst_cache` walk removal
11. §3.2 / §3.3 — no decision needed; only if a large tree is actually slow
12. §3.1 — decided (stream-hash src, re-read dst); ready to land

Each step is one commit, and steps 6-10 should each carry the test named above
written first.

**Status: 1–10 landed.** See the progress log at the top for commits,
measurements, and the two places the implementation departed from what is
written here. 11 and 12 are untouched; 12 has its decision and needs only the
implementation plus the four doc sites named in §3.1.