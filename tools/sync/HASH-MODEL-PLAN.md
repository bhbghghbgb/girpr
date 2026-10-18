# PLAN — hash method identity: `String` → `HashAlg` / `AlgoSet` / `DigestSet`

A self-contained implementation plan. Someone with no prior context should be
able to execute it end to end from this file plus the crate's own docs.

**Scope:** the `girsync` crate at `tools/sync/` in the `girpr` workspace.
**Baseline:** `f897464 girsync: fold the refactor plan into the docs that describe the tool`.

---

## 0. The turn-around rule — read this first

**This applies to every commit in this plan. There is no exception.**

After a commit's work is done and its verification commands all pass:

1. `git add` the files that commit touched. **Only those.**
2. `git commit` with the exact message given in that commit's section.
3. Print the review report using the template in §0.2.
4. **STOP — end your turn.**

"STOP" means end the turn. Do not begin the next commit. Do not stage anything
further. Do not run `git commit` a second time. Do not "just quickly" start the
next one. Wait for a human reply.

**What unblocks you:** any reply that is not a redirect. "continue", "approved",
"looks good", "ship it", a question that does not change the code — all mean go
ahead with the next commit.

**What does not unblock you:** anything that questions a decision, asks for a
different approach, or asks for a change. Act on it, re-run the verification,
report again, stop again.

**The one case where you do not stop at all:** a commit's own verification fails.
Then the commit is not finished. Fix it. If you cannot fix it without changing
the design, that is a stop-and-report too — say what is blocking and why.

### 0.1 Ground rules

- **Never `git push`.** This plan never leaves the local branch.
- **Never `git commit --amend`, rebase, or reset.** Each commit stands alone so it
  can be reviewed and reverted alone. If commit 4 turns out to be wrong, the fix
  is a *later* commit that undoes it, not a rewrite of history.
- **One commit per section.** If a section cannot be done without touching the
  next section's files, stop and report that. Do not merge two sections.
- **Every commit leaves the crate compiling**, including all test targets. This
  type change is public, so a half-migrated intermediate state is a wall of
  errors rather than a small diff. When a signature has to move ahead of the
  thing that motivated it, move it — and say so in the commit body. (Commit 4
  is the worked example: `Required::of` starts returning an `AlgoSet`, so
  `hashes_differ`'s *parameter* has to change there even though its *return*
  type waits for Commit 5.)
- **Do not "improve" anything not listed.** This crate's `DESIGN-NOTES.md` has a
  section headed *Not on the table* listing changes that look like tidy-ups and
  are not. Read it before you touch anything outside the commit's file list.
- **Do not fix unrelated failures.** If the test suite was already failing before
  your commit, say so in the report and carry on. If it was green and your commit
  turns it red, that is yours and it is not finished.

### 0.2 Review report template

```
### Review report — Commit <n>: <subject>
Files: <n files> (+<x> −<y>)
Commit: <short sha>
Verification:
  cargo fmt --all -- --check ............ PASS
  cargo clippy -p girsync --all-targets ... PASS
  cargo test -p girsync .................. PASS (287 passed / 0 failed / 1 ignored)
User-visible behaviour change: <yes: what, or "none">
Deliberately NOT done (next commit):
  - <bullet>
STOPPING — awaiting review.
```

A count other than 287/0/1 is either a regression you introduced or a test that
moved. Both go in the report; neither goes unmentioned.

---

## 1. Orientation

### 1.1 The crate

`girsync` is a dev-only one-way folder mirror with a redb hash cache. Three
subcommands (`update`, `compare`, `compare-self`) plus `sync`, and a one-shot
`sled2redb` converter.

```
tools/sync/
  src/
    hash.rs             <- CHUNK, parse_hash_list/parse_any_of, Digests, hash_file
    cli.rs              <- flag surface (--hash-all-of / --hash-any-of)
    config/common.rs    <- CommonOpts (the resolved per-run options)
    cache/{schema,codec,db,write,open}.rs   <- FileRec, the binary codec, redb
    effective/{entry,phase_a,phase_c,record,side}.rs  <- phases A and C
    planner/{plan,pairs,coverage,stat_trust}.rs        <- what to hash, and whether it can be
    diff/{mod,verdict}.rs                             <- pair verdicts, --why tags
    commands/                                           <- the four subcommands
    report/mod.rs                                       <- stdout records
    convert.rs                                          <- sled -> redb
  tests/               <- integration suite, one dir per concern
  README.md            <- the tool: what a flag means, what a run prints
  DESIGN-NOTES.md       <- invariants a reader must know BEFORE changing something
```

### 1.2 Read these first

- **`tools/sync/DESIGN-NOTES.md`, all of it.** It is short and it is the
  difference between a change that works and one that silently breaks. In
  particular: the four *load-bearing invariants*, the *Not on the table* list,
  and *Known limits, deliberately not fixed*.
- **`tools/sync/README.md`**, the sections on `--hash-all-of` / `--hash-any-of`,
  `--why`, and *Cache is a cache, not truth*.

Neither file needs editing except where this plan says so. If you find yourself
wanting to reword either mid-change, stop and report instead.

### 1.3 What "a hash method" is today

One registry and one string type wearing four hats.

```rust
// src/hash.rs:9
const SUPPORTED_HASHES: &[&str] = &["md5", "sha256"];
```

| role | type | where |
| --- | --- | --- |
| the **request** | `Vec<String>` | `CommonOpts.algos` — `src/config/common.rs:59` |
| the **per-path plan** | `HashMap<String, Vec<String>>` | `HashPlan.by_rel` — `src/planner/plan.rs:60` |
| the **per-pair requirement** | `HashMap<String, Vec<String>>` + `Vec<String>` | `Required.by_rel` / `.fallback` — `src/planner/plan.rs:29-31` |
| the **stored digests** | `HashMap<String, Vec<u8>>` | `FileRec.hashes` — `src/cache/schema.rs:33`; `SideEntry.cached` — `src/effective/entry.rs:205`; `EffRec.hashes` — `src/effective/entry.rs:118` |

