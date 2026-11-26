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
fails loudly about. Several of them.

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
way down. A failed `sync_all` therefore aborts the copy: ignoring it would make
the second read come from the same page cache the flush never wrote through.
`--hash-all-of none` takes a different arm and keeps `std::io::copy`'s fast
path, since there is no digest to justify giving it up.

**Enforced by:** `verify_digests` takes two maps and is unreachable with one.
The enforcement tests cover a disagreement between the digests, a digest only
one side has (the `Some != None` case a loop over `src`'s keys would skip), and
the canonical algorithm named on failure — read both from the error *text* and
from the error **event**'s `algo` field, which is the other channel a parser can
read.

### src → dst, always, for lock ordering

A copy takes both rw permits; it takes **src first**, and a thread holding dst
takes nothing further. Every wait-for edge therefore points forward along
`src → dst` and the graph is acyclic.

`tools/sync/leaves/src/rw/gate.rs`'s `RwLimiter::acquire_copy` is the only place
in this workspace where a wrong edit can *hang* a run rather than fail one. A copy
waiting for dst while holding src is the only operation that ever holds one gate
while waiting for another, so changing the order — or letting a third gate into
the set — is the difference between a
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
single-pass copy would do. Do not "optimise it away" without checking those conditions.

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

Case-only fixtures with identical bytes would pass either way; the
difference-in-content coverage in `tests/sync.rs` and `tests/sync_plan/` gives
the two sides different bytes. If you add a case-only fixture, give the two
sides **different bytes at the same length** or it tests nothing.

### The fold sites, and the questions each one answers

**The path key folds ASCII case only.** `util::path_key` is the key every
relative-path index uses — the mixed-case guard, the diff's pairing, the
rename pass, and the plan-time lookups — so the fold is stated once and every
index agrees on it. ASCII-only is deliberate: a Unicode fold maps the Kelvin
sign `K` onto `k`, and Windows keeps those two spellings apart, so the fold
would pair two files that are not one. The rule is "fold ASCII case, compare
every other byte exactly", so a non-ASCII case variant is a distinct key —
deliberately distinct even where the filesystem would have merged the two
spellings. The planner's pairing is the one index that holds the key in its
borrowed form, `util::FoldedKey`: the same rule, as a key that borrows the
path it folds, so the index holds the paths it indexes rather than a folded
copy of each.

**Path identity is not the path key.** `util::component_eq`, behind
`util::same_path` and `util::path_within`, keeps a Unicode lowercase
fallback, because on Windows the filesystem really does treat `Ärger` and
`ärger` as one file. The two answer different questions: the key asks which
bucket of a run's map a path belongs to, and identity asks whether two names
open the same place on disk. They are allowed to disagree, and that is the
point — on Windows `Ärger` and `ärger` are two keys and one file. Unifying them
would change what a run does: a Unicode key merges spellings the disk keeps
apart, and an ASCII identity rule stops the overlap and containment checks
from seeing a case difference Windows really has.

**The on-disk spelling lookup is neither.** `on_disk_name` in
`src/commands/sync/plan/link.rs` asks which directory entry the OS would open, so
it compares Unicode-lowercased names. It is a question about the filesystem,
not about how a run keys its map, and it follows neither rule above. The
lookup and the path key live in different modules — the lookup in
`src/commands/sync/plan/link.rs`, the key in `util` — and each answers its own
question.

**One mapping, one comparison, two questions.** The Unicode lowercase mapping
the on-disk lookup compares with is the one the identity rule's fallback
compares with (`util::unicode_lowercase`, through the comparison
`util::unicode_case_eq`), so the mapping and the comparison that applies it
are written once, in `util`. What is shared is the mapping and the comparison,
not the question: the identity rule is an equality relation between two
spellings, and the lookup is a choice among the entries of one directory — an
exact match wins, and two entries that fold together are ambiguous rather than
equal. Each site says which question it asks, and the ASCII-only path key
stays a rule of its own.

**The cache reservation is another site, and it folds the path key's way.**
`util::has_cache_prefix` asks a narrower question than any of the others above:
is this path component one of the tool's own names (`girpr-cache` and its
copies)? The prefix it matches is ASCII, so the ASCII-only comparison
(`util::ascii_case_eq`) states it without borrowing the identity rule's Unicode
fallback — that rule answers whether two spellings open the same place on disk,
which is not the question here. On POSIX neither fold applies at all: the two
spellings are distinct entries, so the reservation is an exact prefix match
and `GIRPR-CACHE` is ordinary content.

**The reservation's rules each state that comparison.** The prefix
helper `util::has_cache_prefix` reserves the whole `girpr-cache*` prefix, and
its exact-name counterpart `util::is_cache_name` asks whether the component is
the cache's own name, `girpr-cache`, rather than one of its copies. Both ask
it with `util::ascii_case_eq` on Windows and with an exact match elsewhere, so
a reader who follows one finds the other.

**Enforced by:** the enforcement tests cover the key's ASCII-only fold and its
borrowed form, identity's wider fold and its deliberate disagreement with the
key, the reservation's case folding on Windows only and its exact-name
counterpart, the planner's use of the key (including non-ASCII lookalikes), and
the on-disk lookup across case-differing components, aliases, mirrored targets
and terminals — including the identity rule and the lookup agreeing on a
non-ASCII spelling, the two sites that share the mapping compared with each
other.

### The cache's algorithm vocabulary is open; the request's is not

