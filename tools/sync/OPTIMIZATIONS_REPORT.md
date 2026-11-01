# girsync (`tools/sync`) — refactor, performance, and correctness review

This is a survey of opportunities, not an implementation plan. File and function
names are pointers to where a concern lives; they are not suggestions to edit a
particular range. The crate is unusually well documented, so this report tries to
distinguish **real opportunities** from **documented deliberate choices**.

The review assumes observable behaviour may change and that tests pinning it may
be updated. It still calls out where a change would be broad or would weaken a
safety property.

---

## How to read the entries

Each entry uses four compact labels:

- **Risk** — blast radius, and the chance of a silent regression.
- **Behaviour** — what a user could observe changing.
- **Gain** — what improves, and roughly how much.
- **Tradeoff** — what is given up or made harder.

A "low-risk" item is still low-risk only if its invariant is preserved; in this
crate most of the value comes from the invariants, not from the code shape.

---

## Ground rules: do not "tidy" these

`DESIGN-NOTES.md` is not decorative. Several obvious refactors are explicitly
forbidden because they remove a load-bearing property:

- Do **not** merge `scan_stat_only` and `load_record_side_from`, or make `Side` a
  trait.
- Do **not** touch the `src → dst` lock order, the `RwLimiter`, or the three-gate
  shape.
- Do **not** convert `FileRec.kind` to an enum, add `bitflags` for `AlgoSet`, or
  unify the "open algorithm vocabulary" (cache) with the "closed request
  vocabulary" (`HashAlg`).
- Do **not** collapse the copy's two independent reads into one, or make
  `sync_all` optional in production.
- Do **not** weaken `plan_pairs`' coverage check or `pair_verdict`'s
  `unverifiable` clause.
- Keep the planner's and the diff's `CasePairing` values deliberately different.
- Keep the `--hash-any-of` tie-break tied to flag order.
- Keep `verdicts` and the diff's always-on classification unless a much narrower
  change is made (see the performance section).

Where an item below touches one of these, it says so.

---

## 1. Correctness and subtle bugs

These are mostly edge cases. None of them is likely to affect an ordinary
same-version, same-platform run, but each is a place where two definitions of one
rule have drifted or could drift.

### 1.1 Mixed-case foreign algorithm names bypass the typed lookup

**Where:** `planner/plan.rs` (`HashPlan::missing`), `planner/pairs.rs`
(`obtainable`, `pick_one`, the coverage tally), and the name-keyed lookup in
`hash/digestset.rs`.

**What:** `DESIGN-NOTES.md` states that every caller asking "does this row hold
algorithm X" must use the by-name lookup. In practice the planner uses the typed
`get_algo`, which matches a foreign entry only by the algorithm's canonical
spelling. A known name stored with an unexpected length under a differently
cased spelling (for example `MD5` rather than `md5`) is therefore invisible to the
planner, while `get_named` would find it. The result is a false "must compute"
(which is benign but wasteful) or, on a record side, a false coverage refusal.

**Risk:** Low. Only hand-written, converted, or newer-build cache rows with a
unusual spelling are affected. The change is local and does not touch pair
semantics.

**Behaviour:** Fewer spurious coverage failures and fewer unnecessary rehashes
for those rows. Ordinary caches see no change.

**Gain:** Restores the documented "lookup by name" invariant and removes a
disagreement between the design notes and the code.

**Tradeoff:** The by-name route needs a string; either extend the typed lookup to
fold names consistently, or route these callers through the name-keyed one. The
first is cheaper at the call site but must be documented as a deliberate widening
of `get_algo`.

### 1.2 Two different case folds are used for the same filesystem

**Where:** `util::check_mixed_case_keys` (Unicode `to_lowercase`) versus
`util::component_eq` / `same_path` / `path_within` (ASCII fast path, Unicode
lowercase fallback) and the ad-hoc `to_lowercase` indexes in `planner/pairs.rs`,
`diff/mod.rs`, `effective/phase_a.rs`, `commands/sync/rename.rs`,
`commands/sync/plan/safety.rs`, and `commands/sync/plan/link.rs`.