Derived: `Verdict::Differs { algos }` / `Matches { algos }` — `src/diff/verdict.rs:36,42`;
`Coverage::missing` / `.coverage` — `src/planner/coverage.rs:39,41`; the `algos`
array on `update`'s report — `src/report/mod.rs:245`.

### 1.4 Why this is worth doing

The cost is per **path**, and there are as many paths as files in the tree.

`HashPlan.by_rel` holds one `Vec<String>` per file the run will read.
`Required.by_rel` holds one per undecided pair — and under `--hash-all-of`,
**every one of them is `src.algos.to_vec()`** (`src/planner/pairs.rs:317-319`):
a fresh clone of the run-wide request, per path, all of them identical.

The digest map is worse, and `src/rw/limits.rs:21-23` already names it:
*"One `HashMap<String, Vec<u8>>` per pending file is real overhead on a large
tree — roughly 25–50 MB of map structure at 200k files."* That map is allocated
once per row (**5 allocations**: the table, two `String` keys, two `Vec<u8>`
digests) and **held twice** during phase C, because
`src/effective/phase_c.rs:199` clones `e.cached` into `EffRec.hashes` for every
path the plan did not touch.

`src/hash.rs:74-77` already shows the shape that fixes it — `Digests` is two
inline `Option`s, not a map. This plan extends that shape to what it *produces*.

---

## 2. The shape of the change

**Three types, split by domain — not by representation.**

| type | domain | size | why |
| --- | --- | --- | --- |
| `HashAlg` | what a run may **ask for** | 1 byte | a closed enum; the parser already rejects everything else |
| `AlgoSet` | per-path and per-pair **answers** | 2 bytes | ~100 B and 3 allocations → 2 B and 0, once per path |
| `DigestSet` | what a cache **row** carries | 64 B | 5 allocations → 0, and it is held twice |

Three facts that decide the design. Each is load-bearing; do not trade it away.

**(a) The codec keeps algorithm names inline — as strings.**
`src/cache/codec.rs:3` says *"no fixed id registry for algorithm names — so the
bytes are small, deterministic, and forward compatible"*, and lines 12-14 say why:
a cache written with a new algorithm stays readable by older binaries. **Putting
the enum in the codec is not a refactor, it is a withdrawal of that guarantee**,
and it would fail in the one place nothing else would catch it: a cache written
by a newer build, read by this one.

**(b) There is already a closed registry in this codec, and it is not the same
kind of thing.** `FileRec.kind` is a `String` in memory and a two-value byte
registry in the codec, and `kind_from_byte` **bails on anything else**
(`src/cache/codec.rs:34-40`). That is allowed for `kind` because a third kind
*would* break old binaries. Algorithms are names precisely because it must not.
Do not copy `kind`'s treatment onto algorithms.

**(c) Unknown digests must survive a whole scan, not just a codec roundtrip.**
`tests/convert.rs` pins this: a converted record carries a `blake3` row that
must "survive untouched" and must not affect a diff that never asks for it. The
path is `tests/convert.rs:76` → `src/convert.rs:92` → `encode_rec`/`decode_rec`,
and then every scan: phase A carries it (`src/effective/phase_a.rs:183`),
`merge_row` merges it (`src/effective/entry.rs:170`), phase C writes it back
(`src/effective/phase_c.rs:143`). So `DigestSet` needs a foreign arm that is
**cloned, carried and written back**, not merely one that decodes.

### 2.1 Order: three sites are observable, two structures already normalise

| site | order today | observable? |
| --- | --- | --- |
| `hash::parse_hash_list` — `src/hash.rs:22` | input order kept | feeds the two below |
| `plan_pairs`, `AllOf` — `src/planner/pairs.rs:317` | request order into `required.by_rel` | feeds the diff |
| `diff::hashes_differ` — `src/diff/mod.rs:342` | iterates `Required`'s order | feeds the tag |
| `Verdict::why_tag` → `join(algos)` — `src/diff/verdict.rs:87` | `md5+sha256` | **yes — user-visible, pinned** |
| `pick_one` — `src/planner/pairs.rs:390` | `min_by_key` keeps the **first** minimum | **yes — user-visible, pinned** |
| `report::update_summary` — `src/report/mod.rs:245` | request order | **yes — user-visible** |
| `codec::encode_rec` — `src/cache/codec.rs:51` | **sorted** names, deliberately | no — already canonical |
| `Coverage` `missing` / `coverage` | `BTreeMap` ⇒ sorted | no — already canonical |
| `HashPlan::pending` — `src/planner/plan.rs:76` | explicitly sorted | no |

- **`pick_one` cannot be served by a bitfield.** Its tie-break is the user's
  flag order, pinned by
  `tests/hash_mode/planner.rs::any_of_breaks_a_tie_by_the_flag_order`, which
  asserts that `&["sha256", "md5"]` reverses the choice. **So the request stays
  an ordered `Vec<HashAlg>`.** It is one value per run, so it costs nothing; the
  200 000-instance problem is solved by the bitfield *below* it. This is the
  central trick of the whole plan.
- **The `--why` tag becomes canonical order, and that is the one deliberate
  behaviour change.** Today `--hash-all-of sha256 md5` on a disagreeing pair
  produces `digest-differs:sha256+md5`; after Commit 5 it produces
  `digest-differs:md5+sha256`. This is an improvement — the tag becomes a
  function of *the disagreement* rather than of *how the user ordered two flags*,
  which is what `src/cli.rs:101-103` says a parser needs — and it leaves
  `tests/why/table.rs::a_tag_names_the_algorithms_that_disagreed_in_the_order_they_were_asked_for`
  passing unchanged. It ships in its own commit with a README note.

