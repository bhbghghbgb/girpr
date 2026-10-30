# girsync refactoring and optimization opportunities

**Scope:** `tools/sync/src` (the `girsync` crate). The sync integration tests are
cited only where they pin behavior; they are not themselves in scope.

**Baseline:** `cargo clippy -p girsync --all-targets -- -D warnings` is clean and
the working tree was clean at `36f311b`. Nothing here is a lint failure; these
are design, DRY, maintainability, and theoretically-sound performance
opportunities.

**Ground rules used when judging a change:**

- An unflagged run's stdout records must stay byte-identical, in both
  `--output text` and `--output json` (NDJSON), and in the same order.
- `sync` exits `0` when the run succeeds, regardless of how many differences it
  applied. `compare` and `compare-self` exit `0` for an empty diff and `4` when
  anything differs. An operational error exits `3`; clap's usage error remains
  `2`. `Diff::total()` and `Diff::is_empty()` are separate predicates:
  `CASE-MISMATCH` is excluded from `total()` but still makes `is_empty()` false.
- A dry run must make exactly the same **decisions** as a real run. It suppresses
  writes only: no cache mutation, no cache creation/backup, and no filesystem
  change in `sync`. Hashing and every plan-time refusal still happen.
- The apply pre-drop is committed **before** the corresponding filesystem change.
  The phase order (`fix_dirs`, `mkdirs`, pre-drop, deletes, copies, rmdirs,
  prune) and the cache durability points are load-bearing.
- Path containment and link handling are security boundaries. A change must not
  make a removal or a copy reach through a link/junction, or truncate a
  hard-linked file, even if the plan and the disk disagree.
- `copy_one` verifies with **two independent reads**: the source digest comes
  from the copy stream, the destination digest comes from a fresh read after
  `sync_all()`.
- `Diff::verdicts` is computed for every both-sides pair whether or not `--why`
  is set. Only its storage or rendering may be optimized; the diff itself must
  not become lazy.
- Deliberate non-fixes stay non-fixes: the `--case-sensitive` prune case, the
  src-rewritten-during-copy race, the stat-only stuck row, and the coverage/trust
  split documented in `DESIGN-NOTES.md`.

## Risk legend

| Risk | Meaning |
|---|---|
| **Low** | Local, behavior-preserving refactor or a pure helper. Tests should pass unchanged. |
| **Medium** | Touches a gate, an ordering rule, a counter, a public type, or a scan verdict. Needs targeted tests and a careful diff. |
| **High** | Changes a documented invariant, a broad public API, or moves a large amount of pipeline logic. Split into smaller changes and land behind tests. |

## Findings at a glance

| ID | Area | Risk | Summary |
|---|---|---|---|
| A1 | rename | Medium | Index the longest-prefix rewrite instead of scanning every move per path. |
| B1 | apply / cache | Medium | Load cache keys once when pre-dropping a link's descendants. |
| B2 | apply / plan | Medium | One `DirPrefixes` helper for every wiped-subtree test. |
| C1 | plan | Low | Precompute the `type_conflict` set used by the link-extra loop. |
| C2 | plan safety | Medium | Precompute copy/fix membership for the link-storage guards. |
| D1 | commands | Low | Share the src/dst link-disjoint validation loop. |
| D2 | commands | Low | Give `ScanMode` a constructor for all four command call sites. |
| D4 | compare-self | Low | Add a count-only cache method for the open log. |
| D3 | commands | Medium | Share the compare/compare-self diff, report, and end-log tail. |
| E1 | report | Low | Compute `why_tag` only when `--why` is set. |
| E3 | report | Low | Render `Record` text and JSON without per-field allocations. |
| E2 | report | Medium | Stream verdict records instead of materializing a `Vec`. |
| F2 | effective | Low | Share the mixed-case collision check and drop the redundant set. |
| F1 | effective | Medium | Build the alternate-case index from keys and fetch adopted rows on demand. |
| G1 | diff | Low | Walk src then dst-only instead of materializing an exact-key union. |
| G2 | diff | Low | Share the both-sides pair classification between the two branches. |
| G3 | planner | Low | Precompute the run-wide `AlgoSet`s used by every pair. |
| G4 | planner | Low | Share the shortfall scan and avoid an exact-key `HashMap` in `shared_pairs`. |
| H1 | cache codec | Low | Decode fields by borrowing slices instead of allocating per field. |
| H2 | cache open | Low | Share the backup/snapshot copy-aside helper. |
| I1 | hash docs | Low | Correct `DigestSet::merge`'s documented winner. |
| I2 | hash lookup | Low | Add a typed digest lookup that does not reparse an algorithm name. |
| I3 | path utilities | Medium | Add an ASCII fast path to Windows `component_eq`. |
| J1 | copy | Low | Reuse one chunk buffer per copy worker instead of one per file. |

---

## A. Case-fixing rename

### A1 — Index the longest-prefix rewrite

**Risk:** Medium · **Kind:** performance / clarity · **Files:**
`tools/sync/src/commands/sync/rename.rs:118-176`,
`tools/sync/src/commands/sync/rename.rs:310-363`,
`tools/sync/src/commands/sync/rename.rs:382-504`

The rename pass has the right semantics but three linear scans around them:

- `plan_steps` calls `rewritten_from(from, &dir_moves)` for every rename. The
  `dir_moves` vector grows during the loop, so planning is `O(renames²)`.
- `final_key` scans every exact rename with `renames.iter().find`, then scans
  every directory move again. It is called once per `dm` key and once per cache
  key, so the cache re-key is `O((keys + renames) × renames)`.
- `DiskSpelling::disk_rel` and `plan_rel` scan the whole move vector on every
  lookup. They are used by the plan-time link guards, which ask about many
  action paths.

The scans are also a place where the three copies can drift. All three need the
same rule: **exact match first, otherwise the longest matching ancestor on a
component boundary**.

**Suggested shape:** introduce one small internal move index. A single
`old -> new` map is enough because an exact hit is just the zero-length suffix
of the ancestor walk:

```rust
#[derive(Default)]
struct MoveIndex<'a> {
    // old path -> new path, for exact renames and directory moves together
    moves: HashMap<&'a str, &'a str>,
}

impl<'a> MoveIndex<'a> {
    fn insert(&mut self, old: &'a str, new: &'a str) {
        self.moves.insert(old, new);
    }

    /// The rewritten path, or `None` when no move applies.
    fn rewrite(&self, rel: &'a str) -> Option<(&'a str, usize)> {
        if let Some(new) = self.moves.get(rel) {
            return Some((new, rel.len()));
        }
        // Longest ancestor first. A match is only valid at a `/` boundary.
        for (i, _) in rel.match_indices('/').rev() {
            if let Some(new) = self.moves.get(&rel[..i]) {
                return Some((new, i));
            }
        }
        None
    }
}
```

Then:

- `plan_steps` keeps a **directory-only** index, inserts `from -> to` after a
  performed directory step, and calls `index.rewrite(from)`. This is
  `O(path_depth)` per rename and preserves parent-before-child behavior.
- `final_key` builds one index from `renames` and the final `dir_moves`, then
  uses the same `rewrite`. Exact renames naturally win because the full key is
  checked before any ancestor.
- `DiskSpelling` builds two indexes, one for `post -> disk` and one for
  `disk -> post`. Both directions use the same boundary rule. `disk_rel` and
  `plan_rel` return `Cow::Borrowed(rel)` when no move applies, exactly as today.

The call sites stop passing `&[(String, String)]` around and stop re-deriving
the answer. The sort that gives `RENAME` order stays in `case_renames`.

**Contract notes:** preserve the existing longest-prefix semantics, the exact
rename winning over an ancestor, the parent-first step order, the no-op steps
whose map/cache key still follows, and the dry-run `DiskSpelling` map. The
`nested_directory_renames_rewrite_through_the_longest_prefix` and
`the_dry_run_spelling_map_undoes_the_planned_renames` tests are the guards.

---

## B. Apply and cache pruning

### B1 — Load the key set once for link pre-drops

**Risk:** Medium · **Kind:** performance / cache ordering · **Files:**
`tools/sync/src/commands/sync/apply/mod.rs:205-230`

For every `delete_files` entry that is a link, `predrop_cache_entries` calls
`w.load_keys()?` and scans the whole cache table for descendants:

```rust
if self.dm.get(r).is_some_and(|rec| rec.is_link) {
    let prefix = format!("{r}/");
    for key in w.load_keys()? {
        if key.starts_with(&prefix) {
            w.remove(&key)?;
        }
    }
}
```

A plan with many links therefore walks the cache once per link. The pre-drop is
one transaction and all removals are idempotent, so the key snapshot does not
need to be refreshed after each link. Load once before the loop (or collect all
link prefixes and do one pass after the loop), then remove every descendant in
the same transaction.

**Suggested shape:**

```rust
let link_prefixes = DirPrefixes::from_dirs(
    plan.delete_files
        .iter()
        .filter(|r| self.dm.get(*r).is_some_and(|rec| rec.is_link))
        .map(String::as_str),
);
// One snapshot for the whole pre-drop transaction.
let keys = if link_prefixes.is_empty() {
    Vec::new()
} else {
    w.load_keys()?
};
for key in keys {
    if link_prefixes.has_ancestor(&key) {
        w.remove(&key)?;
    }
}
```

The `DirPrefixes` helper is B2; B1 can land with a local `HashSet` and then
adopt the helper. The loop must still run before `w.commit()` and before the
delete phase changes the filesystem.

**Contract notes:** the rows removed are the same rows. The link itself is
removed by the existing `w.remove(r)`, and only descendants are matched by the
prefix. The commit remains the durability point before `delete_extras`. Do not
move this into `delete_extras`: a crash between the unlink and the cache write
would leave rows for paths that no longer exist.

---

### B2 — One `DirPrefixes` helper for wiped subtrees

**Risk:** Medium · **Kind:** DRY / performance · **Files:**
`tools/sync/src/commands/sync/apply/phases.rs:236-300`,
`tools/sync/src/commands/sync/apply/phases.rs:345-384`,
`tools/sync/src/commands/sync/plan/build.rs:231-255`

Three places ask the same question: “is this path strictly under one of these
directories?” Each builds a `Vec<String>` of `dir/` prefixes and scans the whole
vector for every key/path:

- `remove_unknown_dirs` collects `wiped: Vec<String>` and checks
  `wiped.iter().any(|p| key.starts_with(p))`;
- `prune_dst_cache` builds `wiped` from copies landing on dst directories and
  link deletes, then checks the same thing;
- `build_plan` builds `wiped` from the copy list to keep `rmdir` from naming a
  directory `copy_one` will already remove.

The copy-wiped derivation is duplicated between `build_plan` and
`prune_dst_cache`, which is exactly the kind of drift the plan exists to avoid.
A key set also makes each check `O(path_depth)` instead of `O(number_of_wiped)`.

**Suggested shape:** a small internal type over directory paths **without** the
trailing slash:

```rust
#[derive(Default)]
struct DirPrefixes {
    dirs: HashSet<Box<str>>,
}

impl DirPrefixes {
    fn from_dirs<'a>(dirs: impl IntoIterator<Item = &'a str>) -> Self { /* ... */ }

    fn insert(&mut self, dir: &str) { self.dirs.insert(dir.into()); }

    /// True only for a strict descendant, never the directory itself.
    fn has_ancestor(&self, rel: &str) -> bool {
        rel.match_indices('/')
            .any(|(i, _)| self.dirs.contains(&rel[..i]))
    }
}
```

Use it in all three places. `build_plan` can build it from `copy` where
`dst_is_dir`; `prune_dst_cache` can use the same constructor plus the
link-delete dirs; `remove_unknown_dirs` can insert each removed directory and
then do one pass over `load_keys()`. Put the helper in `plan/build.rs` (or
`util.rs`) and re-export it to `apply`; it is about plan/apply path semantics,
not about redb.

**Contract notes:** the strict-descendant rule is load-bearing. A copied path
keeps its own cache row even when its old dst was a directory, and a removed
directory must not match itself. `build_plan`'s `rmdir` list must still exclude
the copy-wiped subtree. The `rows_for_a_subtree_a_copy_wiped_are_dropped` and
`a_folder_prunes_its_own_cache_before_the_planner_sees_it` tests pin the two
directions.