`HashAlg` is a closed enum and the parser refuses everything else, so
what a run may **ask for** is settled at compile time. What a cache row **holds**
is not: a cache written by a newer build may carry a digest under a name this one
has never heard of, and that row has to stay readable. `tools/sync/cache/src/wire.rs`
therefore keeps algorithm names inline rather than behind an id registry, and `DigestSet`'s
custom deserializer never refuses one.

The in-memory end of the same guarantee is `DigestSet`'s `foreign` arm, and it has
to be more than decode-and-ignore. An unknown name is **carried**: phase A hands it
to the planner, `merge_row` folds it into the next row, phase C writes it back. A
scan that dropped it would quietly lose the row's contents on the next write, which
is the failure a "we don't understand it, so we leave it alone" reading produces.

A *known* name carrying an **unexpected length** is carried too, and for a
different reason. That is a corrupt or hand-written row. It compares byte-wise
against whatever else claims the same name and so reports a difference, and
routing it through `foreign` preserves that; rejecting it would turn a reported
difference into a hard failure, which is not what this format has ever done with
one. `DigestSet::insert_named` is where the routing lives, so the rule cannot be
spelled two ways.

**Why `kind` is the opposite.** `FileRec.kind` *is* a closed registry in the wire
format and its deserializer refuses an unknown spelling on purpose: a new kind
would break older binaries that cannot represent it, so failing loudly is the
right answer.
The two look inconsistent and are not. Do not "fix" the asymmetry, and do not
convert `kind` to an enum on the strength of `HashAlg` being one — the enum is what
makes the *request* cheap to hold; inline names are what make the *cache* survive.

**Enforced by:** the codec's preservation of unknown algorithms, checked in a
round trip and across a whole scan; the `merge_row` case where the rule has to be
visible to `merge` after a stat change; and
`a_digest_set_is_eighty_bytes_and_allocates_nothing`, which fails if a later
edit puts a map back — a map passes every other test here and adds allocations
to every row.

### The effective source is checked against dst before a plan exists

Roots can be siblings and still overlap through the walk. `walk_live`
follows links, so if `src/link` resolves to `dst` (or to a directory containing
`dst`), one physical file appears both at its real dst path and under `link/`.
The real path then looks `EXTRA`, the delete pass removes it, and the copy that
needed it fails afterwards: data loss reported as a copy error. The root check
in `validate_inputs` cannot see this, because it canonicalizes only the roots.

`validate_src_links` therefore canonicalizes every link the **effective** src
map kept and refuses an intersection with dst in either direction. It runs after
src's phase A — so `--include`/`--exclude` and `--max-depth` have already
decided what this run's source is — and before dst is scanned, planned, renamed
or applied. A link to anywhere else is ordinary mirrored content, including one
that points back inside `src`.

`validate_dst_links` is the same check in the other direction, run immediately
after dst's phase A. A dst link that resolves into `src`, or contains it, is
refused before the plan, the rename or any apply phase. The dangerous shape is a
final-component file link at a copy path: `File::create` follows it, truncates
the source file, and the verify then reads the same file back through the link,
so the run could exit 0 having destroyed its own input. The check is about where
the link resolves, not about what this run's plan would do with it, so a dst-only
link into `src` is refused even though unlinking it would be safe. `copy_one`
re-asks the filesystem at the final component before it unlinks or creates
anything, so a plan and a disk that disagree still cannot reach `src`. A dst link
to anywhere else — including one inside `dst` itself — keeps the documented
write-through.

The removal passes do not rely on that check alone. `guard_removal_outside_src`
canonicalizes each delete target's **parent** and refuses one that resolves
inside src; the final component is deliberately not resolved, so a link at the
target is removed as a link and its target survives. That is the second line
because a plan is data: a later phase that invents a delete must not be able to
remove the file the run is copying.

**Enforced by:** the enforcement tests cover src links into dst (directory,
inside, containing and file-link shapes) and dst links into src (file and
directory shapes), plus a removal whose parent resolves inside src. Host
coverage varies: the junction cases run on Windows, and the file-link cases
may skip where the host refuses the primitive.

### A filtered dst blocker is refused before the rename

`--include`/`--exclude` hide entries from the effective dst map (`dm`) but not
from the disk. A file that the filter hides can still be sitting exactly where
the plan will `mkdir`, so `build_plan` sees no type conflict and
`ensure_dirs_applicable` — which reads `dm` — sees no blocker. The dry run then
prints a `MKDIR`/`COPY` plan that the real run cannot execute: `create_dir_all`
fails with `AlreadyExists` after earlier phases have changed dst.

`ensure_mkdir_targets_free` closes that gap before the rename pass — a case-only
rename is itself a filesystem change, and a refusal must not follow one. It
derives the same mkdir candidates `build_plan` will:
src directories that the post-rename `dm` does not hold (a pair the path key
merges in insensitive mode, because the rename collapses those), and stats each
candidate and every strict prefix on disk. An existing directory is fine; a
non-directory is refused unless it is a planned `fix_dirs` entry, which runs
before the nested mkdir. Every strict ancestor of a src directory is itself a
src directory, so the blocker is normally its own candidate. `dm` is deliberately
not widened to retain non-directory ancestors: that would make the planner
delete an entry the filter promised to leave alone.

**Enforced by:** the enforcement tests cover a filtered file at a mkdir path, a
non-directory at a strict prefix, a planned fix at a strict prefix, and a
filtered dst file blocking a mkdir being refused before the rename (which also
pins that no case-only rename precedes the refusal).