### 2.2 Reference implementations

These are what the types should look like. They are not finished code — the doc
comments and edge cases are yours to write, and they are the point — but the
signatures are the contract every later commit is written against.

#### `HashAlg` (Commit 2)

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum HashAlg {
    Md5,
    Sha256,
}

impl HashAlg {
    /// Every algorithm, in canonical order.
    pub const ALL: [HashAlg; 2] = [HashAlg::Md5, HashAlg::Sha256];

    pub const fn as_str(self) -> &'static str { /* "md5" | "sha256" */ }

    /// Raw digest bytes — never hex. See `FileRec::hashes`.
    pub const fn digest_len(self) -> usize { /* 16 | 32 */ }

    /// The bit this algorithm occupies in an `AlgoSet`.
    const fn bit(self) -> u16 { 1u16 << (self as u8) }

    /// `md5,sha256` — for the existing `parse_hash_list` error message.
    pub fn names() -> String { HashAlg::ALL.map(|a| a.as_str()).join(",") }
}

impl std::fmt::Debug for HashAlg { /* as_str(), quoted — see the note below */ }
impl std::fmt::Display for HashAlg { /* as_str */ }
impl std::str::FromStr for HashAlg { type Err = UnknownHash; /* lowercasing caller */ }
```

`Debug` **must** render `"md5"` — the command-line spelling, quoted. Not the
derive. `info!(algos = ?algos)` appears in the span of all four subcommands and
prints today as `["md5", "sha256"]`; a derive would silently rewrite every log
line in the tool to `[Md5, Sha256]` and quietly break anyone grepping a log.

#### `AlgoSet` (Commit 2)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct AlgoSet(u16);   // bit i == HashAlg::ALL[i]

impl AlgoSet {
    pub const NONE: AlgoSet = AlgoSet(0);

    /// A one-algorithm set. `HashMode::AnyOf` settles every pair with exactly
    /// one, so this is what the planner's pick becomes.
    pub const fn one(a: HashAlg) -> AlgoSet;

    pub const fn is_empty(self) -> bool;
    pub const fn len(self) -> usize;           // count_ones
    pub fn insert(&mut self, a: HashAlg);
    pub const fn contains(self, a: HashAlg) -> bool;
    pub const fn intersects(self, other: AlgoSet) -> bool;

    /// `self` minus `other`: what this side still owes.
    pub fn difference(self, other: AlgoSet) -> AlgoSet;
    /// Canonical order — `HashAlg::ALL` order, always.
    pub fn iter(self) -> impl Iterator<Item = HashAlg>;
}

impl FromIterator<HashAlg> for AlgoSet { /* fold insert */ }
impl std::fmt::Debug for AlgoSet { /* ["md5", "sha256"] — matches today */ }
```

**No `Display` on `AlgoSet`, deliberately.** Three call sites need three
different separators — `md5+sha256` in `why_tag`, `md5, sha256` in
`Coverage::missing`, `md5 sha256` in `Coverage`'s remedy — so callers iterate
`set.iter().map(HashAlg::as_str)` and join with what they need. Putting one
separator on the type would force the other two to split it back apart.

#### `DigestSet` (Commit 7)

```rust
#[derive(Clone, Default, PartialEq, Eq)]
pub struct DigestSet {
    md5: Option<[u8; 16]>,
    sha256: Option<[u8; 32]>,
    /// Algorithms this build does not know, carried verbatim. Empty in
    /// practice. See §2(c).
    foreign: Box<[(Box<str>, Vec<u8>)]>,
}

impl DigestSet {
    pub const fn len(self) -> usize;
    pub const fn is_empty(self) -> bool;

    /// The digest for a known algorithm.
    pub fn get(&self, a: HashAlg) -> Option<&[u8]>;

    /// The digest under *any* name, known or not. This is what
    /// `hashes_differ` and availability tests need.
    pub fn get_named(&self, name: &str) -> Option<&[u8]>;

    /// Known algorithms in canonical order. Foreign ones are not included;
    /// `foreign_iter` is separate and a caller that wants them asks.
    pub fn iter(&self) -> impl Iterator<Item = (HashAlg, &[u8])>;

    /// `self`'s entries win over `other`'s — this run's computed digests
    /// replace the prior row's, everything else rides along.
    pub fn merge(&mut self, other: &DigestSet);
}
```

`Box<[...]>` not `Vec<...>`: one pointer, and an empty slice allocates nothing.
So a normal row is **~72 bytes and zero allocations**, against ~250 bytes and
five. The size is derived, not guessed: 16 for the `Box` fat pointer, plus
`Option<[u8; 16]>` and `Option<[u8; 32]>` at 17 and 33 — an `Option` over an
array has no niche to spare, so it is the payload plus a discriminant byte.
Commit 7 pins the real number with a test rather than trusting this
arithmetic; if the test disagrees, work out why before pinning it.

**`DigestSet` is not `Digests`.** `src/hash.rs:74` is the streaming *context*
type that absorbs bytes; `DigestSet` is the *result*. Two different jobs; do not
merge them.

### 2.3 The one open question, answered

`parse_hash_list` uses `Vec::dedup`, which removes **consecutive** duplicates
only. `--hash-all-of md5 --hash-all-of sha256 --hash-all-of md5` parses as three
algorithms, which reaches `HashPlan::digest_count()` (three digests promised)
and a `digest-differs:md5+sha256+md5` tag. Nothing tests it.

**A set is the honest model for "which algorithms", so fix it — and fix it
first, on its own, before the type changes.** That is Commit 1.

---

## 3. Verification, for every commit

From the workspace root (`E:\hoccode\hocrust\girpr`):

```powershell
cargo fmt --all
cargo fmt --all -- --check
cargo clippy -p girsync --all-targets
cargo test  -p girsync
```