---

## C. Plan and plan-time safety

### C1 — Precompute the `type_conflict` set

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`tools/sync/src/commands/sync/plan/build.rs:171-176`

For every dst-only path under a link, `build_plan` asks whether that link is
already a planned type conflict:

```rust
let replaced = diff.type_conflict.iter().any(|t| t.as_str() == link);
```

This is a linear scan of every conflict for every link-extra path. Build a
`HashSet<&str>` once before the `diff.extra` loop and use it for the whole
loop:

```rust
let type_conflicts: HashSet<&str> =
    diff.type_conflict.iter().map(String::as_str).collect();
// ...
let replaced = type_conflicts.contains(link);
```

**Contract notes:** this changes no decision. A conflict at the link still means
the link is replaced, not left as a shared non-convergence. The link rule and
the `shared_link_extras`/`delete_files` split stay exactly as written.

---

### C2 — Precompute copy/fix membership for the link guards

**Risk:** Medium · **Kind:** performance / fail-fast safety · **Files:**
`tools/sync/src/commands/sync/plan/safety.rs:145-168`,
`tools/sync/src/commands/sync/plan/safety.rs:224-259`

`spare_link_storage` walks every kept link and, for each terminal target, does
two linear scans of the plan:

```rust
plan.copy.iter().any(|item| item.rel == *rel || item.rel == tr)
plan.fix_dirs.iter().any(|f| f == tr)
```

`ensure_shared_storage_safe` then scans `plan.copy` again for every shared
storage group. On a plan with many links and many copies this is
`O(links × copies)`. Both checks only need membership, and the plan is not
mutated until `storage.apply` at the end.

**Suggested shape:** build two sets at the start of `spare_link_storage` and
scope them so the mutable `storage.apply(diff, plan)` call is not blocked:

```rust
let copy_rels: HashSet<&str> = plan.copy.iter().map(|i| i.rel.as_str()).collect();
let fix_dirs: HashSet<&str> = plan.fix_dirs.iter().map(String::as_str).collect();
// ... use copy_rels.contains(rel.as_str())
//     / copy_rels.contains(tr.as_str()) / fix_dirs.contains(tr.as_str())
```

Pass the same sets (or build them inside) `ensure_shared_storage_safe`, whose
`rels.contains(&item.rel.as_str())` becomes an intersection test:

```rust
if !rels.iter().any(|r| copy_rels.contains(*r)) {
    continue;
}
```

If a group's label lookup is also hot, carry the display label in the grouped
value instead of the final `terminals.iter().find`; that is optional and can be
a separate small change.

**Contract notes:** every refusal stays fail-fast and happens before the first
filesystem change. The single-link rule (“one file cannot be given two
contents”) and the shared-storage rule are unchanged. Keep the deterministic
group ordering and the exact diagnostic text. The
`two_kept_links_on_one_storage_refuse_when_either_is_copied` test and the
symlink end-to-end case are the guards.

---

## D. Command pipeline

### D1 — Share the link-disjoint validation loop

**Risk:** Low · **Kind:** DRY / maintainability · **Files:**
`tools/sync/src/commands/sync/mod.rs:482-588`

`validate_src_links` and `validate_dst_links` differ only in which side is
walked, which path is joined, and the words in two diagnostics. The body —
skip non-links, canonicalize, check `target` inside `other`, check `other`
inside `target` — is identical. A shared core keeps the two directions from
drifting while leaving the two public wrappers readable.

**Suggested shape:**

```rust
enum LinkSide { Src, Dst }

fn validate_links_disjoint(
    root: &Path,
    entries: &HashMap<String, SideEntry>,
    other_canonical: &Path,
    side: LinkSide,
) -> Result<()> { /* one loop, side-specific message */ }
```

Keep `validate_src_links` and `validate_dst_links` as one-line wrappers so the
call sites and their comments still name the direction. Preserve the exact
context strings (`resolve src link` / `resolve dst link`) and the diagnostic
sentences; those are what a user acts on. If the messages are factored through
a small `match side`, the two arms are still one place.

**Contract notes:** both functions still run after their side's phase A and
before the rename, plan, and every apply phase. Filters and `--max-depth` still
decide which links are in `entries`. The containment relation is unchanged:
inside the other root, or containing the other root, both refuse. This is not a
place to weaken `path_within`.

---

### D2 — Give `ScanMode` a constructor

**Risk:** Low · **Kind:** DRY / API · **Files:**
`tools/sync/src/config/common.rs:304`,
`tools/sync/src/commands/sync/mod.rs:171-176`,
`tools/sync/src/commands/compare.rs:85-90`,
`tools/sync/src/commands/compare_self.rs:182-190`,
`tools/sync/src/commands/update.rs:57-60`

Four commands construct `ScanMode`. `sync` and `compare` spell the same closure:

```rust
let scan_mode = |no_trust_cached_hashes| ScanMode {
    no_trust_cached_hashes,
    dry_run,
};
```

`compare-self` hardcodes `false`/`true`, and `update` hardcodes `true` and
`common.dry_run`. A field added to `ScanMode` currently needs four edits. Add a
constructor on the type:

```rust
impl ScanMode {
    pub const fn new(no_trust_cached_hashes: bool, dry_run: bool) -> Self {
        Self { no_trust_cached_hashes, dry_run }
    }
}
```

Then `sync` and `compare` keep a local closure (or two direct calls),
`compare-self` calls `ScanMode::new(false, true)`, and `update` calls
`ScanMode::new(true, common.dry_run)`.

**Contract notes:** `compare-self` must keep `dry_run: true` unconditionally and
`no_trust_cached_hashes: false`; `update` must keep
`no_trust_cached_hashes: true` and take `dry_run` from the flag. The constructor
must not gain a default that lets a command accidentally opt out of the dry-run
cache suppression.

---

### D4 — Add a count-only cache method for the open log

**Risk:** Low · **Kind:** performance / API · **Files:**
`tools/sync/src/commands/compare_self.rs:157-167`,
`tools/sync/src/cache/db/cachedb.rs:280-309`

