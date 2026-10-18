# `girsync` fixup plan — temporary working document

**This file is temporary and will be deleted once the work below is done.**

It exists so the fixes are written down rather than half-remembered. It is not
part of the crate, nothing in the crate should come to depend on it, and
nothing that lands should refer to it — not in a doc comment, not in a module
doc, not in a commit message, not as a `Refs:` trailer. If a change below is
made, this file is stale by definition; delete it rather than update it.

**Known-good baseline you are starting from** (measured on this branch, HEAD
`a0543a0`):

| check | result |
| --- | --- |
| `cargo test -p girsync` | 284 tests, 283 run, 1 `#[ignore]`d, all green |
| `cargo clippy -p girsync --all-targets` | clean, zero warnings |
| `cargo fmt -p girsync -- --check` | clean |

If any check below comes back different, that is your change, not a flake.
Re-run twice before believing a failure.

---

## How the work is grouped

Three commits. Two carry behaviour-relevant change; the third is pure
housekeeping and should be reviewed as such. Steps 1 and 2 are independent and
may land as one commit if a single logical change reads better that way.

| # | commit carries | why it is separate |
| --- | --- | --- |
| 1 | §A | the only required change; needs its test written first |
| 2 | §B | two call-site swaps plus one doc sentence; no new behaviour |
| 3 | §C | comments and whitespace only |

Order matters only in that §B's second half wants §C's trailing-newline pass
not to have run yet, so do §C last.

---

## §A — pin that a copy's verify is still a check (required)

### The problem

The copy phase streams src to dst while digesting the bytes going past, then
re-reads dst from disk after `sync_all()` and digests that, and compares. The
whole point of keeping the re-read is that the two digests come from **two
independent reads of two different files**, so a disagreement means something.
If both sides were ever digested from the same chunks, the comparison would be a
value against itself — equal by construction, unable to fire, and reading like
a guarantee while checking nothing.

Nothing in the suite tests that. `grep` for `verify hash|verify size|verify
mtime` under `tools/sync/` returns hits in `src/` only — no test in `src/` or
`tests/` drives `copy_one`'s verify down its failure path. Every copy in the
suite is byte-correct, so a compare written as `src.get(a) != src.get(a)` would
pass all 284 tests unchanged.

The existing tests do pin real things and keep them:

- `the_digest_of_a_copy_is_the_digest_of_the_file`,
  `no_digest_requested_still_copies_every_byte`, `an_empty_file_copies_and_hashes`
  — pin `copy_and_digest`'s own output on bodies larger than one buffer.
- `a_digest_is_the_same_however_the_bytes_were_split` and its two siblings —
  pin that `Digests` and `hash_file` agree however the bytes were divided.
- `tests/migration_lock.rs` (`cached hash matches hash_file`, and the
  `hash_file(&dst.join("a.txt"), …)` assertion) — pin that the row a copy
  caches is dst's real on-disk digest, not the stream's.

What none of them pin is that `copy_one` **compares two things**. That is the
gap this section closes.

### The change

`src/commands/sync/apply.rs` — lift the comparison out of `copy_one` into a
free function that takes both digests as separate arguments, so it can be
called with a disagreement:

```rust
/// Compare the two digests a copy produced, and fail if they disagree.
///
/// Two *separate* arguments, and that is the whole point. `src` is the digest
/// of the bytes the copy streamed; `dst` is a fresh read of the file on disk,
/// taken after `sync_all()`. A single pass that digested both sides from the
/// same chunks would hand this the same map twice and it would have nothing to
/// say — which is why the signature takes two, and why a caller that cannot
/// produce two has no verify to run at all.
///
/// Only the requested `algos` are judged. A digest the run did not ask for is
/// not evidence about this copy either way.
fn verify_digests(
    path: &Path,
    algos: &[String],
    src: &HashMap<String, Vec<u8>>,
    dst: &HashMap<String, Vec<u8>>,
) -> Result<()> {
    for a in algos {
        if src.get(a) != dst.get(a) {
            error!(algo = %a, path = %path.display(), "verify hash mismatch");
            bail!("verify hash({}) {}", a, path.display());
        }
    }
    Ok(())
}
```

`copy_one` then reads:

```rust
    let dh = hash_file(&d, algos).with_context(|| format!("hash {}", d.display()))?;
    verify_digests(&d, algos, &sh, &dh)?;
```