**What:** On Windows, path identity and mixed-case collision detection are both
approximations of the filesystem's own upcase table. The crate currently uses at
least three spellings of the rule: ASCII-only, Unicode lowercase, and the
`to_lowercase` index key. For most names they agree; for characters where Unicode
lowercasing and the NTFS upcase table differ (dotted/dotless I, some ligatures,
case-mapping expansions, non-BMP names), they can disagree. That can produce a
false collision (a harmless-looking refusal) or a missed collision.

**Risk:** Medium if the fold is changed, because collision detection feeds
refusals and cache keys. The blast radius is limited to unusual names, but the
change touches several modules.

**Behaviour:** A different (hopefully more consistent) set of names is treated as
the same path. On POSIX this should remain exact byte equality.

**Gain:** One documented fold instead of several accidental ones; fewer
surprising refusals or missed collisions on Windows.

**Tradeoff:** A truly exact NTFS fold is not available portably. The practical
choice is to centralize the existing approximation, state its limits, and make
every index use the same helper. That is a coherence win, not a perfect one.

### 1.3 Backup and snapshot creation is unconditional and unbounded

**Where:** `commands/sync/mod.rs` (the pre-run backup step), `cache/open.rs`
(`backup_db`, `snapshot_old`, `copy_aside`), `util::unique_sibling`.

**What:** Every non-dry `sync` copies the src cache, the dst cache, and the dst
cache again as an `old-*` snapshot, before anything is read. There is no retention
policy and no flag to skip it. Each run therefore adds up to three full copies of
the cache files, forever. On a large cache this is the dominant startup cost and
an unbounded disk consumer. `unique_sibling`'s "exists, then create" pattern also
has a small race between two concurrent runs against one root.

**Risk:** Medium. The backup is a safety feature, and tests assert the names and
the "keep-all" behaviour. Removing it would be a real loss; making it
configurable changes what a run leaves behind.

**Behaviour:** A run could create fewer backups (or none) depending on a flag or
policy, and old backups could be pruned. The filesystem layout becomes smaller
and more predictable.

**Gain:** Large startup I/O reduction, bounded disk use, and fewer inode/name
churn events on network or HDD storage.

**Tradeoff:** Users lose an automatic recovery point unless they opt in. A
retention policy must decide what "old" means, and `unique_sibling` should be
made collision-safe (atomic create or rename) if concurrent runs are supported.

### 1.4 `init_meta` acquires a write transaction on the no-change path

**Where:** `cache/db/cachedb.rs` (`init_meta`).

**What:** A read/write open always begins a write transaction, inspects the meta
row, and then drops the transaction when nothing changed. Dropping avoids the
commit, but the write lock, page setup, and any redb bookkeeping have already
happened. On HDD or under concurrency this is a measurable open-time cost for the
common case where the cache is already correct.

**Risk:** Low–medium. The fresh-cache path still has to create both tables, and
the version/case mismatch paths still have to write.

**Behaviour:** No visible change; the same meta is written when it needs to be.

**Gain:** Avoids a write transaction on every normal open.

**Tradeoff:** The fast path becomes "read meta first, then begin a write only if
needed", which is two opens on the rare path. The logic must preserve the
"fresh file gets both tables" behaviour.

### 1.5 `convert` has no atomicity or cleanup on failure

**Where:** `convert.rs` and `src/bin/sled2redb.rs`.

**What:** The converter reads the whole sled DB into memory, then opens the
destination and writes rows. If the write fails partway, the destination is a
partial redb file. `--force` removes the old file before the new one is known to
be good. There is also no progress or streaming: a very large legacy cache is held
entirely in memory as decoded records.

**Risk:** Medium. This is a one-shot migration path, so the blast radius is
limited to the conversion, but a failed conversion can destroy the previous
destination under `--force`.

**Behaviour:** A failed conversion leaves either no destination or a clearly
incomplete one; a successful one is unchanged.

**Gain:** Recoverable migrations and bounded memory.

**Tradeoff:** Writing to a temporary sibling and renaming is the obvious shape,
but it interacts with the "destination next to the sled directory" rule and with
Windows sharing/locking. Streaming rows into the redb write transaction is
possible, but the sled handle must stay open until the scan is done; the current
code deliberately drops it before opening the destination.

### 1.6 Smaller correctness/robustness notes

- `util::unique_sibling` is a check-then-use; `copy_aside` then overwrites via
  `std::fs::copy`. Two simultaneous syncs can pick the same backup name. Low risk,
  easy to make atomic if backups stay.
