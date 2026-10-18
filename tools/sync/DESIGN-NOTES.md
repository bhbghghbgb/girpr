# girsync — design notes

Reasoning that fits nowhere else. `README.md` is the tool: what a flag means,
what a run prints, what the gotchas are. This file is for the things a reader has
to know *before* changing something and would otherwise have to re-derive — the
invariants that are enforced by structure rather than by test, the changes that
look like tidy-ups and are not, and the few places where the code is knowingly
worse than it could be.

Nothing here is a record of a change. If a statement is about what the code *is*,
it is true; if it would only be about what the code *was*, it belongs in `git log`
and not here.

---

## Load-bearing invariants, and what enforces them

The dangerous edits in this crate are the ones that break an invariant nothing
fails loudly about. Four of them.

### The verify needs two independent reads

`copy_one` digests src as the copy streams it, then **re-reads and re-hashes
dst** and compares. Two reads, not one.

The tempting optimisation — digest both sides from the same chunks going out — is
not a weaker check, it is *no* check. Both digests would come out of one loop over
one buffer, so they are equal by construction and `verify hash mismatch` can never
fire. It would read as a guarantee while asserting `x == x`.

Two independent reads are what make the message mean anything, and because the
second happens after `sync_all()`, it is a digest of the bytes that actually
reached the disk — which is the only way a copy catches silent corruption on the
way down. `--hash none` takes a different arm and keeps `std::io::copy`'s fast
path, since there is no digest to justify giving it up.

**Enforced by:** `verify_digests` takes two maps and is unreachable with one;
`a_disagreement_between_the_two_digests_is_a_failure` hands it two maps that
disagree, and `a_digest_that_only_one_side_has_is_a_failure` covers the `Some !=
None` case a loop over `src`'s keys would skip.

### src → dst, always, for lock ordering

A copy takes both rw permits; it takes **src first**, and a thread holding dst
takes nothing further. Every wait-for edge therefore points forward along
`src → dst` and the graph is acyclic.

`rw/gate.rs:acquire_copy` is the only place in the crate where a wrong edit can
*hang* a run rather than fail one. A copy waiting for dst while holding src is
the only operation that ever holds one gate while waiting for another, so changing
the order — or letting a third gate into the set — is the difference between a
failing test and a hung run. `RwRuntime::new` sizes the pool at the largest number
of permits a run can hold at once, so no worker usually blocks on the gate at all;
that is a throughput property, not the correctness argument.

**Enforced by:** the ordering itself plus `RwPermit`'s reverse-order drop;
`permits_are_released_on_panic` covers the release path under unwinding.

### The pre-drop is belt-and-braces, and the note about it is load-bearing

Cache entries for every path a run will touch are dropped **and committed**
before the corresponding filesystem change. That is what makes a crash re-copy
rather than trusting a half-written file.

It is *not* the mechanism. Truncation makes a partial write strictly smaller;
mtime is stamped only after a complete write; a verify failure leaves matching
stat but content that differs from the row's digest. The pre-drop stays because it
costs one batched write and depends on none of those.

It becomes load-bearing the moment `copy_one` stops truncating, stops deferring
mtime, or **reuses a digest across a content change** — which is exactly what a
single-pass copy would do. Do not "optimise it away" without checking those three.

### The plan must precede the rename, and the two pairings must differ

`sync` pairs its two sides as they are *on disk* (`CasePairing::Lowercase` in
insensitive mode) and then diffs *after* the rename pass has re-keyed dst
(`CasePairing::Exact`). The two values must differ; they are deliberately passed
as two named values rather than two bare booleans precisely so that a reader
sees the disagreement at the call site.

Getting either half wrong is silent and damaging:

- Planning **post**-rename with exact pairing makes a case-only pair look
  one-sided, skips the digest it needs, and leaves the diff comparing two entries
  where one has no digest. `hashes_differ` is silent on a missing digest, size
  and mtime agree, and the run reports success having copied nothing.
- Diffing **pre**-rename resurrects the `CASE-MISMATCH` bucket the rename pass
  exists to eliminate.