Keep the two user-visible strings byte for byte as they are —
`verify hash mismatch` on the `error!` and `verify hash({algo}) {path}` on the
`bail!`. They are stderr contract.

Move the existing explanatory comment with the code, not with the call site; it
is about what the two arguments *mean*, and it reads wrong next to a call that
no longer shows both digests arriving separately.

### The tests

Write these **before** the swap, and watch them fail against today's
`copy_one` (they will not compile against it — that is the point: the shape
does not exist yet). Put them in the existing `mod tests` at the bottom of
`apply.rs`, next to the three `copy_and_digest` cases.

1. `a_disagreement_between_the_two_digests_is_a_failure` — same key set, one
   digest differs. Assert `is_err()`, and that the message contains
   `verify hash(md5)` and the path. This is the test the whole section is for.
2. `agreement_is_not_a_failure` — the two maps equal, `algos` non-empty,
   `Ok(())`. Otherwise test 1 passes for the wrong reason.
3. `a_digest_that_only_one_side_has_is_a_failure` — `src` has `md5`, `dst` does
   not. `Some(..) != None` must fail; a hand-rolled `for (k, v) in map` version
   would skip it and silently pass.
4. `only_the_requested_algorithms_are_judged` — `algos == ["md5"]`, the maps
   disagree on `sha256` only, `Ok(())`. Pins that the loop walks `algos` and not
   the maps.

No integration test is added here. Making a real copy fail its verify
deterministically needs dst to change between the write and the re-read, which
a test cannot arrange without a seam in `copy_one`. Say so in the commit body
rather than implying the gap is fully closed — this closes the
"compare-is-a-tautology" half, not the "both-digests-from-one-read" half.

### Verify

```
cargo test -p girsync apply::tests
cargo test -p girsync
cargo clippy -p girsync --all-targets
cargo fmt -p girsync -- --check
```

---

## §B — use the key-only cache read where it was built to be used

### B1 — `prune_dst_cache` decodes every row to look at none of them

`src/commands/sync/apply.rs:419`:

```rust
    for rel in w.load_all()?.keys() {
```

`.keys()` is the only thing read. This is the same shape the key-only read
exists for, on the same dst table, in the same final-apply pass a real sync
runs once. Swap it:

```rust
    for rel in w.load_keys()? {
```

One mechanical follow-on: `rel` is now an owned `String` instead of a `&String`,
so the `w.remove(rel)?` inside the loop becomes `w.remove(&rel)?`. Nothing else
in the body changes — `rel.len()`, `rel.starts_with(p.as_str())` and
`present.contains(rel.as_str())` all read the same on either type.

Behaviour is unchanged. The two cases that already pin this function's answer
are in `tests/sync.rs`: `rows_for_a_subtree_a_copy_wiped_are_dropped` (the
prefix subtraction) and `rows_for_a_directory_created_and_removed_in_one_run_
are_dropped` (`#[ignore]`d, the known case-sensitive limitation). Both assert
against the cache, not against the run's output — leave them that way.

### B2 — `compare-self` decodes the record table twice in a row

`src/commands/compare_self.rs:153`:

```rust
        entries = record.load_all()?.len(),
```

That decodes every `FileRec` in the record purely to fill an `info!` field that
is usually not printed at all — and then `load_record_side_from` decodes the
entire table again on the very next line (`src/effective/record.rs:35`). Swap
it:

```rust
        entries = record.load_keys()?.len(),
```

The handle is read-only; the key-only read is a read, so it is allowed there.
Behaviour is unchanged, and so is what a corrupt record does: the decode that
used to fail on the counting line now fails on the line below it, which is the
one that builds the side. Nothing is detected one step later than before.

### B3 — the doc sentence that contradicts its own test

`src/cache/db.rs:229`, on `load_keys`:

> The answer is the same key set in the same order

`load_all` returns a `HashMap`, which has no order, and the test directly below
deliberately compares as a set because "the order is redb's business and is not
part of what a caller may rely on". Pick one and make the doc agree with the
test — the test is right. Something like: the same key set; order is redb's
business, membership is the contract.

`src/cache/write.rs`'s `load_keys` doc has no equivalent claim, so it needs
nothing.

### Verify

```
cargo test -p girsync
cargo test -p girsync -- --ignored   # the known-limitation case still behaves
cargo clippy -p girsync --all-targets
cargo fmt -p girsync -- --check
```