`compare-self` reports the record's row count by loading every key and
allocating a `String` for each, then immediately calls
`load_record_side_from`, which loads and decodes the whole table again:

```rust
entries = record.load_keys()?.len(),
// ...
let rec = load_record_side_from(&record, ...)?;
```

The count is only for the log line. Add a count-only method that asks the table
for its length (or iterates without building the path strings), and keep
`load_keys` for callers that need the keys themselves.

**Suggested shape:**

```rust
/// Number of rows in the entries table, without decoding or allocating keys.
pub fn entry_count(&self) -> Result<u64> {
    let txn = self.db.begin_read().context("begin read")?;
    let tbl = match txn.open_table(ENTRIES) {
        Ok(t) => t,
        Err(e) if is_table_missing(&e) => return Ok(0),
        Err(e) => bail!("open entries table: {e:#}"),
    };
    tbl.len().context("count cache entries")
}
```

The log field is `u64`-shaped; no formatting contract changes.

**Contract notes:** the logged count is the **raw** cache row count, before the
record side filters cache paths or globs. Do not replace it with
`rec.map.len()`, which answers a different question. A missing table still reads
as zero.

---

### D3 — Share the compare/compare-self tail

**Risk:** Medium · **Kind:** DRY / maintainability · **Files:**
`tools/sync/src/commands/compare.rs:160-181`,
`tools/sync/src/commands/compare_self.rs:259-284`,
`tools/sync/src/commands/mod.rs:39-43`

After the two commands have built their maps, `diff_maps` is called with the
same plan-derived arguments. The last two steps are then identical:
`report_diff`, followed by one `info!` carrying `missing`, `extra`, `changed`,
`type_conflict`, `case_mismatch`, `total_diff`, `elapsed_s`, `exit`, and the
message `"end"`. Only the local variable names differ. A shared helper for those
two steps prevents a new summary field or exit rule from being added in one
place only.

**Suggested shape:**

```rust
fn report_and_finish(
    diff: &Diff,
    report: &Report,
    why: bool,
    show_identical: bool,
    t0: Instant,
) -> i32 {
    let code = report_diff(diff, report, why, show_identical);
    info!(
        missing = diff.missing.len(),
        extra = diff.extra.len(),
        changed = diff.changed.len(),
        type_conflict = diff.type_conflict.len(),
        case_mismatch = diff.case_mismatch.len(),
        total_diff = diff.total(),
        elapsed_s = elapsed_s(t0),
        exit = code,
        "end"
    );
    code
}
```

Each command calls it and returns `Ok(code)`. The helper belongs in
`commands/mod.rs` beside `report_diff`, or as a small private function in the
commands module.

**Contract notes:** exit `0`/`4` still comes from `Diff::is_empty`, not from the
record stream. `total_diff` still excludes `identical` and `case_mismatch`;
`case_mismatch` is still a logged field and still affects the exit through
`is_empty`. The record order and the `"end"` log fields must stay the same.

---

## E. Reporting

### E1 — Compute `why_tag` only when `--why` is set

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`tools/sync/src/report/mod.rs:190-196`,
`tools/sync/src/report/mod.rs:213-220`

`verdict` currently calls `diff.why_tag(r)` for every `CHANGED` and every
`IDENTICAL` record, then discards the result when `why` is false:

```rust
out.push(match diff.why_tag(r) {
    Some(tag) if why => rec.put("why", tag),
    _ => rec,
});
```

`Verdict::why_tag` allocates a `String` (and sometimes a `Vec` for the field
list). On a large diff without `--why`, that is one allocation per changed path
for a value nobody reads.

**Suggested shape:**

```rust
let mut rec = Record::path("CHANGED", r);
if why && let Some(tag) = diff.why_tag(r) {
    rec = rec.put("why", tag);
}
out.push(rec);
```

Do the same for the `IDENTICAL` loop.

**Contract notes:** `why=false` still emits the same record, with no field. A
path with no verdict still gets no `why=` field, never an empty one. The
`Diff::verdicts` map is still built unconditionally; this only stops rendering
a tag that will not be printed.

---

### E3 — Render `Record` without per-field allocations

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`tools/sync/src/report/record.rs:104-195`,
`tools/sync/src/report/mod.rs:130-142`

The text renderer calls `bare(v)`, which clones a `Val::Str` and builds a
`Vec<String>` for every array. The JSON renderer builds a `Map`, clones every
value into it, and then `Report::render` calls `to_string()`. `event()` also
allocates a lowercase `String` for every record.

Reports are one line per event, so this is not the hottest path in the tool, but
the helpers are easy to make allocation-free:

- replace `bare(&Value) -> String` with
  `push_bare(&mut String, &Value)` and recurse into arrays;
- replace `event()` on the render path with `push_event(&mut String)` (the
  labels are ASCII, so `push(c.to_ascii_lowercase())` is exact);
- add `write_json(&mut String)` that writes the object directly. Keep
  `Record::json() -> Value` as the public convenience wrapper for tests and
  library callers.

The JSON writer must **sort keys alphabetically**, including `event`, because
the current `serde_json::Map` is a `BTreeMap` without `preserve_order`; a direct
writer with insertion order would change the bytes even though the docs tell
consumers to read by name. The safest first version is to sort a small
`Vec<(&str, &Value)>` and then emit each key/value with the existing JSON
escaping rules.

**Contract notes:** text and JSON must remain byte-identical, including the
`Val::UInt` vs `Val::Int` distinction, array joining with commas inside `[]`,
and JSON escaping. `Record::fields_json` must keep the same compact object for
the log event. The existing `text_and_json_are_two_renderings_of_one_record`,
`layouts_render_the_documented_lines`, and output tests are the guards.

---

### E2 — Stream verdict records instead of building a `Vec`

**Risk:** Medium · **Kind:** performance / API · **Files:**
`tools/sync/src/report/mod.rs:182-235`,
`tools/sync/src/commands/mod.rs:39-43`,
`tools/sync/src/commands/sync/mod.rs:343-348`