### A hard-linked destination is refused at plan time, not written through

`try_is_link` answers only for reparse points, so a dst file that is another
name for a src file is ordinary content to the walk. `File::create` truncates
the inode, not the name, so a run would report success after destroying the
other name; the verify then reads the changed inode back and agrees with
itself. The decision is taken at plan time, in both modes, by
`ensure_hard_linked_targets_safe`, so a dry run refuses the same tree instead
of reporting a `COPY` the real run will not perform. `copy_one` keeps its own
runtime check.

The count, not an identity comparison with the copied source, is the check that
covers the reported shape: `dst/a.txt` can be a hard link to `src/b.txt` while
the copy is reading `src/a.txt`. A name the plan removes before the copy — a
`delete_files` extra, or a file `fix_dirs` replaces — is credited, because the
copy then truncates the only remaining name. Unlinking a dst **link** is not
credited: it removes the link, not a name for the inode. Otherwise the refusal
is conservative by design — a surviving shared inode is outside a mirror's
write contract, wherever it lives — and the documented dst **symlink**
write-through is unchanged, because a symlink is a different directory entry
rather than a second name for the truncated inode.

**Enforced by:** a dst hard link is refused without touching the shared inode
(also under `--hash-all-of none`), the refusal is the same decision in both
modes, the plan-time preflight cases, and the count and identity cases in the
link-classification leaf.

### A kept dst file link's target is its content, not an extra

The walk follows a file link, so `dm` holds the link at its own path and its
target at the target's path when the target is inside `dst`. The two names are
one file. The delete plan cannot know that from the map alone: the target looks
`EXTRA`, so `delete_extras` removes it before the copy phase, and the copy then
canonicalizes a dangling link (`NotFound`, exit 3) — or, with equal content and
no copy, the run exits 0 leaving the link dangling and the next walk aborts.

`spare_link_storage` answers this at plan time, in both modes, from the disk.
**The dst root is resolved at plan time**, before the pass decides anything —
and it is resolved whether or not the tree holds a kept link: every path under
it is compared with the canonical root, so a root that cannot be resolved
leaves the whole plan unanswerable, and the pass refuses it there rather than
letting a run act on a plan it could not check. For every `dm` file link that
`src` also has as a file (so the link stays), it walks the `read_link` chain. An
intermediate link is part of the chain and is spared too. Every link
**component** of the target path is spared as well — the walk's alias spelling
(`link/x`) and the canonical one (`real/x`) both, because the delete plan may
name either — and every spared entry goes through `keep_path`, so all its
ancestor directories are spared with it as well. A final target inside `dst`
that `src` does not have is the link's content: it is dropped from
`delete_files`, from the diff's `extra` bucket (so the printed record and the
action name the same set), and from `rmdir` together with every directory above
it. A final target `src` **does** have is one inode carrying two mirrored
paths; if either end is planned for copy, the plan is refused before any change,
because one file cannot hold two contents and the parallel copy phase could
interleave them. A target the plan replaces with a directory is refused for the
same reason. A target outside `dst` is the documented write-through and is not
the plan's business.

Kept links can resolve to one terminal even when neither target is
itself mirrored by `src` (`a -> c` and `b -> c`, or the chained `a -> b -> c`).
`ensure_shared_storage_safe` judges that set as a whole by terminal storage: a
plan that would copy any member is refused before the plan print and before any
mutation, while a group with no copy writes nothing and is left alone.

The target is named in the spelling the walk reported. Its final component is
deliberately not resolved — it may be the chain's next hop — so its spelling is
read from the parent directory's listing, case-insensitively on Windows only
(the component rule of `util::same_path`); a link text with different casing
therefore cannot miss `sm`/`dm`, the spare or the grouping key. In a dry run the
link is still at its pre-rename spelling on disk, while `dm` and the plan use
src's casing. `spare_link_storage` stats the on-disk spelling and maps the
canonicalized target back through `DiskSpelling::plan_rel`, the same path map
the other plan-time guards use.

**Enforced by:** the enforcement tests cover write-through of a dst file link,
keeping a link's target when no copy is needed or it sits in a dst-only
directory, refusing a target that src also mirrors, sparing a kept link through
a dst directory link, and judging linked files that share storage as a set —
including chained links (file-link cases self-skip on a host that refuses
`symlink_file`). The plan-time cases cover dropping a spared target from every
removal list, naming the target relative to the dst root, case-differing
components, aliases, terminals and mirrored targets, a case-only rename of the
target, and refusing an unresolvable dst root.

### A digest is looked up by name, not by slot

`DigestSet` has `get_algo(HashAlg)`, which reads the slot *and* the foreign arm;
`get_named(&str)`, which asks by name; and `get(HashAlg)`, which reads the slot
and nothing else. The first two answer the usual question, and they agree.

**Every caller asking "does this row hold algorithm X" must use a name-aware
lookup, not the slot-only `get`** — `obtainable` and `pick_one` in the planner,
`HashPlan::missing`, `hashes_differ` in the diff, and `verify_digests` in the copy
phase all use `get_algo` for it. A row whose `md5` is stored with
an unexpected length is not in the md5 slot, and a slot lookup reports that row
as holding nothing at all.