`cargo clippy -p girsync --all-targets` is **clean at HEAD** — no warnings, exit
0. That is the bar; do not raise it mid-plan and do not lower it to get green.

`cargo test -p girsync` is the whole suite. **Verified green at the baseline
commit: 287 passed, 0 failed, 1 ignored, across 18 test binaries.** That is the
number every commit is measured against.

| target | tests |
| --- | --- |
| `src/lib.rs` unit tests | 91 |
| `src/main.rs`, `src/bin/sled2redb.rs` | 0 |
| `tests/sync.rs` | 13 + 1 ignored |
| `tests/lazy/main.rs` | 27 |
| `tests/why/main.rs` | 24 |
| `tests/stat_trust/main.rs` | 19 |
| `tests/sync_plan/main.rs` | 19 |
| `tests/hash_mode/main.rs` | 18 |
| `tests/rw_threads/main.rs` | 15 |
| `tests/compare.rs`, `tests/compare_self.rs` | 12 each |
| `tests/migration_lock.rs` | 11 |
| `tests/helpers.rs` | 10 |
| `tests/output.rs` | 8 |
| `tests/convert.rs` | 4 |
| `tests/cli_dispatch.rs` | 1 |
| doc-tests | 0 |

The one ignored test is **not** an accident and **not** yours to un-ignore:
`tests/sync.rs:555`, `#[ignore = "known: the derived present set keeps rows for a
dir created and rmdir'd in the same run"]`. It is the documented known limit in
`DESIGN-NOTES.md` § *Known limits, deliberately not fixed*. If it starts
running and passing, that is a behaviour change, not a fix — say so.

Most of the suite is real filesystem work; a full run takes roughly four
minutes. Run it **in full** before each review report. A filtered run is not
verification: several suites share the on-disk cache format and only the whole
run proves nothing about the codec drifted.

---

## 4. Commits

---

### Commit 1 — `girsync: dedupe repeated --hash values, keeping the order they were typed`

**Why it is separate.** It is a behaviour fix, not a refactor, and it must land
before the types change so that the type change is not carrying a behaviour
change with it. It also fixes `parse_any_of`, which delegates.

**Files:** `src/hash.rs` only.

**Change.** In `parse_hash_list` (`src/hash.rs:22`), replace the trailing
`v.dedup();` — which drops only *adjacent* repeats — with a full dedup that keeps
first-seen order:

```rust
    // Full dedup in first-seen order, not `Vec::dedup`'s adjacent-only pass:
    // `--hash-all-of md5 --hash-all-of sha256 --hash-all-of md5` parsed as three
    // algorithms, which reached `HashPlan::digest_count` (three digests
    // promised) and a `digest-differs:md5+sha256+md5` reason tag. Order is the
    // user's, because `pick_one`'s tie-break is defined in terms of it.
    let mut seen: Vec<String> = Vec::with_capacity(v.len());
    for a in v {
        if !seen.contains(&a) {
            seen.push(a);
        }
    }
    Ok(seen)
```

Two `Vec::contains` scans over a list bounded by the argv length. Fine; say so
in the comment if you like, do not switch to a `HashSet` for it.

**Do not touch** the `none` handling, the empty-string `retain`, the validation
loop, or any `bail!` message. The asymmetry between `parse_hash_list` and
`parse_any_of` — `none` is a mode under one and not a value under the other — is
argued in `parse_hash_list`'s and `parse_any_of`'s doc comments and is load
bearing.

**Tests to add**, in `src/hash.rs`'s existing `#[cfg(test)] mod tests`:

- A repeated algorithm is one request, however it is typed:
  `parse_hash_list(&["md5","sha256","md5"])` → `["md5","sha256"]`, and the
  non-adjacent case first, because that is the one that used to fail.
- First-seen order is the order kept: `["sha256","md5","sha256"]` →
  `["sha256","md5"]`. This is the property `pick_one` depends on — say so in the
  test's doc comment.
- Adjacent repeats still collapse (the case `Vec::dedup` already handled), so
  the old behaviour stays pinned.
- `parse_any_of` inherits it: `parse_any_of(&["md5","md5"])` → `["md5"]`.

**Not done here:** anything about `HashAlg`. That is Commit 2.

---

### Commit 2 — `girsync: HashAlg and AlgoSet, before anything uses them`

**Why it is separate.** Purely additive: no caller changes, no behaviour change,
zero risk. The reviewer only has to judge two new types.

**Files:** `src/hash.rs` only.

**Change.** Add the two types exactly as specified in §2.2, in `src/hash.rs`,
below `SUPPORTED_HASHES` / above `parse_hash_list`. Keep `SUPPORTED_HASHES` — it
is still what `parse_hash_list` validates against until Commit 3.

Doc comments are the deliverable here. Each must answer: what is it for, why is
it a set rather than a list, what does it cost, and what does it *not* cover?
Specifically:

- `HashAlg` must say that it is **not** what a cache row stores, and point at
  `crate::cache::codec` for why.
- `AlgoSet` must say that it has no order of its own, that iteration is canonical,
  and that the one order-sensitive consumer — `pick_one` — reads the request
  (`&[HashAlg]`) instead. This is the note that stops the next reader from
  "simplifying" the request into a set.
- `UnknownHash` (the `FromStr` error) needs a `Display` that is not the same
  string twice.

**Every method must have a caller by the end of Commit 8.** `DESIGN-NOTES.md`'s
*Not on the table* section exists because speculative helpers get added and then
justified after the fact. If you find yourself adding a method "for later", stop
and report it instead — the list in §2.2 is already the complete set, and if you
find it is not, that is a finding about this plan worth surfacing before you
write any code.

**Tests to add**, in `src/hash.rs`'s `mod tests`:

- `FromStr` round-trips every name in `HashAlg::ALL`, and is **case-insensitive**
  on its own: `"MD5".parse()` must yield `HashAlg::Md5`. `--hash MD5` works today
  because `parse_hash_list` lowercases before validating, but making `FromStr`
  self-contained means no caller has to remember to. Commit 3 keeps
  `parse_hash_list`'s own `.to_lowercase()` for the `none` comparison, which
  happens before any `FromStr` call.
- `as_str().parse()` is the identity over `ALL`.
- `AlgoSet::iter()` yields `HashAlg::ALL` order for every subset — this is the
  canonical-order claim, and it is what Commit 5 depends on.
- `len() == iter().count()` for every subset.
- `one(a)` has `len() == 1` and contains `a`.
- `difference` is the identity against `NONE` and `NONE` against itself.
- `Debug` renders `["md5", "sha256"]` — pin it, because it is the log-line
  compatibility this whole type exists to preserve.

**Not done here:** the request type. `CommonOpts.algos` is still `Vec<String>`.

---

### Commit 3 — `girsync: the request is a list of algorithms, not of their names`

**Why it is separate.** The first commit anything outside `src/hash.rs` sees.

**Files:**
- `src/hash.rs` — `parse_hash_list` / `parse_any_of` now return `Vec<HashAlg>`;
  `SUPPORTED_HASHES` **deleted**; validation goes through `FromStr`; the error
  message keeps its exact shape (`unsupported hash '{}' (supported: {} or none)`,
  with `HashAlg::names()` in place of `SUPPORTED_HASHES.join(",")`).
- `src/config/common.rs` — `CommonOpts.algos: Vec<HashAlg>`.
- `src/report/mod.rs` — `update_summary(algos: &[HashAlg])`.
- Test fixtures (see the list below).

**Do not touch:** the `--hash-any-of needs at least one algorithm` message and
the argument in `parse_any_of`'s doc about why `none` is not a value there. The
`none` handling still returns an **empty `Vec`**, not a variant — `--hash-all-of
none` stays "no algorithms requested", which is what `HashPlan::plan_one_side`
and `plan_pairs` test with `&[]`.

**Call sites to follow.** `CommonOpts.algos` is read in
`src/commands/{update,compare,compare_self}.rs`, `src/commands/sync/mod.rs`,
`src/effective/phase_a.rs:67`, `src/effective/phase_c.rs` (via the plan),
`src/effective/record.rs`, and the `info!(algos = ?algos)` spans in all four
subcommands. Most of those pass it straight through and will keep compiling.

**Test fixtures to update** — mechanical, `String` → `HashAlg`:

| file | what |
| --- | --- |
| `tests/helpers.rs:78` | `== vec!["md5".to_string()]` → `== vec![HashAlg::Md5]` (the other three assertions in that test call `.is_empty()`/`.is_err()` and do not care) |
| `tests/helpers.rs:149,220` | `cold_then_warm(tag, algos: Vec<String>)` → `Vec<HashAlg>` |
| `tests/common/mod.rs:115,414-416` | `opts()` and `with_algos(&[&str])` — keep taking `&[&str]` at the boundary and map to `HashAlg` inside, so call sites like `with_algos(&[])` and `with_algos(&["md5","sha256"])` do not all change |
| `tests/hash_mode/main.rs:109-113,160-162` | same: keep `&[&str]`, map inside |
| `tests/stat_trust/main.rs:109-111` | same |
| `tests/why/main.rs:88,96,104` | `algos: vec![...]` literals → `vec![HashAlg::Md5]` |
| `tests/why/{table,identical,flags,disagreement}.rs` | `algos: vec!["md5".into()]` → `vec![HashAlg::Md5]` |
| `tests/compare_self.rs:253,422` | `algos: vec![]` — unchanged |
| `tests/migration_lock.rs:33,200` | `with_algos(dir, algos: Vec<String>)` → `Vec<HashAlg>` |
| `src/report/mod.rs:264` | the `update_summary` unit test's `&["md5".to_string()]` |
| `src/planner/fixtures.rs:131` | the `&'static [&'static str]` → `Vec<HashAlg>` conversion inside `Shape::run` |

`tests/why/main.rs:104` (`any_of_md5_sha256`) is the one to be careful with: it
builds `CommonOpts` by hand to set `hash_mode`, and its `algos` order is
meaningful to `pick_one`.

**Not done here:** the plans. `HashPlan.by_rel` and `Required.by_rel` still hold
`Vec<String>`. This commit compiles because nothing reads the element type yet
except `hash.rs` itself and `update_summary`.

---

### Commit 4 — `girsync: bitfield the per-path and per-pair algorithm sets`

**Why it is separate.** This is where the memory win lands, and where
`pick_one`'s order-sensitivity is at risk. It is one commit because splitting it
leaves a state where the request is a `Vec<HashAlg>` and the plans are
`AlgoSet`, with the tie-break half-rewritten.

**Files:**
- `src/planner/plan.rs` — `HashPlan.by_rel: HashMap<String, AlgoSet>`,
  `Required.by_rel: HashMap<String, AlgoSet>`, `Required.fallback: AlgoSet`,
  `HashPlan::get -> AlgoSet`, `HashPlan::missing(entry, algos: &[HashAlg], no_trust) -> AlgoSet`,
  `HashPlan::digest_count`, `plan_one(..., algos: AlgoSet, ...)`.
- `src/planner/pairs.rs` — `plan.required.fallback = AlgoSet::from_iter(src.algos.iter().copied())`;
  the `AllOf` arm inserts `AlgoSet::from_iter(src.algos.iter().copied())`;
  the `AnyOf` arm uses `let chosen = AlgoSet::one(pick);` in place of
  `let chosen = [pick]; ... chosen.to_vec()`;
  `obtainable(e, cap, a: HashAlg)`; `Tally.coverage` keys.