- `open_folder_cache` re-reads `GIRSYNC_TEST_CACHE` on every cache open. This is
  test-only and cheap, but it is a second configuration source next to
  `common.test_cache`; a single resolved value would be clearer.
- `copy_one` re-checks the destination at several points. That is deliberate
  belt-and-braces, but it means the "one place that decides" story is spread
  across `validate_dst_links`, `copy_one`, `guard_removal_outside_src`, and
  `ensure_hard_linked_targets_safe`. It is not a bug; it is a place where a
  future change can easily add a fourth spelling of the rule.
- `ensure_hard_linked_targets_safe` still stats delete/fix-dir targets when
  `plan.copy` is empty. A copy-free run cannot truncate an inode, so the whole
  preflight can be skipped. This is correctness-neutral and purely wasted work.

---

## 2. Performance

The biggest costs are: walking and lowercasing the tree several times, cache
scans and per-key reads, unconditional cache backups, and per-file filesystem
round trips in the copy path. The order below is roughly gain-to-risk.

### 2.1 A shared case index instead of three or more independent joins

**Where:** `planner/pairs.rs` (`shared_pairs`), `diff/mod.rs` (`diff_maps`),
`commands/sync/rename.rs` (`case_renames`), `effective/phase_a.rs` (the alternate
key index), `commands/sync/plan/safety.rs` (`ensure_mkdir_targets_free`), and
`commands/sync/plan/link.rs` (`folded_keys`).

**What:** In insensitive mode each of these builds its own lowercased index over
one or both full trees. A sync can therefore lowercase every key several times
and retain several duplicate maps at once. Each implementation also carries its
own assumptions about collisions and about which side's spelling wins.

**Risk:** Medium. The *pairing policy* must not be unified: the planner pairs
pre-rename by lowercase, the diff pairs post-rename exactly, and rename has its
own direction. A shared helper may only own "normalize a key" and "build a
one-side index"; it must not decide pairing.

**Behaviour:** No change if the helper reproduces the current fold and the
callers keep their own pairing rules. Ordering of emitted records must be
preserved explicitly, because some of these maps are iterated to produce sorted
output.

**Gain:** One full-tree lowercasing pass per index can become one shared
normalization; on a large tree this removes multiple string allocations and
hash inserts per run. It also reduces the chance of the folds drifting (see 1.2).

**Tradeoff:** More coupling and a larger helper surface. Keeping indexes alive
longer can raise peak memory if the helper is used naively; the current code
mostly builds and drops them, which is good.

### 2.2 `phase_a` walks the live list twice and re-derives filters

**Where:** `effective/phase_a.rs` (`scan_stat_only`).

**What:** The first loop builds `live_set` and `dirs` and pulls included
ancestors back in; the second loop re-checks membership and builds the map. The
second pass exists so that an ancestor pulled in by a *later* entry is visible
when an *earlier* entry is processed. In walk order parents precede children, so
most of that concern can be handled in one pass; the safety of doing so depends on
the walk's ordering guarantee.

**Risk:** Medium. `--include` ancestor behaviour is subtle and is covered by
targeted tests. A one-pass rewrite must prove that an entry's inclusion decision
does not depend on a descendant seen later.

**Behaviour:** The effective map should be identical; only construction order
changes.

**Gain:** Half the per-entry membership lookups and one fewer full pass over the
tree; on very large trees the glob and hash work is a real fraction of phase A.

**Tradeoff:** The current two-pass shape is easy to reason about. A one-pass
version is faster but couples the code to walkdir's ordering; a comment and a test
should make that dependency explicit.

Adjacent micro-opportunities in the same phase:

- `is_excluded` rebuilds `MatchOptions` for every entry and re-tests every
  exclude against every ancestor prefix. For filter-heavy runs, a per-directory
  "this subtree is excluded" memo during the walk would remove most of that work.
- The alternate-key index lowercases every cached key even when no exact-case
  miss occurs. This is required for correctness, but the index could use the
  shared normalization helper and avoid a second allocation per key.

### 2.3 Cache scans and per-key reads

**Where:** `commands/sync/rename.rs` (the cache re-key pass),
`commands/sync/apply/phases.rs` (`prune_dst_cache`),
`commands/sync/apply/mod.rs` (`predrop_cache_entries`),
`effective/phase_a.rs` (`load_keys` for alternates).