**The foreign arm is searched by name, not by spelling.** A known algorithm is
found under any ASCII-case spelling of its name — `MD5` as well as `md5` — because
the arm is matched by *comparing* the stored name with that algorithm's canonical
spelling, folding ASCII case only. The comparison is `HashAlg::from_name`'s —
the one the algorithm's own parser uses, so a lookup and the command line cannot
disagree about what a name means — and it allocates nothing: the resolver is a
`const fn`, and a const fn may not allocate, so the property is the compiler's
and not the reader's. A match has no refusal to build and nothing to keep, so
there was nothing for the comparison to keep. `get_named` answers a **known** name
exactly as `get_algo` does, so the two cannot disagree about a row that holds
`MD5` where the request says `md5`. Both go through the same helper, and the
first match wins: the answer is a property of the row rather than of an iteration
order.

**An unknown name is matched exactly.** `get_named("blake3")` finds a foreign
digest stored under that spelling and no other, and no known algorithm's lookup
can return it — it is not an algorithm, so nothing this build knows how to ask for
could mean it.

`get(HashAlg)` is the **slot** read, and it is what `get_algo` asks first. A
caller that means *the digest in this algorithm's slot* may use it; a caller that
means *the digest this row holds under this name* — which is every caller in a
run — must not, because the two have different answers for a row whose known name
carries an unexpected length.

The consequences of getting that wrong are not cosmetic, and they point in opposite
directions. `plan_pairs`'s coverage check would fail a run that is perfectly
answerable, so the user is refused work they could have had. Worse, `hashes_differ`
is *silent* on a missing digest — bounded only because the planner chose those
algorithms — so a slot lookup would report a corrupt row as clean. That is the
false clean, reached from the other side, and the coverage check cannot see it
because the row genuinely does hold a digest under that name.

**Enforced by:** the enforcement tests in `tools/sync/cache/src/hash/tests.rs`
cover a known name in the foreign arm found under any ASCII-case spelling, an
unknown name matched exactly and by no algorithm, a right-length known name still
going to its slot,
a typed lookup finding a known name carried in foreign, and a non-ASCII lookalike
never matching. The resolver is checked against the parser for answering one
question and against the ASCII-case rule written out rather than `std` for how
it folds; its `const` context enforces allocation-freedom from the crate itself,
so even a build that does not build the tests holds it. The arm and the merge
path are checked for replacing entries only under their own spelling, for
keeping distinct spellings of one algorithm across a merge, and for the first
match winning; a known name with the wrong length is checked for routing to
foreign. `tests/why/disagreement.rs` gives `sha256` a wrong-length digest so the
row really does carry a known name in the foreign arm, and asserts the comparison
still finds it.

### A copy is published, and the bound runs under the cache's lock

The copies the tool keeps beside a cache are written under a **working name**
(`girpr-cache-partial-<ts>`) and are **published** under a name of a copy list
(`girpr-cache-backup-<ts>`, `girpr-cache-old-<ts>`) once the copy is complete.

What follows is load-bearing.

**The bound can only ever select a whole copy.** The bound ranges over the
published names, and a published name holds the whole copy or none of it: a
file copy is **linked** into place, so the name is created with the copy's
bytes in it, and a name in a list never carries part of a backup. A copy that
is still being written is not a member of any list, so no bound — however
often it is applied — can take it away. That is the whole reason the copy is
published rather than written in place: writing it straight into
`girpr-cache-backup-<ts>` puts a half-written file under a name the bound
reads, and a run that merely opens the folder can then remove the copy
another run is making there, leaving that run reporting a backup that is not
on disk. It also means the bound's count is a count of whole copies: it never
keeps a half-written one in place of a good one.

The one name in a list that can hold no copy at all is a **claim left
behind**: dropping a claim is best-effort, so a removal that fails leaves an
empty file under a published prefix, and the bound reads that name as a
published copy. It counts towards the bound, and it may be the one the bound
keeps in place of a whole copy.

**The working name is scratch, and it goes.** A file copy is linked into
place, so the published name and the working name are two names for one copy,
and the working one is removed once the copy is published. A removal that
fails leaves the working name behind, and the run warns about it: the copy is
published and whole either way, so the run does not fail over the leftover —
and the leftover is tool state that nothing reclaims.

**The publish is the claim.** A link fails when the name is taken, so the link
that publishes a file copy is also what keeps two runs off one name — there is
no separate claim that a rename could undo. A rename cannot do that job: it
has to be preceded by a claim, and the claim leaves the name empty for as
long as the rename takes, so the name would be a published name with no bytes
in it. A filesystem that cannot link a file has no such publish, and the copy
is renamed onto a name it claims first there — the name is briefly empty, and
the copy lands on it as long as the platform's rename can replace what is at
a name. Where it cannot, the copy never lands there: the publish fails
loudly, the claim is dropped and the name left free, and the run reports the
failure rather than a backup that is not on disk. The drop is best-effort:
its error is ignored, so a removal that fails leaves the claim behind, and
the name is then not free after all. A file copy does not drop its claim
and try again: a rename of a file replaces whatever is at the name, so a
second try could put the copy over another run's published copy — and the
only platform whose rename cannot replace anything at all is the one where
the copy never lands, so there is nowhere a second try would be worth
making.

**A claim that is taken away fails the publish.** A bound cannot tell a claim
from a published copy — both are a name under a list's prefix — and on a
filesystem that cannot lock a file there is no record beside the name to say
the name is being claimed, so the bound can take the claim away. The publish
checks that the name still holds **its own** claim before the copy lands on
it — the same object the run created, and not merely *something* at the name —
and fails loudly when it does not: landing the copy on a name this run no
longer holds would publish it over whatever holds it now, which is the one
thing the claim exists to prevent.