- `src/diff/mod.rs` — **signature only.** `hashes_differ(a, b, algos: AlgoSet)`
  so `pair_verdict` can hand it `required.of(key)`. Its return stays
  `Vec<String>` for this commit (`need.iter().map(|a| a.as_str().to_string()).collect()`);
  Commit 5 changes the return type. **Without this the crate does not compile**,
  because `Required::of` stops returning `&[String]`.
- `src/effective/phase_c.rs` — `plan.get(rel)` is now an `AlgoSet`; the
  `debug!(algos = ?algos)` field still works. The
  `trace!(algos = ?r.as_ref().map(|h| h.keys().collect::<Vec<_>>()))` field reads
  a `HashMap` and is unaffected until Commit 8.
- `src/hash.rs` — `Digests::new(algos: &AlgoSet)` and
  `hash_file(path, algos: &AlgoSet)`, plus their unit-test fixtures.
- `src/commands/sync/apply.rs` — `copy_and_digest`, `copy_one` and `hash_file`
  calls now build `AlgoSet::from_iter(algos.iter().copied())` from the request.
  `verify_digests` still takes `&[HashAlg]` and still walks the request, because
  only the *requested* digests are judged — do not convert it to a set here.
- Test fixtures in `src/planner/{plan.rs,pairs.rs,coverage.rs,fixtures.rs}`,
  `tests/hash_mode/main.rs`, `tests/why/*`.

**The load-bearing detail.** `HashPlan::missing` today is
`algos.iter().filter(|a| no_trust || !entry.cached.contains_key(*a)).cloned().collect()`.
It becomes set arithmetic — *everything requested, minus what the row already
holds* — and **it must still yield canonical order**, because `AlgoSet` has no
order of its own and everything downstream now assumes canonical.

Note what this does to `HashPlan::missing`'s doc: it currently promises *"in
`algos` order"*, which was load-bearing only because `Required` copied the
request order through. After this commit, `Required`'s entries are canonical
too, and the promise moves to `AlgoSet::iter`.

**The one thing that must not change.** `pick_one` (`src/planner/pairs.rs:390`)
keeps iterating `algos: &[HashAlg]` — the *request* — and keeps `min_by_key`,
which keeps the **first** minimum and therefore the **user's** order. It returns
`Option<HashAlg>` now, not `Option<String>`; everything else about it is
untouched. `any_of_breaks_a_tie_by_the_flag_order` in
`tests/hash_mode/planner.rs` is the guard and it must still pass **unchanged**.

**Not done here:** `Verdict` and the tag. `hashes_differ` now takes an `AlgoSet`
but still returns `Vec<String>`, so the reason tag is still built in request
order — which is what makes Commit 5 the single commit that changes what a user
sees.

---

### Commit 5 — `girsync: name the algorithms that disagreed in a fixed order`

**Why it is separate.** **This is the only commit that changes user-visible
output.** It stands alone so it can be reverted alone.

**Files:**
- `src/diff/mod.rs` — `hashes_differ`'s **return type** becomes `AlgoSet`;
  iterate `algos.iter()`. Lookups are still `a.get(algo.as_str())` until
  Commit 8 replaces the maps with `DigestSet`.
- `src/diff/verdict.rs` — `Differs { algos: AlgoSet }`, `Matches { algos: AlgoSet }`;
  `why_tag` joins `algos.iter().map(HashAlg::as_str)` with `+`.
- `src/diff/mod.rs` — `pair_verdict`'s `Verdict::Matches { algos: need }` arm no
  longer calls `.to_vec()`.
- `README.md` — one sentence, in the `--why` section, next to the
  `digest-differs:<algos>` row of the tag table: the algorithm list is in a fixed
  order, not the order the flags were typed. Say what changed and why it is
  better (a parser can switch on it).
- Test fixtures in `tests/why/*` that assert on `Verdict` values.

**The behaviour change, stated precisely.** `--hash-all-of sha256 md5` on a
disagreeing pair produces `digest-differs:md5+sha256` after this commit, where
it produced `digest-differs:sha256+md5` before. Put this in the review report's
"user-visible behaviour change" field **and** in the commit body.

**Do not change:** which algorithms are named. `digest-differs` names the ones
that **disagreed**, not the ones requested — `src/diff/mod.rs:328-341` and
`src/diff/verdict.rs:30-37` both argue this, and the two guards are
`tests/why/disagreement.rs::a_tag_names_only_the_algorithms_that_were_actually_compared`
and `::a_skipped_algorithm_is_never_reported_as_a_disagreement`.