Every other case-only fixture in the suite uses identical bytes and would pass
either way. The one that does not is
`tests/sync.rs::a_case_only_difference_in_content_is_copied_not_merely_renamed`
and its row in `tests/sync_plan/`. If you add a case-only fixture, give the two
sides **different bytes at the same length** or it tests nothing.

---

## Not on the table

Each of these looks like an opportunity to remove duplication. None of them is one.

**Do not merge `scan_stat_only` and `load_record_side_from`.** They are not two
variants of one walk: one reads a directory, the other reads a table. Merging
them would make the record path look swappable with the disk path, which is
exactly the mistake the record path exists to prevent — and `compare-self` is what
that separation buys. `effective/record.rs`'s own module doc says so.

**Do not turn `Side` / `RwSide` into a trait.** Two variants, one dispatch site
(`effective/side.rs`). A trait here would be ceremony with no second
implementor, and the enum's shape is what makes the two spellings in
`case_sensitive` and `case_insensitive` readable at a glance.

**Do not touch the `--rw-threads*` family, `RwLimiter`, or the lock order.** See
the lock-ordering section above. The tests are good (`rw/gate.rs`'s unit tests
plus `tests/rw_threads/limiter.rs`); leave them alone.

**Do not weaken `plan_pairs`' coverage check or `pair_verdict`'s `unverifiable`
clause.** Together they close the last path by which a run can report a confident
answer about content nobody read. `hashes_differ` skips an algorithm either side
lacks, so without them an uncovered pair falls back to size+mtime — which had
already agreed — and the run reports `CHANGED = 0` about a file it never opened.
That is a **false clean**, the worst outcome this tool can produce, and the
failure mode of a regression here is invisible: the counters look fine, the rows
look fine, only the verdict is wrong.

---

## Known limits, deliberately not fixed

Each is cache hygiene or a race, not a wrong answer. All are stated in `README.md`
where a user can meet them; recorded here so a reader deciding whether to "fix"
one knows what the trade was.

**A row for a directory created and removed in one run survives until the next
prune.** Reachable only with `--case-sensitive` on a case-insensitive filesystem:
`mkdir Sub` and a later `rmdir sub` name the same directory. `prune_dst_cache`
derives its present set from the plan and the dst map rather than asking the
filesystem, so it cannot see this without case-folding the prune against the plan
— in the one mode that deliberately does not case-fold. `compare-self` builds a
cold cache, so it cannot be misled. Pinned as an `#[ignore]`d test with the reason
in `tests/sync.rs`.

**A src file rewritten *during* its own copy is not caught.** dst is then a
faithful copy of a src that no longer exists, and the pair compares clean. The
race is unwinnable by construction — phase A's stat sees the new mtime on the next
run and re-copies — so it self-heals, but the run that did it reports no error.
This is the price of not re-reading src, and it is recorded in `README.md`'s
"What a copy costs", in `copy_one`'s doc, and in `predrop_cache_entries`.

**A stat-only row can get stuck.** A size-changed row arrives in phase A with its
digests dropped, and a size difference settles the pair for free, so nothing ever
rehashes it. `--no-trust-size` is the repair. The row looks healthy — right size,
right mtime — which is what makes it worth stating.

---

## Two things that are one

**"What must be computed" and "can the pair be answered" are different questions,
and only one of them honours `no_trust`.** A side's availability is `cached ∪
hashable` with deliberately no trust term: `--no-trust-cached-hashes` asks a side
to re-read rather than reuse, and a record has no filesystem to re-read, so
folding trust into coverage would make the flag fatal for every record holding
exactly the right digests. `planner/mod.rs` carries the table; do not merge the
two.

**The diff and the planner must agree about the short circuit.** Both read
`StatTrust::settles`, and the planner's decision travels out in `PairPlan::stat`
rather than being re-derived from the flags. If they disagree, one of two *silent*
things happens: the planner hashes a pair whose digest nobody reads, or — worse —
a pair the planner called settled turns out to have needed a digest nobody
computed. There is deliberately no `untrusted_disagrees() -> bool` next to
`untrusted_differs`: a second spelling of a rule is a rule that can drift, and the
caller that wants a bool asks `.any()`.