`verdict` materializes every record — including the summary — into a
`Vec<Record>`, and both `report_diff` and `sync` then walk it. On a
`--show-identical` run over a large tree, that is one `Vec` entry per file held
until the whole document is built, even though every record is emitted
immediately.

**Suggested shape:** add a visitor and keep the current function as a thin
collecting adapter:

```rust
pub fn visit_verdict(
    diff: &Diff,
    why: bool,
    show_identical: bool,
    mut emit: impl FnMut(Record),
) {
    // same loops and order as today; call emit(rec) instead of out.push(rec)
}

pub fn verdict(diff: &Diff, why: bool, show_identical: bool) -> Vec<Record> {
    let mut out = Vec::new();
    visit_verdict(diff, why, show_identical, |rec| out.push(rec));
    out
}
```

`report_diff` and the `sync` plan print call `visit_verdict` directly. In
`sync`, the `SUMMARY` filter moves into the closure or into a small adapter:

```rust
visit_verdict(&diff, common.why, common.show_identical, |rec| {
    if rec.label() != "SUMMARY" {
        report.emit(rec);
    }
});
```

**Contract notes:** the order is unchanged: missing, extra, changed,
type-conflict, case-mismatch, identical (when asked), summary last. The summary
is still the only record with `total_diff`, and `identical` is still conditional
on the flag. Keeping `verdict` as an adapter preserves the public
`girsync::verdict` API and the tests that collect it. This does not make the
diff lazy: every both-sides pair still gets a `Verdict`.

---

## F. Effective scan

### F2 — Share the mixed-case collision check

**Risk:** Low · **Kind:** DRY / correctness-adjacent · **Files:**
`tools/sync/src/scan.rs:142-161`,
`tools/sync/src/effective/record.rs:63-78`

Both the live walk and the record loader group keys by lowercase and reject a
group with more than one spelling. The live version additionally builds a
`HashSet<&&str>` just to deduplicate the group:

```rust
let uniq: HashSet<&&str> = v.iter().collect();
if uniq.len() > 1 { ... }
```

The record version has the same shape with `HashSet<&String>`. Because the input
is a set of paths and mixed-case collisions are already the thing being tested,
the extra set is redundant: a group with two entries is a collision, and a
group with one is not. If duplicate exact keys are possible, sort and `dedup`
the small vector instead.

**Suggested shape:** one helper over string keys:

```rust
fn check_mixed_case_keys<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    label: &str,
) -> Result<()> {
    let mut groups: HashMap<String, Vec<&'a str>> = HashMap::new();
    for key in keys {
        groups.entry(key.to_lowercase()).or_default().push(key);
    }
    for (_, mut group) in groups {
        group.sort_unstable();
        group.dedup();
        if group.len() > 1 {
            bail!("mixed-case collision in {label}: {}", group.join(" vs "));
        }
    }
    Ok(())
}
```

The live caller passes `live.iter().map(|e| e.rel.as_str())` and
`&root.display().to_string()`; the record caller passes its keys and
`format!("record {label}")` (or the helper takes a prefix and a label to avoid
that one allocation).

**Contract notes:** the fatal condition is unchanged: in insensitive mode, two
paths differing only by case abort the run. The message prefix and the set of
joined spellings stay the same; the live side was already sorted. Sorting the
record-side group is a determinism improvement for an error path, so if a test
pins the old unsorted order, update the test rather than keeping a
nondeterministic message.

---

### F1 — Index alternate keys, fetch adopted rows on demand

**Risk:** Medium · **Kind:** performance / memory · **Files:**
`tools/sync/src/effective/phase_a.rs:123-190`

In insensitive mode, phase A reads the **entire** cache into `alt_recs` before
the walk:

```rust
let alt_recs: HashMap<String, FileRec> = if !case_sensitive {
    cache.load_all()?
} else {
    HashMap::new()
};
```

It then builds `alt_index` from those keys and, on an exact-case miss with one
alternate, clones the adopted `FileRec` out of `alt_recs`. The result is a
second full copy of the cache in memory, plus a decode of every row that will
never be adopted. The `SideEntry` then clones the adopted record's hashes again,
and `merge_row` may clone them once more.

Only the **keys** are needed to decide whether a path has an alternate. The
value is needed only for the (normally rare) adopted keys.

**Suggested shape:** load keys, not records:

```rust
let alt_keys = if case_sensitive { Vec::new() } else { cache.load_keys()? };
let mut alt_index: HashMap<String, Vec<String>> = HashMap::new();
for key in alt_keys {
    alt_index.entry(key.to_lowercase()).or_default().push(key);
}
```

When `others.len() == 1`, fetch that row on demand:

```rust
let old = others[0];
let rec = match batch.as_mut() {
    Some(w) => w.get(old)?,
    None => cache.get(old)?,
};
if let Some(w) = batch.as_mut() {
    w.remove(old)?;
}
(rec, true)
```

This keeps the batch handle's view consistent (it sees the same committed state
at this point) and uses a read-only `get` in a dry run. The normal path — an
exact-case hit — never pays for the lookup.

**Contract notes:** disk casing still governs. Exactly one stale alternate is
adopted; zero or several stale alternates are not. The adoption still removes
the old key and rewrites the disk-cased key, including in a dry run's decision
set. `stat_matches` and `merge_row` are unchanged, so a stale row's digests are
still dropped. The prune count is still computed in both modes and must not be
derived from a pre-adoption key snapshot. An adopted old key is renamed, not
pruned. A real run's batch has already dropped it; the dry run still reads the
pre-adoption cache today, so it would count that key as stale. Exclude adopted
keys from the dry-run stale set (and add an insensitive-mode dry/real counter
test), or `pruned` is one higher per adoption. Orphan and excluded rows still
count exactly once.

---

## G. Diff and planner

### G1 — Walk src then dst-only in the exact branch

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`tools/sync/src/diff/mod.rs:236-264`

The exact-key branch builds a `HashSet<&String>` of every key from both maps,
then looks each key up in both maps again:

```rust
let mut keys: HashSet<&String> = HashSet::new();
for k in src.keys().chain(dst.keys()) {
    keys.insert(k);
}
for k in keys { /* src.get(k), dst.get(k) */ }
```