**The other behaviour to not change:** `hashes_differ` stays **silent** when
either side lacks a digest. `DESIGN-NOTES.md` (*Do not weaken `plan_pairs`*
coverage check or `pair_verdict`'s `unverifiable` clause*) explains why: that
silence is safe only because the planner chose these algorithms, and coverage is
what bounds it. A `DigestSet::get` that returned a zero digest instead of `None`
would silently turn a false clean back on.

`Verdict::why_tag`'s `join` currently has
`debug_assert!(!algos.is_empty(), "a reason detail must name something")`. Keep
that assertion, or its `AlgoSet` equivalent — an empty set in a tag is a
malformed tag, and both variants that carry one can always be non-empty.

**Not done here:** `Coverage`, which still renders algorithm names from
`BTreeMap<String, _>`.

---

### Commit 6 — `girsync: coverage counts and shortfalls in algorithm terms`

**Why it is separate.** Mechanical, and it only exists on the error path, so it
is the cheapest commit in the plan to review.

**Files:**
- `src/planner/coverage.rs` — `Coverage.missing: BTreeMap<String, AlgoSet>`,
  `Coverage.coverage: BTreeMap<HashAlg, (usize, usize)>`.
- `src/planner/pairs.rs` — `Tally` to match.

**The message must not change.** `Coverage`'s `Display` is the part a user acts
on, and `src/planner/coverage.rs:160-172` says so. Every rendered message must
come out byte-identical:

- `algos.join(", ")` — iterate `set.iter().map(HashAlg::as_str)` and join with
  `", "`. That is why `AlgoSet` has no `Display`.
- `{algo:<12}` — `Display for HashAlg` covers it.
- `self.coverage.keys().cloned().collect::<Vec<_>>().join(" ")` in the two
  remedies — iterate `coverage.keys()` and join `HashAlg::as_str` with `" "`.

**Key order must not change.** `coverage` was a `BTreeMap<String, _>`, i.e.
sorted by name; `HashAlg`'s canonical order is `Md5, Sha256`, which is also
alphabetical for the two that exist. They agree **today**. Say so in a comment
where the `BTreeMap` is declared, because it stops being true the day someone
adds a third algorithm — and at that point the remedy text silently reorders.

The tests in `src/planner/coverage.rs`'s module assert on the *message*, so they
are the guard. `a_record_missing_a_requested_algorithm_fails_the_run` checks for
`"md5"` and `"1/1"`; all of them must pass unchanged.

---

### Commit 7 — `girsync: DigestSet, so a cache row costs no allocations`

**Why it is separate.** Additive again. `DigestSet` exists, is fully tested, and
nothing uses it — so the reviewer can judge the hardest type in this plan before
any existing behaviour is put at risk.

**Files:** `src/hash.rs` only.

**Change.** Add `DigestSet` and `foreign_iter()` exactly as specified in §2.2,
placed next to `Digests` with a doc comment saying plainly that they are
different jobs and must not be merged.

`Foreign` entries are `Vec<u8>`, not fixed arrays, because **a known name with an
unexpected digest length has to go there**: a corrupt or hand-written row that
says `md5` with 8 bytes must still decode, still compare byte-wise, and still
report a difference — which is what happens today, where it lands in a
`HashMap<String, Vec<u8>>` and `x != y` compares two unequal `Vec`s. Routing it
to `foreign` preserves that exactly. Write the test.

**Tests to add** (in `src/hash.rs`'s `mod tests`):

- `size_of::<DigestSet>()`. Pin the number, whatever the test says it is. This
  is the whole memory claim expressed as an assertion: if a later edit adds a
  `HashMap` or a `Vec` field the test fails and forces the decision to be
  deliberate rather than accidental. The derivation is in §2.2 (~72: a `Box` fat
  pointer plus two `Option`s over arrays, which have no niche and so cost a
  discriminant byte each). If the test prints something else, find out why
  before you pin it — do not just paste the number.
- Round-trip for every subset of `{Md5, Sha256}`.
- `merge`: the computed digest wins, the unrequested one rides along, and
  `foreign` entries ride along.
- `get_named("blake3")` finds a foreign entry; `get_named("md5")` does not find
  it and `get(HashAlg::Md5)` does not either.
- A known name with a wrong digest length routes to `foreign` and survives
  round-trip unchanged.
- `is_empty()` for `DigestSet::default()`.

**Not done here:** any caller. `FileRec.hashes` is still
`HashMap<String, Vec<u8>>`.

---

### Commit 8 — `girsync: carry digests in DigestSet end to end`

**Why it is separate.** This is the memory win, and it is also the largest and
riskiest commit in the plan. It stands alone because everything else in the plan
can be reverted without touching the persistence layer.

**Files** (this one is large — the type change is public and the fixture updates
are mechanical):

- `src/cache/schema.rs` — `FileRec.hashes: DigestSet`.
- `src/cache/codec.rs` — `encode_rec` / `decode_rec` speak `DigestSet`. **This is
  the file the forward-compat guarantee rides on.**
- `src/convert.rs` — build a `DigestSet`; unknown sled algorithm names go to
  `foreign`; `Converted.hashes` counts `DigestSet::len()`.
- `src/hash.rs` — `Digests::finish() -> DigestSet`; `hash_file -> DigestSet`.
- `src/effective/entry.rs` — `EffRec.hashes`, `SideEntry.cached`,
  `merge_row(computed: DigestSet, prior: Option<&FileRec>) -> FileRec`,
  `SideEntry::prior()`, and the `digests()` / `algos_of()` test helpers.
- `src/effective/phase_a.rs` — `c.hashes.clone()` → a `DigestSet`.
- `src/effective/phase_c.rs` — `computed: Vec<Result<DigestSet>>`,
  `hashes: e.cached.clone()`.
- `src/effective/record.rs` — `cached: r.hashes` (moves; becomes a conversion).
- `src/effective/mod.rs` re-exports, if the type list there changes.
- `src/diff/mod.rs` — `hashes_differ(a: &DigestSet, b: &DigestSet, ...)`.
- `src/commands/sync/apply.rs` — `copy_and_digest`, `verify_digests`,
  `copy_one`, and the `digests()` test helper.
- `src/commands/sync/rename.rs` — the `HashMap::new()` it builds.
- `src/cache/{db,write}.rs` — the `FileRec` fixtures in their `mod tests`.
- Test fixtures: `tests/common/mod.rs:351-352` (`digests_of`),
  `tests/hash_mode/main.rs:136,153`, `tests/convert.rs:76,121`,
  `tests/helpers.rs`, `tests/migration_lock.rs:21-33`.

**The codec, precisely.** Keep the wire format byte-identical.

- `encode_rec`: emit `HashAlg::ALL` in canonical order (skipping absent slots),
  then `foreign` **sorted by name**. Today it sorts all names together. For a
  row holding only known algorithms the bytes are therefore **unchanged**, since
  canonical order (`md5`, `sha256`) is also alphabetical order. For a row
  holding an unknown name the order changes (`blake3` now sorts after the known
  ones rather than interleaved) — the meaning does not, and nothing compares
  encoded values for equality (`CacheWrite::put` just inserts). **Say this in a
  comment**, because "sorted for deterministic bytes" is a claim a reader will
  check.
- `decode_rec`: a known name with the expected length goes to its slot; a known
  name with any other length goes to `foreign`; **any unknown name goes to
  `foreign`**. Never `bail!` on an algorithm name. This is the whole guarantee
  and it is a one-line difference from `kind_from_byte`, which *is* allowed to
  bail (`src/cache/codec.rs:34-40`) precisely because a third kind would break
  old binaries. Leave a comment at both saying why they differ, or someone will
  "fix" the inconsistency.
- Keep the `rec.hashes.len() > 255` guard and the `name.len() > 255` /
  `digest.len() > 255` guards. The doc comment at `src/cache/codec.rs:6-10` is
  the format spec; update it if the layout text changes (it should not).

**What must not be lost** (each is load-bearing, each has a test, and this
commit is large enough to lose one):

- **`verify_digests` still takes two independent maps.** `DESIGN-NOTES.md` § *The
  verify needs two independent reads*: folding the src and dst digests into one
  value would compare `x` with `x`, and `verify hash mismatch` could never fire
  while still reading as a guarantee. `src/commands/sync/apply.rs:446-491` is the
  reasoning; the unit tests at `apply.rs:680-755` are the guard.
- **`merge_row` still drops *every* digest on a stat change**, unrequested ones
  included — and now `foreign` too. `entry.rs:372`
  (`merge_row_drops_unrequested_digests_when_the_stat_changed`) is the guard, and
  it **has no foreign-row analogue today: add one**, because the foreign arm is
  new and the rule is "a stat change makes every stored digest suspect",
  including the ones this build cannot check.
- **`a_digest_is_the_same_however_the_bytes_were_split`** (`src/hash.rs:168`) is
  the only thing holding `copy_and_digest` and `hash_file` to one definition of
  "absorb these bytes". Changing the return type of `finish()` is exactly the
  change that could let them drift.
- **`codec_preserves_unknown_algorithms`** (`src/cache/codec.rs:157`) and the
  `blake3` cases in `tests/convert.rs` — these *are* the forward-compat suite.
- **`EffRec` and `SideEntry` still derive `PartialEq`/`Eq`.** `entry.rs:105-111`
  explains why: it is how a test asserts two runs produced the *same* effective
  map, which is the strongest available statement of the dry-run contract.
- **`CacheWrite` still does not decode a row to answer "which keys exist?"** —
  `load_keys` must keep walking values without decoding.

**Not done here:** nothing. This is the last code commit.

---

### Commit 9 (optional) — `girsync: fold what the new types taught into the docs`

**Only do this if a genuine new invariant emerged** — for example, if you found
yourself wanting to write down "a known algorithm's digest length is checked at
decode and a mismatch is routed to `foreign` rather than rejected".

Per `DESIGN-NOTES.md`: *"Nothing here is a record of a change. If a statement is
about what the code **is**, it is true; if it would only be about what the code
**was**, it belongs in `git log`."* So this commit adds statements about the
present, never a changelog.

If nothing new emerged, skip it and say so in the final report.

---

## 5. Non-negotiables — check these at the end

Before declaring the plan done, confirm each of these. They are the properties
the change exists to preserve.

- [ ] `parse_hash_list` dedupes fully, in first-seen order.
- [ ] `pick_one`'s tie-break is still the user's flag order, and
      `any_of_breaks_a_tie_by_the_flag_order` passes unchanged.
- [ ] `decode_rec` never `bail!`s on an algorithm name;
      `codec_preserves_unknown_algorithms` and `tests/convert.rs`'s `blake3`
      cases pass.
- [ ] A `blake3` digest survives a **scan**, not just a codec roundtrip.
- [ ] `encode_rec` output is still deterministic, and the reason it is
      deterministic is still written down next to it.
- [ ] `verify_digests` still takes two independent reads.
- [ ] `hashes_differ` is still silent on a missing digest.
- [ ] `merge_row` still drops every digest — `foreign` included — on a stat change.
- [ ] `a_digest_is_the_same_however_the_bytes_were_split` passes.
- [ ] `plan_pairs`' coverage check and `pair_verdict`'s `unverifiable` clause are
      untouched. **A false clean is the worst outcome this tool can produce.**
- [ ] The lock ordering in `src/rw/gate.rs` was not touched.
- [ ] `cargo clippy -p girsync --all-targets` is clean; `cargo test -p girsync`
      is **287 passed / 0 failed / 1 ignored** — the same as the baseline.

---

## 6. Out of scope

Do **not** do any of these, even if they look like obvious tidying:

- Converting `FileRec.kind: String` to an enum. It is tempting and it is wrong:
  `kind` is a closed registry in the codec **by design**, and a third kind must
  be rejected so old binaries fail loudly. Different from algorithms.
- Converting `SideEntry.fresh: bool` or `SideCapability`'s two bools into
  anything.
- Replacing `RwLimits`, `StatTrust` or `CasePairing`. `DESIGN-NOTES.md` § *Not
  on the table* covers these.
- Converting `src/hash.rs` into a `hash/` directory module to give the new types
  their own file. Gratuitous churn.
- Adding `bitflags`, `smallvec` or any other dependency. `bitflags` 2.x *is*
  already in `Cargo.lock` transitively, so it would cost nothing to compile — but
  `AlgoSet` is ~50 lines of newtype, its `Debug` has to render `["md5", "sha256"]`
  anyway, and this crate hand-rolls `RwLimits`, `StatTrust` and `CasePairing` in
  preference to derives. Consistency wins.
- Any performance measurement. This plan changes allocation counts, which are
  exact and checkable by reading the types. Byte figures would need `dhat` or
  `heaptrack`, which are not dependencies here.
- Reordering, reformatting or rewording anything not in a commit's file list.