**What:** A sync can perform several full key scans of the dst cache. The rename
pass then follows its key scan with a `get` per key before opening a write
transaction — an N+1 read pattern on a large cache. `prune` and `predrop` each
scan the key set independently.

**Risk:** Low–medium. These passes are about which rows survive; the ordering
relative to filesystem changes is load-bearing. Combining scans must not move a
read across a write.

**Behaviour:** Same rows, same commit points.

**Gain:** On a large cache, one key scan instead of two or three, and one read
transaction instead of one per renamed key.

**Tradeoff:** The passes run in different phases and own different data (plan,
`dm`, `DirPrefixes`), so combining them means carrying more state forward. A
smaller, safer step is to make the rename re-key read its rows in one transaction
rather than per key.

### 2.4 The copy path's filesystem round trips

**Where:** `commands/sync/apply/copy.rs` (`copy_one`),
`commands/sync/apply/phases.rs` (`copy_files`).

**What:** Per copied file the path does roughly: link-ancestor checks, a final
link check, `is_dir`, possibly another link check, a hard-link count, a src stat,
src open, dst create, stream copy, flush, drop, a second open of dst just to set
mtime, a dst stat, and a dst re-read for the verify. On Windows the hard-link
count is a `CreateFileW`/`GetFileInformationByHandle` pair, and the final link
check can be a `symlink_metadata` plus `read_link`.

**Risk:** Low for the mtime-open removal; medium for merging stat/handle work,
because the hard-link and link checks are safety properties.

**Behaviour:** None if the same checks run in the same order.

**Gain:** One open/close per copied file from setting mtime on the already-open
destination handle; one or two fewer metadata/handle calls per file from
consolidating destination attributes. On a tree of many small files this is
measurable, especially on Windows.

**Tradeoff:** Consolidating into a platform handle helper moves the "read
attributes without following a link" and "link count" logic closer together; that
is exactly the logic `hardlink.rs` exists to centralize, so it should be added
there rather than in `copy_one`.

Two trivial wins in the same area:

- `copy_files` converts the algorithm slice to `AlgoSet` once per copy inside
  `copy_one`; compute it once outside the pool and pass it down.
- `copy_one`'s `d.is_dir()` follows links and is then paired with a separate
  `try_is_link`. A single `symlink_metadata`-based decision could answer both,
  provided the write-through-to-a-file-link behaviour is preserved.

Do not "optimize" the two reads or the per-file `sync_all`; those are the
durability and verify guarantees.

### 2.5 Plan-time filesystem probes

**Where:** `commands/sync/plan/safety.rs` (`spare_link_storage`,
`ensure_mkdir_targets_free`, `ensure_hard_linked_targets_safe`),
`commands/sync/plan/link.rs` (`folded_keys`).

**What:** `spare_link_storage` sorts every `dm` key, canonicalizes `dst`, and on
Windows builds a full lowercase map *before* it knows whether any kept file link
exists. On a large tree with no links this is pure overhead. `ensure_mkdir_targets_free`
stats every candidate prefix without memoizing, and the hard-link preflight opens
every copy target even when no copy will occur.

**Risk:** Low for early returns and prefix memoization; medium for changing what
is probed, because these checks are what make a dry run agree with the real run.

**Behaviour:** Same refusals; possibly a different error when several are
possible, if probe order changes.

**Gain:** A link-free sync avoids a full sort and a full lowercase map; a
copy-free sync avoids all hard-link opens; deep trees avoid repeated prefix
stats. On Windows large trees this can be a noticeable part of planning.

**Tradeoff:** Early returns must be based on a condition that is truly sufficient
("no kept file links", "no copies"), not merely a heuristic. A wrong early return
would silently skip a safety refusal, which is worse than the wasted work.

### 2.6 Redb cache size is hardcoded

**Where:** `cache/db/backend.rs`.

**What:** Every database uses a 64 MiB cache, for both large and small caches.
The comment says the value was chosen partly for tests. On a multi-hundred-
thousand-row cache this can force more page churn than necessary; on a tiny cache
it reserves memory that buys nothing.

**Risk:** Low. Memory use is the only direct effect.

**Behaviour:** None, apart from resource usage and possibly throughput.

**Gain:** Better read/write throughput on large caches if the limit scales with
file size or is configurable.