The check is an identity check, and it has to be: the name is briefly empty,
so *something* being at it says only that the name is taken, not that the
claim is this run's. A bound that took the claim away leaves the name free,
and another run can take it and publish there — the claim is exactly what
keeps two runs off one name, so the run whose claim was taken away has to
notice that the name is no longer its own. The identity of a file claim is
read from the **handle** the claim was created with, not from the name: a
handle names the object it was opened on even after the name has gone, so the
answer cannot be another run's file.

**The check is a check-then-act, and the gap is a narrow one.** The
check and the rename that follows it are two calls, and a claim taken away
between them is still landed on. There is no portable rename that is
conditional on the name holding this run's claim, so the gap cannot be
closed. It is reachable only where the filesystem can neither link nor lock
a file: with a link the publish is atomic and needs no claim at all, and
with a lock the record beside the name keeps a bound off the claim until the
run opens the folder's cache.

**The ask is a check-then-act too.** The name is read, and the claim is
removed in the next call, so a claim taken away in between is still removed.
No portable call removes a claim by what it is rather than by what it is
called — the handle a file claim keeps does not name it for a removal — so
this gap cannot be closed either. It is reachable only where the filesystem
cannot lock a file, since a record a live run holds keeps a bound off the name until the
run opens the folder's cache. The same goes for the claim a **file** copy
drops when its rename does not land: the name is asked about again before
the claim goes, so another run's published copy is left alone, and the gap
between the ask and the removal is still there.

**There is no age and no lease.** "Finished" and "in flight" are told apart by
the name, and the name is the only thing the bound reads to make that
distinction. A rule that spared the young copies instead — "only remove a copy
older than an hour" — would be a weaker guarantee wearing the same clothes: the
copy of a slow run would still be taken, and the hour would be a number nobody
could check.

That is also why the **published** name says when the copy became *whole* and
not when the copy started. The bound orders the copies by that name, so a name
that said "started" would make the copy of a slow run look old the moment it
landed, and the bound would take it away from the run that had just made it.
The working name carries the start, because it is scratch: nothing but the
run making it ever reads it.

The order is the order of **age**, read off the name: the stamp it carries,
and — for a name that carries the fallback suffix — the number after it. The
suffix does not sort as a number, so copies published in the same instant could
be ordered wrongly by the name alone, and the bound would take the newest of
them away. The number is therefore read as a number, and the orders the bound
uses — the one it walks the copies in and the one it picks the newest by — are
the same one.

**The family of names has a limit of its own.** A name of that shape that is
taken is stepped over rather than reused, so the published name is one of a
family: the plain `girpr-cache-backup-<ts>` and its numbered spellings. The
family is bounded by a generous limit on purpose, because a name is only ever
stepped over when a run collided at the same instant; it is there so a
pathological folder cannot spin the claim loop forever, not because the limit
is expected to be reached.
A claim that finds no free name within the limit fails the run loudly rather
than spinning: the run reports the failure rather than a backup that is not on
disk.

The bound is applied **after** the cache is opened, and the open holds the
cache's sidecar lock (`girpr-cache-lock`, a file of its own beside the cache)
until the run is done with the cache. The crate takes that lock non-blockingly,
so at most one run is in its run phase for a folder's cache at a time —
**where the filesystem can lock a file at all**. The sidecar is used because
redb locks the database with byte ranges and, on Windows, takes a blocking
lock on a reserved byte when a byte-range lock is refused: a whole-file lock on
the cache itself would block rather than fail fast. A filesystem that cannot
lock a file has neither sidecar nor redb lock, and there the exclusion is gone:
the bound runs under no lock at all, and nothing says that another run is in
its run phase for the folder. The bound
also enumerates the folder once, at its start, so a copy published after that
is not a candidate at all: a run that publishes while another run is bounding
the folder is spared by the enumeration, not by a rule about ages.

A cache that does **not** open is bounded as well, because the bound is about
the folder and not about the open. The bound runs under the cache's own lock
there too — taken for the bound, since the open did not take it — and a lock
another run holds says that run is in its run phase for the folder and bounds
it itself, so that run's copies are not touched. Without this, a corrupt
cache that the open refuses would leave the copies beside it to accumulate
until `--ignore-cache` rebuilt the cache.

**A cache file that cannot be opened for writing is still locked.** The lock
is taken on the cache file, and it is what says that no other run is in its
run phase for the folder — so the file is opened for reading when it cannot
be opened for writing. A run in its run phase holds the cache open for
writing, so a cache nobody can open for writing is a cache no run has open;
but a run that opened it before the file stopped being writable is still
holding it, and the lock is exactly what says so. Only a cache that cannot be
opened at all — not even for reading — has no lock to take, and the bound
runs without one there.

**A filesystem that cannot lock a file has no lock to take either.** The lock
cannot be taken there, so the bound runs without one there as well, and
nothing says that another run is in its run phase for the folder.

