# Review note — the scratch-directory refactor (`17dfc98`)

A hand-off note on the commit `17dfc98` ("girpr, girpr-tempdir, gircap, girsync: the scratch
directory is one crate, not six copies"), the work being **"unify temp directory creation across
projects"**. It is written for the agent that did the work: every item is a concrete, located
fix, not a direction.

**This file is temporary.** It is deleted once the fixes land, exactly like
`docs/10-artifact-test-scope-review.md` and `docs/07-streaming-zst-decode-review.md` were. Nothing
should refer to it — not the code, not the documents, not the comments. A finding that needs a
permanent home moves into the document that owns the thing it is about.

---

## 0. What I already verified — do not redo it

Everything below was run against the tree as it stands (`17dfc98`, the current `HEAD`), on this
machine:

| check | result |
|---|---|
| `cargo check --workspace --all-targets` | **clean** — no warnings, no errors |
| `cargo nextest run --workspace` | **1235 passed, 8 skipped**, 0 failed — the same numbers the commit message reports |
| per-package counts | girpr 268 (the README's number is still right), girpr-log 18, **girpr-tempdir 7**, gircap 619, girsync 324 |
| `git grep -n "testtemp\|TempRoot\|tempdir\.rs"` | **empty** — no source, no document, no comment refers to the six copies |
| `git grep -n "env::temp_dir"` | **one hit**, `crates/tempdir/src/lib.rs:82` — the shared crate is now the only thing in the tree that makes a scratch directory |

**The refactor itself is sound, and it is complete.** There is nothing here about the shape of
the change: the API is a faithful union of what the copies had, every call site is a rename, no
capability was lost, and the dependency boundary is right — `girpr-tempdir` is a **dev-dependency**
of all three projects, nothing that ships links against it, and the crate itself has no
dependencies at all.

**And it found two drifts the copies had already accumulated**, which is the strongest argument
for having done it:

- girsync's `TempRoot` was not a copy at all — a different name (`girsync_run_<pid>_<nanos>_<n>_<tag>`),
  a different API (`mkdirs`, `root()`), and its own `Drop`.
- `tools/sync/src/scan.rs` built `girsync_scan_prune_<pid>` by hand: pid-only, so two concurrent
  runs of that suite shared one tree, and a panic before the last line left it behind.

**Do not redo the refactor.** What is left is six small corrections — three of them in prose, two
of them in the crate's own contract, one of them a sentence in a doc that now contradicts itself.

---

## 1. The crate's own docs say six identical copies; there were two per project, and one had already drifted

**Where:** `crates/tempdir/src/lib.rs:4-9`.

**What it is.** The module docs open with:

```rust
//! copy of this helper — six copies in all, three of them in the same project
//! — and the copies had to be kept in step by hand, because a helper that
//! drifts is a helper whose two halves fail in different ways.
```

"**three of them in the same project**" is wrong. The six were two in each project:
`src/testtemp.rs` + `tests/common/tempdir.rs` (girpr), `src/testtemp.rs` + `tests/tempdir.rs`
(gircap), `src/testtemp.rs` + the `TempRoot` in `tests/common/mod.rs` (girsync).

The second half of the sentence is also wrong in a way that matters more: it says the six were
kept in step. They were not. girsync's second copy had **already** drifted — different name
shape, different API, its own cleanup. The commit message says so plainly ("girsync's `TempRoot`
built its own name … and cleaned up in its own `Drop`"), but the crate's own docs, which is where
a reader looks first, say the opposite.

**Why it matters.** This crate exists because a drifted copy of a policy is invisible until it
eats another test's tree. Its module docs are the argument for the crate, and they currently
argue for a situation that did not exist: five copies kept in step by hand, and one that drifted
in a way the text says was impossible. A reader who checks the history finds a sixth copy that
already disagreed with the other five — which is a better argument for the crate, not a worse
one.

**Fix.** Replace the second half of that paragraph so it says what is true. Something like:

```rust
//! copy of this helper — six copies in all, two in each project — and they
//! had to be kept in step by hand, because a helper that drifts is a helper
//! whose two halves fail in different ways. They are all here now: one name,
//! one construction, one cleanup, for all three.
//!
//! And they had already drifted: girsync's second copy was not a copy at all
//! but a `TempRoot` with a name of its own (`girsync_run_<pid>_<nanos>_<n>_<tag>`)
//! and a cleanup of its own, which is the sentence this crate exists to end.
```

---

## 2. The capture spec now contradicts itself about how many policies are shared

**Where:** `tools/capture/docs/00-spec.md:40`, `:42`, `:50`, and the new paragraph at `:60-66`.

**What it is.** §2's table now has both of these rows:

```
| No shared contracts, except the log policy | … |
| Two shared *leaf* crates: the log policy and the scratch-directory policy | … |
```

and the prose below still opens with:

> **The log policy is the one thing that *is* shared, deliberately.**

**Why it matters.** The table row was updated and the prose was not, so the document now says
"one thing is shared" and "two leaf crates are shared" three paragraphs apart. §2 is the
*non-negotiable* section of that spec — the part a reviewer reads to decide whether the capture
tool is still independent of the repair binary — so a contradiction there is not cosmetic. It is
also the section the commit message points at ("the capture spec's independence rule — which said
'one shared leaf crate' and now names two"), which means the intent was to update it, and the
prose paragraph was missed.

**Fix.** Three edits, all in §2:

1. **`:50`** — `**The log policy is the one thing that *is* shared, deliberately.**` becomes
   `**The log policy is the one thing about a *run* that is shared, deliberately.**`
   The sentence that follows it — "What stays independent is everything that could couple the
   *meaning* of a run: the exit codes, the report format, the fixtures, and every flag that is
   not about logging" (`:56-58`) — is what keeps row 40 honest, and it is still true. Keep it.
2. **`:40`** — the rule cell stays "except the log policy"; extend the "how it shows up" cell
   with one clause so the row cannot be read as denying the row below it: *the scratch-directory
   policy is a test concern and shares no flag, no exit code and no output of any program*.
3. **`:42`** — the row is right as written. Leave it.

---

## 3. The crate's own test asserts a naming invariant that the crate does not enforce

**Where:** `crates/tempdir/src/lib.rs:202-221` (the test `the_name_carries_the_prefix_the_tag_and_the_pid`)
against `crates/tempdir/src/lib.rs:76-86` (`unique_path`).

**What it is.** The test asserts the generated name splits into **exactly five** dash-separated
parts:

```rust
// `<prefix>-<tag>-<pid>-<nanos>-<seq>`, and nothing else.
let parts: Vec<&str> = name.split('-').collect();
assert_eq!(parts.len(), 5, "the name is {name}");
```

Nothing in `unique_path` prevents a `prefix` or a `tag` from containing a dash. The five-part
shape holds today only because every caller happens to pass dash-free strings — and one of them
builds its tag from data:

```rust
// tools/sync/tests/why/table.rs:220
let t = temp_root(&format!("why_{}", row.name.replace(' ', "_")));
```

The row names happen to have no dash today (`"size differs, mtime agrees"` and the rest), so the
invariant is currently true **by luck**. `unique_path`'s doc does not say the constraint exists:
it documents the shape as `<temp>/<prefix>-<tag>-<pid>-<nanos>-<seq>` and says nothing about what
`prefix` and `tag` may contain.

**Why it matters.** This is the one class of bug the crate was built to end: an invariant that
lives in a test rather than in the code, and therefore holds only until the first caller that
breaks it. A dash in a tag does not fail today — nothing parses the name — but the crate's own
test says the name is five parts "and nothing else", and that sentence is false the moment a tag
carries a dash. The name is a contract; the contract should be held by the code that makes the
name.

**Fix.** Pick one and say which in the commit message.

**(a) Hold it in the code — recommended.** `unique_path` normalises the two free-form parts, so
the name always has exactly five parts:

```rust
pub fn unique_path(prefix: &str, tag: &str) -> PathBuf {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = /* … */;
    // The name is a contract: `<prefix>-<tag>-<pid>-<nanos>-<seq>`, five parts,
    // split on `-`. A prefix or a tag that carries a dash would break the
    // shape, so the dash is folded away here rather than left to the caller.
    let prefix = prefix.replace('-', "_");
    let tag = tag.replace('-', "_");
    std::env::temp_dir().join(format!(
        "{prefix}-{tag}-{}-{nanos}-{seq}",
        std::process::id()
    ))
}
```

and `unique_path`'s doc gains one line: *a prefix or a tag that carries a dash has it folded to
an underscore, so the name always has exactly five parts.*

**(b) Weaken the test.** Assert that the name **starts with** `<prefix>-<tag>-` and **ends with**
`-<pid>-<nanos>-<seq>`, rather than that it splits into exactly five parts, and write the
constraint on `prefix`/`tag` into `unique_path`'s doc as a rule the *caller* owes.

Either is acceptable. (a) is the one that matches the crate's own argument.

---

## 4. `unique_path` is public API with no caller

**Where:** `crates/tempdir/src/lib.rs:76` (`pub fn unique_path`), and the crate row in
`README.md:42`, which advertises it:

```
| `crates/tempdir/` | the shared scratch-directory policy: … | `TempDir`, `unique_path` |
```

**What it is.** `rg -n unique_path` over the whole tree returns the definition, its three uses
inside `crates/tempdir` (including two in the crate's own tests), and the one README line. No
project calls it.

**Why it matters.** It was `pub` in the old copies, so keeping it `pub` was faithful to "the API
is the union of what the copies had" — but that reasoning applied to copies that had to be
*reachable* from a test binary, where every item was needed by whichever copy included it. As
the shared crate's advertised surface it is now a public API with no caller, and the README
lists it as one. A public item with no caller is not a bug, but it is an item the next reader
has to evaluate for no reason.

**Fix.** Pick one:

- **(a) Make it private** — `fn unique_path` — and drop `unique_path` from the README's crate
  row (`README.md:42`). The crate's own tests live in the same module and keep working. Nothing
  outside the crate needs it today.
- **(b) Keep it public and give it a job.** It is how a caller builds a path to hand to
  `TempDir::at` — the "the code under test picks the path" case — which is what `at`'s doc
  already gestures at ("Constructed from a `unique_path`"). If that is the intent, say it in
  `at`'s doc as a recipe, and keep the README row.

Today `TempDir::at` has exactly one caller, `tools/capture/tests/offline_bundle.rs:547`, and it
builds the path with `with_file_name`, not with `unique_path`. If no second caller appears,
(a) is the honest answer.

---

## 5. `TempDir::at`'s doc overstates what it removes

**Where:** `crates/tempdir/src/lib.rs:125-131`.

**What it is.** The doc says:

> **Anything already at the path is removed first.**

The code is:

```rust
pub fn at(path: PathBuf) -> Self {
    let _ = std::fs::remove_dir_all(&path);
    Self { path }
}
```

`remove_dir_all` removes **directories**. On a *file* at that path it fails, and `let _ =`
swallows the error — so a stale file survives, and the code under test meets a file where the
guard's doc promised a clear path.

**Why it matters.** `at` is the one constructor where the caller does not choose a fresh path:
the path is the one the code under test picks, which is exactly why the doc says the directory is
not created and the caller is trusted to make it. A guard that promises "clear" and leaves a file
behind is a guard whose promise is not kept — the same class as a drifted copy of a policy.

**Fix.** Either make the doc true, or make the code true:

- Make the code true: `let _ = std::fs::remove_file(&path); let _ = std::fs::remove_dir_all(&path);`
- Or narrow the doc to what the code does: *"Anything already at the path is removed, if it is a
  directory."*

The first is two lines and keeps the stronger promise; the second is one word. Either way the
doc and the code must agree — that is the whole point of the crate.

---

## 6. `TempDir::new` no longer removes an existing tree first, and nothing records the change

**Where:** `crates/tempdir/src/lib.rs:99-103`, against girsync's old `TempRoot::new`
(`tools/sync/tests/common/mod.rs`, before `17dfc98`).

**What it is.** The old `TempRoot::new` did:

```rust
std::fs::remove_dir_all(&path).ok();
std::fs::create_dir_all(&path).unwrap();
```

The shared `TempDir::new` does not remove first:

```rust
pub fn new(prefix: &str, tag: &str) -> Self {
    let path = unique_path(prefix, tag);
    std::fs::create_dir_all(&path).expect("create the temp dir");
    Self { path }
}
```

**Why it matters.** It is safe — the name is unique by construction (pid + timestamp +
per-process counter), so there is nothing there to remove — but it is a **silent semantic change**
for the one suite that had the old behaviour, and the change is not written down anywhere. The
next person to read `TempDir::new` cannot tell whether the removal was dropped on purpose or
forgotten. (It was right to drop it: a "remove first" on a shared path would delete another
run's tree, which is the exact failure mode this crate's name is built to prevent.)

**Fix.** One line in `new`'s doc, and nothing else:

> The path is unique by construction — pid, timestamp and per-process counter — so there is
> nothing at it to remove: the directory is created, not cleared.

If the agent would rather have the belt and braces back, `let _ = std::fs::remove_dir_all(&path);`
before `create_dir_all` restores the old behaviour exactly — but then the doc has to say *why* a
remove is safe on a path that is unique. Either is fine; the silence is not.

---

## 7. Housekeeping

- **Delete this file** when the fixes land, in its own commit, exactly like
  `docs/10-artifact-test-scope-review.md` was deleted at `4f36b67`. Nothing should refer to it
  afterwards — not the code, not the documents, not the comments.
- **Do not cite this note** from a commit message, a source comment or a document. A finding that
  needs a permanent home moves into the document that owns it: §1 into
  `crates/tempdir/src/lib.rs`'s module docs, §2 into `tools/capture/docs/00-spec.md`, §3 into
  `crates/tempdir/src/lib.rs` (the invariant, in the code, with the test agreeing), §4 into
  `crates/tempdir/src/lib.rs` and `README.md:42`, §5 and §6 into the doc comments of the two
  constructors they are about.
- **The two problems this note does not cover** — the tests that scan `target/debug/logs/`, and
  the `gircap` tests that walk the system temp directory — are tracked separately in `TODO.md` at
  the root of the repository. They are not part of this refactor and were not caused by it.