**Tradeoff:** A larger default raises peak memory for every process. A
configuration knob is another thing to document; scaling automatically makes runs
less predictable and harder to test.

### 2.7 Smaller performance notes

- `HashPlan::pending()` sorts, and `resolve_folder` sorts its own newly built
  pending list without using that result. One of the two sorts is redundant.
- `walk_live` counts files by a second pass over the output vector. The count can
  be maintained while pushing entries.
- `report::record::write_json` sorts a small field vector per record and
  allocates it. That is fine for a handful of fields, but on a tree with a record
  per file it is per-record allocation. A fixed field order or a small inline
  buffer would avoid it.
- `Record::fields_json` clones every field into a `serde_json::Map` for the
  debug log. This is only paid when debug logging is enabled, so it is correctly
  low priority.
- `copy_files` collects one `Result<()>` per copy. The memory is small, but the
  collection exists only to report the lowest-index failure in plan order; a
  bounded error collector could keep only the first failure.
- `RwRuntime::new` creates three gates even in shared/split mode. This is
  documented and the cost is negligible; do not change it for performance.
- `Gate::release` uses `notify_all`. It is a documented tradeoff against missed
  wakeups; leave it.

---

## 3. Clean code and structure

These are mostly readability and duplication, with smaller direct gains than the
performance items. Several are worth doing *because* they reduce the chance of
the correctness drift in section 1.

### 3.1 Give filtering a name

**Where:** `config/common.rs`, `filter.rs`, and every caller that threads
`includes`, `excludes`, and `case_sensitive` through.

**What:** The filter triple travels as three parameters or three `CommonOpts`
fields, and `is_excluded` rebuilds its options on every call.

**Risk:** Low. This is a pure bundling refactor.

**Behaviour:** None.

**Gain:** Fewer parameters, one place to validate/compile patterns, and an
obvious home for a future subtree-exclusion memo.

**Tradeoff:** A new struct adds a layer; it should stay a plain data holder and
not become a trait.

### 3.2 Decompose `copy_one` around its safety checks

**Where:** `commands/sync/apply/copy.rs`.

**What:** `copy_one` mixes five concerns: link containment, final-component link
containment, destination type cleanup, hard-link refusal, and the copy/verify
pipeline. The comments explain each, but the control flow is interleaved.

**Risk:** Medium. Every step is load-bearing, and reordering even the "remove
destination directory" and "count hard links" steps changes what is protected.

**Behaviour:** None if the extracted helpers preserve order and error text.

**Gain:** A reader can see the pipeline (guard, prepare, copy, flush, stamp,
verify) without reconstructing it from comments; future safety checks have one
obvious insertion point.

**Tradeoff:** Extraction can make it easier to reorder accidentally. The helpers
should be private to `copy.rs` and tested by the existing end-to-end cases; a
trait or a generic "copy strategy" would be over-engineering.

### 3.3 Keep the sync orchestrator, but name its phases

**Where:** `commands/sync/mod.rs`.

**What:** `cmd_sync` is a long linear function with numbered comments. The
numbering is good documentation, but the function is where every phase's state
meets.

**Risk:** Medium–high. The phase order is the invariant; moving code can change
borrows, lifetimes, or the point at which a backup happens.

**Behaviour:** None intended, but this is the kind of refactor that can change
when a cache is opened or when a failure aborts.

**Gain:** Readability only. Low direct performance gain.

**Tradeoff:** This is the last thing to do, not the first. The current shape is
explicit and easy to audit; extracting phases for its own sake risks the one
property the module exists to guarantee.

### 3.4 `sled2redb` can use the argument parser already in the dependency graph

**Where:** `src/bin/sled2redb.rs`.

**What:** The binary hand-rolls positional and flag parsing while `clap` is a
normal dependency (and already a dev-dependency).

**Risk:** Low.

**Behaviour:** Help text, error text, and the exact acceptance of flags change.
The exit code can be preserved.

**Gain:** One parser, consistent errors, and `--help` that matches `girsync`.

**Tradeoff:** A small binary gains a derive-based CLI it does not strictly need.
If binary size or compile time matters more, leave it.

### 3.5 Plan labels are built inline from temporaries

**Where:** `commands/sync/mod.rs` (the `SideRequest` construction in `cmd_sync`).