The bound also never removes the copy the run applying it published: that run
is the one reporting the copy. The count is made up from the next copy in age
order instead, so the folder ends within its bound — but only while there are
enough copies the bound may remove to make the count up. A copy that is spoken
for does not spend the removal budget either: the bound walks on to the next
copy in age order, and the count is made up from there — and that copy may be
one of the newest ones, so the bound can take a copy that is among the newest
it keeps and leave an older copy in its place. The copies a folder keeps are
therefore not necessarily its newest, and a folder whose oldest copies are
spoken for ends with more copies than the bound asks for as long as there are
not enough copies the bound may remove to make the count up. The copy is
spared by every bound **that run** makes for that list, and not only by the
first one; the copies an earlier run in this process published belong to
earlier runs, and those are ordinary copies again.

**The run is the boundary, and the boundary is the open.** The copies a run
published are the ones published since the folder's cache was last opened:
the run that opens the cache is the run that published them, and the run
before it published the ones before that. A run that publishes nothing into a
list has nothing of its own to spare there — so the copy the run before it
published is an ordinary copy again, and sparing it would keep an older copy
in place of a newer one. The registry that says this keeps, per folder, the
copies waiting for the next open and the copies the last open adopted. It is
process-wide because the copies are made in more than one
place, and it is scoped to a run because the guarantee is about a run's copy
and not about a process's.

**A copy published by a run that never opens the folder's cache is adopted by
the next run that opens it.** The boundary is the open, and a run that
publishes a copy without opening the folder's cache leaves that copy waiting:
the next run to open the folder adopts it and spares it, so an older copy is
kept in place of a newer one for one bound cycle — after which it is an
ordinary copy again, because the next open clears the list. The registry lives
in the process, and the command line runs one command per process, so a copy
left waiting is forgotten with the process that published it and is an ordinary
copy again for the next run. The adoption is reachable only when two runs
share a process.

**A run whose open failed does not adopt them.** The boundary is the open,
and an open that failed did not cross it: the copies keep waiting, and the
next run that opens the folder adopts them. A run that adopted them would
take them off the waiting list, and the run that opens the folder next would
spare nothing — the copy that was waiting would be an ordinary copy again.
The bound that run makes — a cache that does not open is bounded as well —
spares them all the same. The waiting list holds the copies this process
published since the folder's cache was last opened, and this run cannot tell
its own from an earlier run's in the same process, so it spares them all:
the ones that are its own are the ones it is about to report, and the ones
that are not are spared at the price of the folder holding more copies than
its bound for a cycle.

**A copy is published before the lock that would protect it is taken.** That
gap is the one place where a live run's copy can be lost: the run that
published it has not opened the folder's cache yet, so another run can be in
its run phase for the folder, and its bound can take the copy away from the
run that is about to report it. The bound cannot tell the two apart by name —
both are published names — so a published copy carries a **record** of its
own: `girpr-cache-hold-backup-<ts>` beside `girpr-cache-backup-<ts>`, and
`girpr-cache-hold-old-<ts>` beside `girpr-cache-old-<ts>`. The publishing run
holds an exclusive OS lock on that record, and the record is taken **before**
the copy's name is claimed, so a published name never appears without one —
except on a filesystem that cannot lock a file, which has no record to take.

The record is a lock and nothing else: it is an empty file, and the lock on it
is the whole of what it says. There is no age and no lease in it: the
OS releases the lock when the process ends, and it releases it when the
process is killed, so a record left behind by a dead run is **stale** rather
than a claim that never expires. A bound that meets a record a live run holds
leaves the copy alone; a bound that meets a stale record takes the copy like
any other and reclaims the record with it. Once the run holds the cache's own
exclusive lock, that lock is the protection — no other run is in its run phase
for the folder — and the record is released, because it has nothing left to
say.

A record that cannot be **read**, or whose lock cannot be **tested**, says
nothing either, and says it without sparing the copy: nobody can be holding a
lock on a file the filesystem cannot lock, and a record that cannot be read
cannot be the record of a live run. Such a record is left where it is — a
bound that cannot judge a record cannot claim it either — but the copy it
spoke for is an ordinary copy again. Otherwise a record left behind by an
older build would defeat the bound for that copy for good, on a filesystem
that cannot lock a file.

The record is **per copy**, not per run, so two runs on one folder never
share one: a run that finds the record for a name already held is looking at a
name another run is publishing under, and it moves on to the next name rather
than racing for it. Taking the record before the claim is also what keeps two
runs from publishing onto one name.

The tool takes **locks of its own**: the per-copy record's and the
cache's sidecar. The bound itself still runs under the cache's sidecar, which
the open holds: a folder whose filesystem cannot lock a file still gets the
bound, and its copies are
published and reclaimed as they were before a record existed — the record's
lock is per copy, and a filesystem that cannot take one leaves the copy
unmarked rather than failing the run. A publish that fails fails the run
loudly, rather than leaving a run reporting a backup that is not there.

**Enforced by:** the enforcement tests cover the publish — a copy still being
written is out of the bound's reach, a published name holds the whole copy, a
failed removal leaves the working name behind, and the copy is published whole
under a claimed name, including on a filesystem that cannot link. They cover a claim that went away or was taken over by another
run failing the publish loudly, a publish that fails when its rename does not
land, and a publish not dropping another run's copy. They pin that the bound
orders copies by age and not by name (the name says when a copy became whole),
keeps the copy the run published, keeps sparing it on a later bound in the same
run, treats an earlier run's copy as ordinary again (also when that run published
nothing), and adopts a copy left waiting for an open; a run whose open failed
neither adopts nor loses those copies. They cover bounding a folder seen through
a whole run, and bounding a folder whose cache is absent, corrupt or held by
another run, as well as a cache not openable for writing still being locked and
the protection the cache's own lock gives once a record is released. Finally
they cover the per-copy record: published copies carry one, a live run's copy is
spared by the bound, a spared copy does not spend the removal budget, a record
whose run is gone or whose lock cannot be tested does not spare its copy, a
record of a gone copy is reclaimed, a record is not one of the copies the bound
counts, a record carries nothing but its lock, concurrent publishes get copies
and records of their own, and the record is released once the cache is open.