The buckets are sorted at the end, so iteration order does not matter. Walk
`src` once and classify its misses, then walk `dst` and classify only the keys
`src` does not hold:

```rust
for (k, s) in src {
    match dst.get(k) {
        None => d.missing.push(k.clone()),
        Some(t) => classify_pair(&mut d, k, s, t, /* case mismatch */ None),
    }
}
for (k, t) in dst {
    if !src.contains_key(k) {
        d.extra.push(k.clone());
    }
}
```

This removes the union allocation and the second lookup per key.

**Contract notes:** `missing`/`extra`/`changed`/`type_conflict` are sorted
afterwards, so the emitted order is unchanged. `case_mismatch` is only produced
by the lowercase branch. A both-sides pair still gets exactly one
`pair_verdict` and one `verdicts` entry.

---

### G2 — Share the pair classification

**Risk:** Low · **Kind:** DRY / correctness-adjacent · **Files:**
`tools/sync/src/diff/mod.rs:241-264`,
`tools/sync/src/diff/mod.rs:284-304`

The exact and lowercase branches contain the same classification block:

```rust
let v = pair_verdict(stat, required, s, t, key);
if v.is_changed() {
    d.changed.push(key.clone());
} else if want_identical {
    d.identical.push(key.clone());
}
d.verdicts.insert(key.clone(), v);
```

The lowercase branch additionally pushes `case_mismatch` before that. Extract
one helper:

```rust
fn classify_pair(
    d: &mut Diff,
    key: &str,
    s: &EffRec,
    t: &EffRec,
    stat: StatTrust,
    required: &Required,
    want_identical: bool,
) {
    if s.kind != t.kind {
        d.type_conflict.push(key.to_string());
        return;
    }
    let v = pair_verdict(stat, required, s, t, key);
    if v.is_changed() {
        d.changed.push(key.to_string());
    } else if want_identical {
        d.identical.push(key.to_string());
    }
    d.verdicts.insert(key.to_string(), v);
}
```

The caller keeps the `case_mismatch` push before calling it. This is one
definition of “changed vs identical vs verdict”, which is the property the
module already claims.

**Contract notes:** kind conflicts still bypass `pair_verdict` and get no
`verdicts` entry. `want_identical` still only fills the equal bucket. The diff
and the reporter still read the same `Verdict` value.

---

### G3 — Precompute the run-wide `AlgoSet`s

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`tools/sync/src/planner/pairs.rs:249-327`

`plan_pairs` builds `AlgoSet::from_iter(...)` on every undecided pair:

```rust
let want = AlgoSet::from_iter(src.algos.iter().copied());
plan_one(&src, s, srel, want, &mut plan.src);
plan_one(
    &dst, d, drel,
    AlgoSet::from_iter(dst.algos.iter().copied()),
    &mut plan.dst,
);
```

The request is run-wide; the per-pair answer under `AllOf` is the same set every
time. Compute it once before the loop, keeping the src and dst requests
separate because `SideRequest` permits them to differ:

```rust
let src_all = AlgoSet::from_iter(src.algos.iter().copied());
let dst_all = AlgoSet::from_iter(dst.algos.iter().copied());
// use src_all / dst_all in the loop
```

`plan.required.fallback` already uses `src_all`; use the same value there so the
fallback and the per-path answer cannot be built from different inputs.
`AnyOf`'s `AlgoSet::one(pick)` is already per-pair and stays.

**Contract notes:** `--hash-any-of` tie-breaking still reads the ordered
`&[HashAlg]` request, never an `AlgoSet`. The fallback remains the src request.
Do not collapse `src.algos` and `dst.algos` into one list without checking the
caller contract.

---

### G4 — Share the shortfall scan and remove the exact-key `HashMap`

**Risk:** Low · **Kind:** DRY / performance · **Files:**
`tools/sync/src/planner/pairs.rs:106-140`,
`tools/sync/src/planner/pairs.rs:300-354`

Two independent cleanups in the same path:

1. `shared_pairs`' exact branch builds a `HashMap<&str, &String>` of every src
   key just to look up each dst key. Both maps are keyed by `String`, so
   `src.get(drel.as_str())` is the same lookup without the extra table:
   ```rust
   for drel in dst.keys() {
       if let Some(srel) = src.get(drel.as_str()) {
           out.push((srel, drel));
       }
   }
   ```
2. The shortfall block is duplicated in `AllOf` and in the `AnyOf` “no pick”
   branch. Extract a helper that records, for each side that cannot hash from
   disk, the requested algorithms it cannot obtain:
   ```rust
   fn record_shortfalls(
       tally: &mut [Tally; 2],
       sides: &[(usize, &SideRequest<'_>, &SideEntry, &String)],
   ) { /* ... */ }
   ```
   `AllOf` calls it for every undecided pair; `AnyOf` calls it only when
   `pick_one` returns `None`.

**Contract notes:** `AllOf` still records every shortfall; `AnyOf` still records
only when nothing is obtainable on both sides, because a record short of one
algorithm may be answerable by another. Coverage counts are still accumulated
under both modes. The exact branch still assumes phase A already made
mixed-case collisions fatal.

---

## H. Cache codec and snapshots

### H1 — Decode fields by borrowing slices

**Risk:** Low · **Kind:** performance / clarity · **Files:**
`tools/sync/src/cache/codec.rs:92-133`

`decode_rec`'s `take` closure returns a new `Vec<u8>` for every field:

```rust
let take = |cur: &mut &[u8], n: usize| -> Result<Vec<u8>> {
    // ...
    Ok(h.to_vec())
};
```