---

## §C — housekeeping the last two commits left behind

No behaviour. Review as one mechanical commit.

1. **`src/diff/mod.rs`** — the merge dropped the blank line between
   `hashes_differ`'s closing brace and `#[cfg(test)] mod tests`. Put it back.
   rustfmt does not care either way; it is the only thing the merge resolution
   changed that nobody asked for.

2. **`src/testtemp.rs:6`** — the doc links `[`crate::tests_common`]`, which is
   a module name from the other crate in this workspace. girsync's `lib.rs` has
   no such module, so the link resolves to nothing. It never fires, because the
   whole file is `#[cfg(test)]` and `cargo doc` never builds it — which is
   exactly why it has survived. Drop the link syntax and name the integration
   helper in plain prose, or name its actual path.

3. **`src/testtemp.rs:8`** — "the two in-crate test sites below get this
   instead". There are now **nine** call sites across four files:
   `src/cache/db.rs` ×2, `src/commands/sync/rename.rs` ×1,
   `src/commands/sync/apply.rs` ×3, `src/hash.rs` ×3. Make the sentence say
   that, or say something that does not need a number to stay true.

4. **`src/testtemp.rs:66`** — `TempDir`'s doc says only "Derefs to [`Path`]",
   but the type also implements `AsRef<Path>`. The sibling copy in
   `tools/capture/src/testtemp.rs` says both; this one was not updated when the
   impl landed.

5. **`tools/sync/README.md`** — no trailing newline. It has not had one since
   before the merge; add it while you are here.

6. **Optional, `tools/sync/REFACTOR_PLAN.md`** — same missing trailing newline,
   and its closing tally says "283 tests, 282 run". The real numbers are
   **284 / 283 run / 1 ignored**. The per-step deltas in that table are all
   correct (+6, +2); the post-merge baseline it quotes (275) is one low, which
   is where the drift entered, and every total after it inherits the error.
   Fix the numbers and the newline, or skip both — the file is on its way out
   and a separate piece of work is going to remove it and the references to it.
   Either way, do not spend effort on that removal here.

7. **Explicitly out of scope.** `cargo fmt --all -- --check` is dirty across
   roughly two hundred files in `gircap` and the root crate. That is the other
   branch's mess, not this crate's, and `cargo fmt -p girsync -- --check` is
   clean today. Leave it alone.

---

## §D — optional: put a number on the copy change

Not a fix, so it is listed last and can be dropped whole.

The copy phase used to be `std::io::copy` followed by two full re-reads; it is
now a 512 KB user-space loop that digests as it writes, followed by one re-read.
The I/O count is genuinely lower — 2 reads + 1 write instead of 3 + 1 — but on
the hashing path that loop also gave up whatever fast path `std::io::copy` had
between two files, and the `--hash none` path still has it. An I/O-count
argument is not a speed argument, and no measurement exists.

If you want the number: build the binary at `48ce583`'s parent (`7792a38`) and
at `HEAD`, generate identical synthetic trees (N files of one fixed size, mtimes
matched), drop the OS file cache or accept the noise and take best-of-5, and
time a converged `sync` at several N. Report the table; if the change is a
wash, say so — that is a result too, and a more useful one than the I/O count.

---

## Definition of done

- `cargo test -p girsync` — 284 tests, 283 run, 1 `#[ignore]`d, zero failures.
  The count goes up by four (§A's four new cases): **288 total, 287 run**.
- `cargo clippy -p girsync --all-targets` — zero warnings.
- `cargo fmt -p girsync -- --check` — clean.
- No file anywhere in the repo refers to this document, by name or otherwise.
- This document is deleted.

## What this plan deliberately does not ask for

- No new cache API beyond what already exists. `load_keys` is on both handles
  and does the job; §B is calls, not additions.
- No change to the rw limiter, the src→dst lock order, or anything under
  `src/rw/`. That is the one part of this crate where a wrong edit hangs a run
  rather than failing one.
- No change to the CLI surface, the stdout record vocabulary, exit codes, or any
  documented behaviour in `README.md`.
- No attempt to close the "both digests from one read" half of §A's gap. It
  cannot be closed deterministically without a seam in `copy_one`, and adding a
  seam to production code to enable a test is a worse trade than stating the
  limitation.
- Nothing about the eventual deletion of the crate's own planning documents, or
  the references to them. Separate work, separate session.