---

## Not on the table

Each of these looks like an obvious tidy-up — a duplication, or a type that could be
the tidier shape. None of them is one.

**Do not merge `scan_stat_only` and `load_record_side_from`.** They are not two
variants of one walk: one reads a directory, the other reads a table. Merging
them would make the record path look swappable with the disk path, which is
exactly the mistake the record path exists to prevent — and `compare-self` is what
that separation buys. `effective/record.rs`'s own module doc says so.

**Do not turn `Side` / `RwSide` into a trait.** Each is a small enum with its
dispatch concentrated at one site (`effective/side.rs` for `Side`, the rw gate
for `RwSide`). A trait here would be ceremony with no second implementor, and
the enum's shape is what keeps each dispatch site readable at a glance.

**Do not touch the `--rw-threads*` family, `RwLimiter`, or the lock order.** See
the lock-ordering section above. The tests are good
(`tools/sync/leaves/src/rw/gate.rs`'s unit tests plus
`tests/rw_threads/limiter.rs`); leave them alone.

**Do not convert `FileRec.kind` to an enum.** It is the obvious companion to
`HashAlg`, and it is the wrong direction: `kind` is a closed registry *on the
wire* by design, so that a new kind breaks old binaries loudly instead of
quietly. See the algorithm-vocabulary invariant above for why algorithms are not
the same case.