**What:** `label: &format!("folder {}", …)` borrows a temporary for the duration
of the call. It compiles because the temporary lives to the end of the statement,
but it is harder to read than a pre-built label, and the same pattern recurs in
other commands.

**Risk:** Low.

**Behaviour:** None.

**Gain:** A clearer call site and one place that decides how a side is named in
errors.

**Tradeoff:** One allocation per side per run; irrelevant.

### 3.6 Documentation and code should not disagree

**Where:** `DESIGN-NOTES.md` versus `planner/plan.rs` / `planner/pairs.rs`
(see 1.1), and the "known limits" that describe current behaviour.

**What:** The design notes are treated as the contract. Where the code has moved
on (the by-name lookup), either the code or the note should be corrected. Stale
"must be true" statements are worse than no statement in a crate this heavily
documented, because they teach the next reader the wrong invariant.

**Risk:** Low.

**Behaviour:** None.

**Gain:** Trust in the notes; fewer future regressions.

**Tradeoff:** Requires deciding which side is authoritative. For 1.1 the note is
the safer rule; for other notes it may be the code.

---

## 4. Operational and configuration improvements

### 4.1 Make the cache's memory budget configurable or self-scaling

See 2.6. This is primarily an operational tuning knob: large trees benefit from a
larger redb cache, small trees do not need 64 MiB. An environment variable or a
flag is simpler than auto-scaling, because it keeps test behaviour explicit.

### 4.2 Make backups a policy, not a fixed step

See 1.3. A `--no-backup` / `--backup-retain N` pair, or a "only snapshot when the
plan is non-empty" rule, would make the cost proportional to the work. The
snapshot is most valuable for runs that will change dst; a no-op sync does not
need a second copy of the cache. This is a behaviour change and should be
documented in `README.md` as such.

### 4.3 One resolved test-cache decision

`common.test_cache` and `GIRSYNC_TEST_CACHE` are two sources for one answer. The
environment variable exists for subprocess tests; the field exists for library
tests. Resolving them once at command start would remove the per-open read and
make the precedence obvious.

### 4.4 Keep an eye on `--ignore-cache` and backup interaction

`sync` backs up both caches, snapshots dst, then removes both caches when
`--ignore-cache` is set. The backups are still valuable, but the ordering means a
failed removal leaves both a backup and a live cache. This is not a bug, but it
is a place where the operational story ("what is left after a failure") is worth
writing down.

---

## 5. Testing and diagnostics

The suite is extensive and pins a lot of observable behaviour. The useful
additions are not "more of the same"; they target the drift risks above.

1. **Mixed-case foreign digest lookup.** Add a case where a known algorithm has
   an unexpected length and an unusual spelling, on a record side, and assert the
   planner either accepts it (by-name) or refuses for the documented reason.
   This is the missing counterpart to the existing lowercase-foreign tests.
2. **One case-fold rule.** Add tests for names where Unicode lowercasing and the
   platform's identity rule differ, on the platforms where it matters, and pin
   whatever rule is chosen. This is the only way to keep the shared helper from
   silently diverging again.
3. **No-op run cost.** A test or benchmark that a sync with no work does not scan
   the cache more than once, and (if backups become conditional) does not copy
   the cache at all. Cache-scan counts are easier to observe than wall time.
4. **Plan-time early returns.** For the link-storage and hard-link preflights,
   add cases that prove the early return does not skip a refusal: a kept link
   with a mirrored target, a link-free tree, a copy-free tree with hard-linked
   extras, and a deep mkdir path with a blocked prefix.
5. **Conversion failure.** A test that a failed `sled2redb` does not leave a
   destination that looks valid, and that `--force` does not destroy the previous
   destination before the new one is complete.
6. **Benchmarks, not gates.** A small harness over three tree shapes (many tiny
   files, a few large files, deep nesting) would make the performance claims in
   this report checkable. Do not turn it into a CI gate; the crate's current tests
   are about meaning, not speed.

---

## 6. Dependencies and build

- `sled` is used only by the one-shot converter. Feature-gating it (for example
  a `convert` feature that enables the `sled2redb` binary) would remove a heavy
  dependency from normal builds and from the shipped `girsync` binary. **Risk:**
  low–medium; the converter binary must be built with the feature, and docs/tests
  must say so. **Gain:** compile time and supply-chain surface. **Tradeoff:** a
  feature matrix and a less obvious way to run the converter.