A normal row therefore allocates for the format byte, kind byte, size, mtime,
count, each name, and each digest. The bytes are already in the input slice; the
decoder should borrow them and only allocate where the destination type owns
data (`kind: String`, `insert_named`'s digest copy).

**Suggested shape:**

```rust
fn take<'a>(cur: &mut &'a [u8], n: usize) -> Result<&'a [u8]> {
    if cur.len() < n {
        bail!("truncated cache entry");
    }
    let (head, tail) = cur.split_at(n);
    *cur = tail;
    Ok(head)
}
```

Use `std::str::from_utf8(take(...)?)` for the name and pass the borrowed digest
to `insert_named`. `from_le_bytes` still needs a fixed-size conversion; the
slice borrow avoids the intermediate `Vec` there too.

**Contract notes:** the wire format is unchanged. An unknown algorithm name is
still carried, and a known name with an unexpected length still lands in
`foreign`. Truncated entries and trailing bytes still fail with the same
conditions. The byte-identity codec test is the guard.

---

### H2 — Share the backup/snapshot copy-aside helper

**Risk:** Low · **Kind:** DRY / maintainability · **Files:**
`tools/sync/src/cache/open.rs:127-168`

`backup_db` and `snapshot_old` are the same function with two names, two message
prefixes, and two destination prefixes:

```rust
let dest = unique_sibling(parent, format!("girpr-cache-backup-{}", ts_now()));
// ... is_dir -> copy_dir_all, else fs::copy ...
```

They differ only in `girpr-cache-backup-` vs `girpr-cache-old-` and the
`"backup"`/`"snapshot-old"` log labels. A private helper taking a prefix and a
label removes the duplicate while keeping the two public functions and their
instrumentation.

**Suggested shape:**

```rust
fn copy_aside(db_path: &Path, prefix: &str, label: &str) -> Result<Option<PathBuf>> {
    // shared existence check, unique_sibling, dir/file copy, start/done logs
}
```

The public functions become one-line calls. Keep `#[tracing::instrument]` on the
public wrappers so the span names stay `backup_db` and `snapshot_old`; the helper
owns only the shared body and the labelled start/done messages.

**Contract notes:** backup names stay `girpr-cache-backup-<ts>` and snapshots
stay `girpr-cache-old-<ts>`; both remain keep-all and never overwrite an
existing sibling. Legacy sled directories still use `copy_dir_all`. A missing
source still returns `Ok(None)`.

---

## I. Hash and path utilities

### I1 — Correct `DigestSet::merge`'s documented winner

**Risk:** Low · **Kind:** documentation / correctness-adjacent · **Files:**
`tools/sync/src/hash/digestset.rs:140-173`,
`tools/sync/src/effective/entry.rs:178-188`

The doc says merge folds `other` in “with `self` winning”, but the code writes
`other`'s digest over `self`:

```rust
if let Some(d) = other.md5 {
    self.md5 = Some(d);
}
```

`merge_row` relies on that direction: it clones the prior row into `self` and
passes this run's `computed` as `other`, because the computed digests must win
while uncomputed prior digests ride along. The implementation is right and the
documentation is wrong. Fix both comments: the `merge` doc at
`hash/digestset.rs:140` and the `merge_row` comment at `effective/entry.rs:180`
repeat the same “`self` winning” sentence, then immediately describe `computed`
replacing the prior row. Say **`other` wins** in both, and point at `merge_row`
as the reason. Do not swap the implementation to match the old sentence.

**Contract notes:** the merge-write rule is pinned by the `merge_row_*` tests in
`effective/entry.rs` and by the foreign-digest merge test in `hash/tests.rs`.
The doc and those tests should tell the same story.

---

### I2 — Add a typed digest lookup

**Risk:** Low · **Kind:** performance / API · **Files:**
`tools/sync/src/hash/digestset.rs:71-87`,
`tools/sync/src/planner/pairs.rs:291,405-417`,
`tools/sync/src/planner/plan.rs:116`,
`tools/sync/src/diff/mod.rs:345-356`,
`tools/sync/src/commands/sync/apply/copy.rs:59-66`

`get_named(&str)` parses the name into a `HashAlg` on every call, even when the
caller already has a `HashAlg`. It is used in the planner's per-pair loop, the
diff's per-pair algorithm loop, and the copy verifier. With two algorithms the
parse is small, but it is repeated for every undecided pair and every digest.

Add a typed lookup that keeps the current fallback semantics:

```rust
/// The digest for `a`, including a malformed known-name entry carried in
/// `foreign` (a known name with an unexpected length).
pub fn get_algo(&self, a: HashAlg) -> Option<&[u8]> {
    self.get(a).or_else(|| {
        self.foreign
            .iter()
            .find(|(name, _)| &**name == a.as_str())
            .map(|(_, digest)| digest.as_slice())
    })
}
```

Use it at typed call sites. Keep `get_named` for callers that genuinely have a
string (the codec and tests).

**Contract notes:** do **not** simply replace these calls with `get(HashAlg)`.
`get` cannot see a known name with the wrong length that was routed to
`foreign`; `get_algo` preserves the documented behavior that such a row still
counts as held and still compares its bytes. This is availability and diff
semantics, not just a lookup optimization.

---

### I3 — Add an ASCII fast path to Windows `component_eq`

**Risk:** Medium · **Kind:** performance / security-adjacent · **Files:**
`tools/sync/src/util.rs:137-147`

On Windows, every path component comparison allocates two lowercased `String`s:

```rust
a.as_os_str().to_string_lossy().to_lowercase()
    == b.as_os_str().to_string_lossy().to_lowercase()
```

`same_path` and `path_within` are used in input validation, link checks, and
removal guards. Most paths are ASCII, where case folding is just
`eq_ignore_ascii_case`.

**Suggested shape:**

```rust
fn component_eq(a: Component<'_>, b: Component<'_>) -> bool {
    #[cfg(windows)]
    {
        let (a, b) = (a.as_os_str(), b.as_os_str());
        if a == b {
            return true;
        }
        if let (Some(a), Some(b)) = (a.to_str(), b.to_str())
            && a.is_ascii()
            && b.is_ascii()
        {
            return a.eq_ignore_ascii_case(b);
        }
        // Non-ASCII or non-UTF-8 keeps the existing lossy Unicode fold.
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}
```

**Contract notes:** this is still a component-wise comparison and still
case-insensitive on Windows only. The ASCII fast path must not replace the
Unicode fallback: Windows paths are not ASCII-only, and the fallback is what
preserves the current behavior for non-ASCII names and non-UTF-8 `OsStr`
values. The `containment_is_component_wise` and
`path_identity_ignores_case_on_windows` tests are the guards. Do not change
`path_within` to a textual prefix check for speed.

---

## J. Copy workers

### J1 — Reuse one chunk buffer per copy worker

**Risk:** Low · **Kind:** performance / memory · **Files:**
`tools/sync/src/commands/sync/apply/copy.rs:26-46`,
`tools/sync/src/commands/sync/apply/phases.rs:153-204`

`copy_and_digest` allocates a 512 KiB buffer per copied file:

```rust
let mut buf = vec![0u8; CHUNK];
```

The copy phase runs many files through a Rayon pool, so this is one allocation
and one zero-fill per file, on top of the actual I/O. The buffer is
worker-scoped state, not file-scoped state.

**Suggested shape:** have the copy phase create a scratch buffer per worker with
`map_init`, and pass it down:

```rust
plan.copy
    .par_iter()
    .map_init(|| Vec::<u8>::new(), |buf, item| {
        // ...
        copy_one(..., buf)
    })
```

`copy_and_digest` resizes to `CHUNK` only when `algos` is non-empty, so a
`--hash none` copy can leave the buffer empty and keep the `io::copy` fast path.
The buffer is never shared across workers; the Rayon initializer owns its
lifetime.

**Contract notes:** the bytes written, the digests, the `sync_all` point, and the
independent destination re-read are unchanged. Resize the scratch buffer to
`CHUNK` before the loop, and after each `read` use only `buf[..n]`; do not assume
the reused tail was cleared. This is a per-worker cache, not a cross-file data
cache.

---

## Suggested implementation order

The order below runs from pure cleanup through local performance to
behavior-adjacent changes, ending with the rename path. Each step lands with a
green suite before the next one starts.

### Phase 0 — pure cleanup (no behavior change)

1. **I1** — correct `DigestSet::merge`'s doc to match `merge_row`.
2. **D2** — add `ScanMode::new`.
3. **D1** — share the link-disjoint validation loop, preserving diagnostics.
4. **E1** — gate `why_tag` behind `why`.
5. **E3** — allocation-free `Record` rendering, with sorted JSON keys.
6. **F2** — one mixed-case collision helper.
7. **H2** — one backup/snapshot copy-aside helper.
8. **C1** — precompute the `type_conflict` set.

### Phase 1 — local performance and reporting

9. **D4** — add `CacheDb::entry_count` and use it for the open log.
10. **G1** — src-then-dst-only exact diff.
11. **G2** — shared pair classification.
12. **G3** — precompute the run-wide `AlgoSet`s.
13. **G4** — shared shortfall scan and exact-key lookup.
14. **H1** — borrowed-slice codec decode.
15. **I2** — typed digest lookup with the foreign fallback.
16. **J1** — per-worker copy scratch buffer.
17. **E2** — `visit_verdict` with the collecting adapter retained.

### Phase 2 — behavior-adjacent

18. **B1** — one key snapshot for link pre-drops.
19. **B2** — `DirPrefixes` across `build_plan`, `remove_unknown_dirs`, and
    `prune_dst_cache`.
20. **C2** — copy/fix membership sets in the link-storage guards.
21. **D3** — shared compare/compare-self tail.
22. **F1** — keys-only alternate index, on-demand adopted rows, and the
    adoption-aware dry-run prune count.
23. **I3** — Windows ASCII fast path with the Unicode fallback.

### Phase 3 — rename path

24. **A1** — `MoveIndex` for `plan_steps`, `final_key`, and `DiskSpelling`.
    Land this after the diff/plan tests are green, because it touches both the
    filesystem-facing step planner and the cache re-key.

## What not to change

These are deliberate decisions documented in the code, README, or
`DESIGN-NOTES.md`. A refactor that “fixes” them is a regression:

- **`Diff::verdicts` is computed for every both-sides pair.** Do not make the
  diff conditional on `--why`. A lazy diff would be two diffs, and a `CHANGED`
  record and its tag could disagree.
- **`copy_one`'s two independent reads.** The source digest comes from the copy
  stream; the destination digest comes from a fresh read after `sync_all()`.
  A single-pass “verify” compares a value with itself and is not a verify.
- **The pre-drop and phase order.** Cache rows are removed and committed before
  the filesystem change. `predrop_cache_entries` is belt-and-braces, and it
  becomes load-bearing if the copy ever stops truncating, stops deferring mtime,
  or stops re-reading the destination.
- **Rename prefix semantics.** A prefix matches only on a `/` boundary, the
  longest prefix wins, an exact rename wins over an ancestor, parents are
  processed before children, and a child covered by a parent move is a no-op
  that still re-keys its map/cache row.
- **The dry-run rename re-key.** A dry run must re-key `dm` to src's casing even
  though it does not touch the disk, and `DiskSpelling` must let the plan-time
  guards stat the old spelling.
- **Link, junction, and hard-link guards.** This includes
  `validate_src_links`, `validate_dst_links`, `link_ancestor_above`,
  `link_ancestor`, the final-component carve-outs, `guard_removal_outside_src`,
  and the hard-link count/credit rules. Do not weaken them for speed or to make
  a plan “just apply”.
- **Shared-link non-convergence.** A dst link src also has is left in place, and
  its extra descendants are a default-run refusal or a warning under
  `--keep-extra`/`--missing-only`. Do not turn that into a recursive delete
  through the link.
- **`CASE-MISMATCH` counting.** It is excluded from `Diff::total()` but included
  in `Diff::is_empty()` and therefore in the compare exit code. `sync` still
  exits `0` after a successful run even when the diff is non-empty.
- **Dry-run decisions.** A dry run still stats, reads cached digests, hashes
  undecided pairs, runs every plan-time guard, and computes `pruned` and the
  summary counts. Only writes are suppressed.
- **Cache durability and forward compatibility.** Commits stay durable in
  production, the pre-drop commit stays before filesystem changes, unknown
  algorithm names stay readable, and a known name with an unexpected digest
  length stays in `foreign` rather than becoming a hard failure.
- **The documented known limits.** The `--case-sensitive` prune case, the
  src-rewritten-during-copy race, and the stat-only stuck row are known
  trade-offs in `DESIGN-NOTES.md`, not open bugs for this refactor to “solve”.