**Do not inline `same_spelling`.** It is a one-line wrapper over an equality
comparison, and it stays because it is the one place the foreign arm's
exact-match rule is written: the lookup that finds a digest under a name no
algorithm answers to, and the two writers that put one there, all come through
it. (A known algorithm is found under any ASCII-case spelling of its name, and
that is the other rule — the one the algorithm's own resolver writes.) Inlined,
the rule would be spelled at every call site, and a second spelling of a
rule is a rule that can drift — and this one drifts quietly, because a row
that grew a second entry under one name still answers its lookups. If the
arm's exact-match rule ever stops being exact, this is where it changes.

**Do not add `bitflags` (or any dependency) for `AlgoSet`.** `bitflags` is already
in `Cargo.lock` transitively, so it would cost nothing to compile — which is
exactly why it is worth refusing. `AlgoSet` is a newtype whose `Debug`
has to be hand-written anyway to keep printing `["md5", "sha256", "xxhash64"]`,
and this crate hand-rolls `RwLimits`, `StatTrust` and `CasePairing` in preference
to derives.
Consistency wins, and a bitset crate is not a rule the crate can enforce for us.

**Do not weaken `plan_pairs`' coverage check or `pair_verdict`'s `unverifiable`
clause.** Together they close the last path by which a run can report a confident
answer about content nobody read. `hashes_differ` skips an algorithm either side
lacks, so without them an uncovered pair falls back to size+mtime — which had
already agreed — and the run reports `CHANGED = 0` about a file it never opened.
That is a **false clean**, the worst outcome this tool can produce, and the
failure mode of a regression here is invisible: the counters look fine, the rows
look fine, only the verdict is wrong.

**Do not fold the rename failure hooks into one.** `GIRSYNC_TEST_FAIL_RENAME`,
`GIRSYNC_TEST_FAIL_RENAME_STEP1` and `GIRSYNC_TEST_FAIL_RENAME_STEP2` are separate
switches because the case-fixing rename has distinct failure points, and each leaves
something different behind — nothing, a file placeholder, an entry to roll back.
One switch with a stage value would put that stage in the *value*, and the value
is a path everywhere else in the family, so the family would carry different kinds of
value to parse and one rule to state twice. The stage in the switch's **name**
keeps every failure hook's value a plain relative path.

It also keeps them separately armable. Each stage reads its own variable
and holds its own path, so a test can name a stage and a path per variable; a
single variable would hold one stage and one path at a time. The compound value
the copy gate already uses (`<wait>:<on>`) shows what the alternative costs: a
stage vocabulary of its own, a split, and a malformed value that asks for
nothing, all of it beside the rule the paths already have.

---

## Known limits, deliberately not fixed

They are several kinds of thing: state in the cache that the next run reclaims, a
race the run cannot win, state in the cache or the folder that nothing reclaims
on its own, a limit on what the host can create, and an approximation of the
filesystem's own case table. Every one of them is stated in `README.md` where a
reader meets it — the host-coverage one in the suite's own section, since it is
about the test suite rather than about a run. Recorded here so a reader deciding
whether to "fix" one knows what the trade was.

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
"What a copy costs" and in `copy_one`'s own doc.

**A stat-only row can get stuck.** A row whose stat changed arrives in phase A with
its digests dropped, and while a stat difference settles the pair for free — a size
difference, or an mtime one — no comparison rehashes it, so the row stays
stat-only. Repairs: the flag that distrusts the field doing the settling —
`--no-trust-size` for a size difference, `--no-trust-mtime` for an mtime one, and
either acts only when the distrusted field is the *only* difference — and
`girsync update --dir <the folder>`, which rehashes every file and writes the
digest back into the row. The row looks healthy — right size, right mtime — which
is what makes it worth stating.

**A converter's temporary left by a killed run stays in the folder.** A
converter writes the new cache into a temporary beside the destination and
moves it onto the destination only once it is whole, so a killed run leaves
that temporary behind — `girpr-cache-convert-<pid>-<ts>.redb`, or the numbered
spellings of that name when the plain one is taken. That family of names is
bounded as well: the plain name and its numbered variants up to a generous
limit, and a converter that finds every one of them taken fails rather than
trying forever. Nothing reclaims any of them: the retention bound
ranges over a cache's published copies and not over these names, so a leftover
stays in the folder until it is removed by hand.
That is the same trade as the working copy a killed backup leaves, and for the
same reason: a temporary is the file a live run is writing, and a sweep that
could not tell a live one from a dead one would take the copy a running
converter is making. Stated in `README.md`, where a user meets it.

**Host coverage of the link cases.** Some end-to-end cases need primitives a
regular user cannot always create: Windows without developer mode refuses
`symlink_file`/`symlink_dir`, while a directory junction (`mklink /J`) and a
hard link are both created without elevation. `tests/symlink.rs` and
`tests/sync.rs` self-skip the cases whose primitive the host refused, printing a
`skip:` line that says why, so a locked-down host runs the junction- and
unit-level halves while the file-symlink bodies do not. A green local run is
therefore not full link coverage — that gap is closed on a host that can create
the links, not by weakening or removing the cases. The skip reason sits beside
each case; there is no separate document to keep in step with them.

**Neither fold is the filesystem's own case table.** Windows treats `Ärger`
and `ärger` as one file and the Kelvin sign and `k` as two, so the ASCII-only
key keeps apart a pair the disk has merged and the Unicode identity rule
merges a pair the disk keeps apart; there is no portable exact NTFS fold, so
the approximation is kept rather than guessed at.

---

## Things that are one

**"What must be computed" and "can the pair be answered" are different questions,
and only one of them honours `no_trust`.** A side's availability is `cached ∪
hashable` with deliberately no trust term: `--no-trust-cached-hashes` asks a side
to re-read rather than reuse, and a record has no filesystem to re-read, so
folding trust into coverage would make the flag fatal for every record holding
exactly the right digests. `planner/mod.rs` carries the table; do not merge the
two.

**The diff and the planner must agree about the short circuit.** Both read
`StatTrust::settles`, and the planner's decision travels out in `PairPlan::stat`
rather than being re-derived from the flags. If they disagree, a *silent*
failure results: the planner hashes a pair whose digest nobody reads, or — worse —
a pair the planner called settled turns out to have needed a digest nobody
computed. There is deliberately no `untrusted_disagrees() -> bool` next to
`untrusted_differs`: a second spelling of a rule is a rule that can drift, and the
caller that wants a bool asks `.any()`.

**The request and the answer must stay different shapes.** `CommonOpts.algos` is an
ordered `Vec<HashAlg>`; every per-path and per-pair answer is an unordered
`AlgoSet`. They look like the same thing at different sizes, and flattening either
one is a real loss:

- The **request's order is a contract with the user.** `pick_one` breaks an
  `--hash-any-of` tie in the order the flags were typed, and
  `planner::any_of_breaks_a_tie_by_the_flag_order` pins it. Make the request a set
  and the tie-break silently becomes alphabetical, so
  `--hash-any-of sha256 --hash-any-of md5` stops meaning what it says — with no
  test failing, because every fixture that names two algorithms happens to name
  them in canonical order.
- The **answers have to be a set** because there is one per path, for every path in
  the tree, and under `--hash-all-of` every one of them is the same value. A list
  there is a heap allocation per path holding a fresh clone of the request; a set
  is a compact value inside a map value that already exists.

Neither shape is derivable from the other, and the reason each is right is
opposite to the other's.

**The request is spelled one algorithm per occurrence.** `--hash-any-of` and
`--hash-all-of` are repeatable, and each occurrence names one algorithm, so a
request for more than one is written `--hash-any-of sha256 --hash-any-of md5`. A flag that
named two values in one occurrence is refused by the parser before the run
starts, so every place that spells these flags — the CLI's own help, the README,
and the remedy a coverage error prints — repeats the flag. The remedy matters
most: it is printed for the user to paste, and in the other shape it is a dead
end rather than a way out.

**The order is a contract where a choice is made, and the copy path makes none.**
`copy_one` takes the run's algorithms as an `AlgoSet` and judges exactly that
set: the digests it compares are the ones it computed, so a digest the copy did
not produce could not be compared at all, and a request beside the set would be
a second spelling of the same thing waiting to drift. Nothing on that path
depends on the order the user typed, so a disagreement is named in canonical
order — as the diff's `digest-differs` tag already names it.

**The commit thresholds and the durability are one policy.** A cache write
resolves both when it starts — a batched write handle when it is opened, the
open's own one-off `meta` write when it runs — and keeps them for as long as
the write lasts. They are test-only switches and they look like independent
knobs; they are not, because a write that committed some of its
batches one way and the rest another would leave a run's own cache half
durable and half not, which is the opposite of what the durability switch is
for. The thresholds exist so that a small fixture can make a commit
observable, and the durability exists so that a whole suite's commits are
cheap; neither is for changing a policy half way through a write. So one
function reads both, at one moment, and a handle's later transactions take the
policy it opened with rather than asking the environment again.