- `chrono` is used for timestamps. It is a reasonable choice; replacing it with
  `std::time` formatting would save a dependency but cost clarity and correctness
  for UTC formatting. Not recommended unless dependency count is a hard goal.
- `windows-sys` enables `Win32_Security`; confirm it is actually needed by the
  handle calls. Trimming unused features is a small, low-risk build win.
- `clap` is both a normal and a dev dependency, which is expected here. No action.

---

## 7. Recommended ordering

### Within each category

**Correctness:** 1.1 (by-name lookup) → 1.6 (copy-free preflight early return) →
1.2 (one case fold, because it needs tests first) → 1.4 (meta write path) → 1.3
(backup policy, because it is a product decision) → 1.5 (converter atomicity,
because it is a one-shot path).

**Performance:** 2.1 (shared case index) → 2.5 (plan-time early returns and prefix
memoization) → 2.4 (mtime handle and per-copy `AlgoSet`) → 2.3 (cache scan
consolidation) → 2.2 (phase A one-pass, only after the ordering argument is
tested) → 2.6 (cache size) → 2.7 (micro).

**Clean code:** 3.1 (filter bundle) → 3.5 (labels) → 3.2 (copy decomposition) →
3.4 (converter CLI) → 3.3 (orchestrator extraction, last) → 3.6 (docs, ongoing).

**Operations:** 4.3 (one test-cache decision) → 4.1 (cache size) → 4.2 (backup
policy) → 4.4 (failure story).

**Tests:** 5.1 and 5.2 alongside 1.1/1.2; 5.3–5.5 alongside their performance,
backup, and conversion changes; 5.6 whenever a performance claim needs checking.

### Overall

1. **Fix the documented invariant drift (1.1) and add its test (5.1).** Small,
   local, and it removes a real disagreement between the design notes and the
   code.
2. **Add the cheap plan-time early returns (2.5, 1.6).** They are low-risk, need
   no new abstraction, and help every link-free or copy-free run.
3. **Centralize the case fold and the one-side index (1.2 / 2.1).** This is the
   largest structural performance win, but it must be done as a pure helper with
   the pairing policies left in place. Do it after the tests in 5.2 exist.
4. **Trim copy-path round trips (2.4).** One open per file and one `AlgoSet` per
   run are easy; consolidating destination metadata is a separate, more delicate
   step.
5. **Consolidate cache scans (2.3).** High value on large caches; keep the phase
   boundaries intact.
6. **Decide the backup policy (1.3 / 4.2).** This is a product/safety decision
   more than an engineering one. Do it before tuning cache size, because it
   changes the I/O profile.
7. **Make cache memory configurable (2.6 / 4.1).** Small change, easy to revert.
8. **Clean-code work (section 3).** Do the filter bundle and the copy
   decomposition when touching those files anyway; leave the orchestrator alone
   unless it becomes a real obstacle.
9. **Converter hardening and feature-gating (1.5 / 6).** Valuable but isolated;
   schedule it when the migration path matters.
10. **Benchmark harness (5.6).** Add it once, before the performance work, so the
    gains and regressions are visible; do not gate CI on it.

---

## 8. Things that look worth doing and are not

Repeating these here because a review of this crate will otherwise keep
rediscovering them:

- A single-pass copy verify, or dropping `sync_all` for speed.
- Gating the diff's classification on `--why`; the design deliberately computes
  one verdict per pair so the bucket and the reason cannot disagree. A narrower
  memory win (not storing equal verdicts when `--show-identical` is off) is
  possible, but it changes a documented internal contract and should be treated
  as a separate decision, not a drive-by.
- Making `Side`/`RwSide` a trait, or merging the record and disk scan paths.
- Replacing `AlgoSet` with a bitset crate, or making `FileRec.kind` an enum.
- Unifying the planner's request order with the answer set.
- Adding a third gate or changing the `src → dst` order.
- Folding the planner's and the diff's `CasePairing` into one value.

The value of most of the recommendations above is that they either remove
duplicated *rules* (the case fold, the by-name lookup) or remove duplicated
*work* (the indexes, the scans, the per-file opens) without changing which rule
wins. Changes that unify a rule and a policy at the same time are the ones this
crate's design notes warn about.
