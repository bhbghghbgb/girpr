# girsync — dev-only one-way folder mirror with hash cache

**Audience: developers and AI agents working on `girpr`. Not for end users.**
**Direction is always `src` (old-version reference) → `dst` (working dir). Never the reverse.**

## What it is

Workspace member (`tools/sync`, binary `girsync`) for testing version upgrades/downgrades:
snapshot a game folder's state, diff two folders/records, and restore `dst` to `src` content.
Separate from the `girpr` binary; shares no flags, no exit-code contract, no code.

## Source layout

`src/lib.rs` holds the whole implementation; `src/main.rs` is just
parse -> init tracing -> `run` -> exit. Tests are integration tests in `tests/`
and drive the public API.

| File | Responsibility |
| --- | --- |
| `cli.rs` | clap surface: `Cli`, `Cmd`, and `CommonArgs` (flattened into all four subcommands) |
| `config.rs` | validated per-run options: `CommonOpts`, `ScanMode`, `LogCtx`, `Update/Compare/SyncOpts` |
| `report.rs` | the stdout data plane: `Record` + `Report`, `--output text\|json`; `verdict` (every record a diff reports) |
| `commands/mod.rs` | `run` dispatch; converts raw CLI strings into `CommonOpts` (where they are validated); the diff reporter/exit code shared by `compare` and `compare-self` |
| `commands/update.rs` | refresh one folder's cache |
| `commands/compare.rs` | diff two sides, print, exit 4 on any difference |
| `commands/compare_self.rs` | diff a folder against its own cache, folder side scanned with no cache |
| `commands/sync/mod.rs` | the mirror run: validate, back up, load, then rename -> plan -> apply |
| `commands/sync/rename.rs` | case-fixing rename pass, so the diff can be case-sensitive |
| `commands/sync/plan.rs` | `Plan` + `build_plan` (pure) and the `--dry-run` printer; `open_folder_cache` (in `effective.rs`) is what makes `--dry-run` mean the same thing in every command |
| `commands/sync/apply.rs` | `Applier`: the ordered apply phases, plus `copy_one` |
| `cache.rs` | redb schema (`Meta`, `FileRec`, binary codec), `open_db` (`CacheOpen` read/write vs read-only), backup/snapshot helpers |
| `scan.rs` | `walk_live` (the on-disk walk) and `check_mixed_case` |
| `effective.rs` | the two phases for one side: `open_side`/`scan_stat_only` (stat + cache, never hashes) and `resolve_side`/`resolve_folder`/`resolve_record` (produce what was planned); `merge_row`, the cache write rule |
| `planner.rs` | `HashPlan::plan_one_side` (one side, `update`) and `plan_pairs` (two sides, `HashMode` and the coverage check): the only place that decides which digests a run must compute. `Required` carries the per-path answer to the diff |
| `diff.rs` | `Diff` buckets, `diff_maps`, and `Verdict` + `pair_verdict` — the per-pair classification and its reason tag |
| `filter.rs`, `hash.rs`, `util.rs`, `logging.rs` | glob filters, digests, path/time/FS helpers, tracing setup |

Adding a flag: declare it in `cli.rs` (or `CommonArgs` if shared), read it from
`CommonArgs` in `config.rs`'s `TryFrom`, then use `common.<field>` in the command.

**Two channels, and nothing is printed raw.** **stdout** is the answer —
differences, plan actions, summaries — and it is made of `report::Record`s
rendered by `--output` (see "Output" below). **stderr** is the narration:
`tracing` events with named fields, filtered by `--log-level` and optionally
mirrored to a JSON `--log-file`. `logging.rs` owns stderr and `report.rs` owns
stdout; neither writes to the other's, which is what lets `--output json` hand a
caller a clean stream of JSON records with no log chatter mixed in. The only
`println!` in the crate is `Report::emit`, and the only raw stderr writes are the
two `FATAL` lines that happen *before* a subscriber exists.

`build_plan` reads only the two effective maps, which is what makes `--dry-run`
exact. The apply phases are ordered inside `Applier::apply` because the order is
a correctness invariant: cache entries for every path a run will touch are
dropped *before* the filesystem change, so a crash re-copies rather than
trusting a half-written file.

## Commands

```
girsync update    --dir <DIR>                        [--hash-all-of md5] [--hash-any-of md5] [--include G --exclude G] [--case-sensitive] [--max-depth 10] [--ignore-cache] [--dry-run]
girsync compare   --src <DIR|RECORD> --dst <DIR|RECORD>  [--hash-all-of md5] [--hash-any-of md5] [--no-trust-size] [--no-trust-mtime] [--why] [--show-identical] [--no-trust-cached-hashes src|dst] [--dry-run] [...]
girsync compare-self --dir <DIR>                    [--hash-all-of md5] [--hash-any-of md5] [--no-trust-size] [--no-trust-mtime] [--why] [--show-identical] [--no-trust-cached-hashes] [--dry-run] [...]   # --no-trust-cached-hashes and --dry-run are accepted and do nothing
girsync sync      --src <DIR> --dst <DIR>           [--missing-only] [--keep-extra] [--jobs 4] [--hash-all-of md5] [--hash-any-of md5] [--no-trust-size] [--no-trust-mtime] [--no-trust-cached-hashes src|dst] [--dry-run] [...]
```

Global flags: `--log-level trace|debug|info|warn|error`, `--log-file <PATH>`,
`--output text|json`. `sled2redb` takes `--force` and the same `--output`.

### `--no-trust-size` / `--no-trust-mtime`

The stat short circuit is an optimisation, so it is switchable. These name the two
fields it consults rather than adding a switch for the optimisation itself — **both
together is "disable it"**, expressed as the reason:

| | a pair is settled for free when |
| --- | --- |
| default | sizes differ, **or** mtimes differ |
| `--no-trust-size` | mtimes differ (size alone no longer settles it) |
| `--no-trust-mtime` | sizes differ (mtime alone no longer settles it) |
| both | never — every file-on-both-sides pair needs a digest |

Only *file on both sides* pairs are affected. Dirs are compared by presence, a kind
conflict by kind, and a one-sided path is `MISSING`/`EXTRA` — none of which any flag
here touches.

The two flags are **not** interchangeable, and the difference is worth knowing:

- **`--no-trust-mtime` can change a verdict.** A file whose mtime moved but whose
  bytes did not — a `touch`, an extractor rewriting identical bytes — is currently
  `CHANGED`. Distrusting mtime is how you ask whether that is really so, and it may
  come back equal.
- **`--no-trust-size` cannot.** Different lengths are different content, so the digest
  comparison will always disagree and the verdict is unchanged. What the flag buys is
  the *read* — and that repairs the cache: a size-changed row arrives in phase A with
  its digests dropped (a stale row's are all suspect), so under the short circuit it
  stays stat-only forever and can never become comparable again. This is what puts the
  digest back.

Neither is `--no-trust-cached-hashes`, which distrusts a **digest** rather than a
**stat field**: it only ever affects pairs that were going to be read anyway, and it
re-reads them even when the cache holds a matching one.

#### They need something to fall back on

Both flags are a request to decide a pair *some other way*, and the "some other way" is
a digest. So neither can be combined with a run that requests none:

| | |
| --- | --- |
| `--hash-all-of none` | **refused** — there is nothing for the difference to fall back on |
| `--hash-all-of md5 --no-trust-size` | fine — the digest decides |

Refused rather than warned, and that is a deliberate break from how `update` treats the
other flag it cannot honour. The distinction is whether the verdict stays right: ignoring
a flag that does not apply still produces a correct answer, whereas ignoring *this* one
produced a wrong one, so a warning would have been describing a choice the user
believes protects them. Concretely, `--hash-all-of none --no-trust-size` used to report
a size-differing pair as **equal**, and `sync` under it left `dst` stale while
reporting success.

Two guards, on purpose. The flag layer refuses the combination, so the user gets a
message instead of a run — and it refuses it in one place, so every subcommand including
`update` behaves the same way, since the combination is a property of the *request* and
not of the command receiving it. Behind it, `diff::pair_verdict` refuses to report a pair
equal when it consulted **no digest at all** and there was a difference it was told to
ignore; that covers what the flag layer cannot reach, which is a caller assembling
`CommonOpts` directly. While a digest *is* available it stays a real verdict, so a
touched file whose bytes agree is still cleared.

`update` accepts both otherwise and ignores them, with a warning — it rehashes every
file regardless, so there is no short circuit for them to switch off, and obeying them
by doing *less* would be wrong.

### `--hash-all-of` / `--hash-any-of`

Both are repeatable (`md5`, `sha256`; default `md5`) and mutually exclusive. They
answer one question — *how many of the named algorithms must a comparison be able
to use?* — and the difference is what happens when a side cannot supply them:

| | requirement on an undecided pair | a folder side short of one | a **record** side short of one |
| --- | --- | --- | --- |
| `--hash-all-of` (default) | **every** algorithm named | backfills it by hashing | **exit 3**, naming the remedy |
| `--hash-any-of` | **one**, chosen per pair | hashes the chosen one | **exit 3**, if *none* qualify |

A pair is undecided when it is a file on both sides with equal size+mtime — the
only state where a digest is consulted at all. Anything else is already decided by
stat or presence, so no algorithm is required and no coverage is checked.

`--hash-any-of` picks cheapest-first — an algorithm both sides already hold costs
no read, one costs a read, two cost two — and within a tier it takes the **order you
wrote the flags in**, so `--hash-any-of sha256 md5` prefers sha256. The
choice is **per pair**: one run can settle path X by md5 and path Y by sha256.

That makes it strictly weaker than all-of, and never weaker than the pick: every
algorithm it chooses is obtainable on *both* sides, so a pair settled this way is
settled by a digest both sides hold. That is the whole difference from the old
behaviour, which skipped whatever digest was missing and reported the pair equal on
size+mtime alone.

Reach for it when a side holds only some of what you would otherwise demand — a
record written by an older run, typically. An `--hash-all-of md5` history compared with
`--hash-any-of md5 sha256` succeeds wherever the record has md5, reading one digest
instead of refusing and without repopulating the tree. On two *folders* it is mostly
free money: both can always backfill, so the saving is whatever the caches already
cover.

`--hash-all-of none` is the stat-only audit — no hashing at all, decisions by
size+mtime only (verify-after-copy also size+mtime). `none` is **not** accepted by
`--hash-any-of`: with nothing requested there is no "any of" to choose.

> **Breaking change.** `--hash` was renamed. A record missing a requested digest used
> to degrade silently to size+mtime and report the pair equal; under the default
> `--hash-all-of` it is an error. A script passing `--hash md5` must become
> `--hash-all-of md5` — the old spelling is rejected rather than aliased, so a typo
> that used to work now says so.

`--no-trust-cached-hashes` is repeatable and takes a side: pass `src`, `dst`, or
both. On a named side, a cached digest is never reused — every file is rehashed
even when size+mtime match the cache. Off by default, and it only concerns
*digest* reuse, not the stat data (which is always re-read from disk). Combining
it with `--hash-all-of none` is harmless but pointless: there are no digests to
distrust. `update` has no such flag — it is defined as a full repopulate, so it
never trusts cached digests in the first place.

### update

Refreshes `<DIR>/girpr-cache` to current disk state: stats every file, records
empty dirs (presence-only), prunes rows for deleted/excluded paths. Backs up
the old DB first (see below). Reports one record: the directory, the file and
dir counts, and the algorithms it computed.

It recomputes every algorithm named by `--hash-all-of` on every run, but that
does **not** mean every stored digest is replaced:

- **stat unchanged** — algorithms this run did not ask for are kept, so
  `--hash-all-of none` records stat data without touching stored digests at all,
  and `--hash-all-of md5` leaves an existing `sha256` alone. To refresh an
  algorithm, ask for it by name.
- **stat changed** — *all* stored digests are dropped, including algorithms
  this run did not request, since the content is assumed to have changed with
  them. Only the requested set is written back.

So the cache is a superset of what you last asked for, pruned to the current
stat. `girpr-cache-backup-*` (written before the run) is the manual way back
from a bad prune; it is a recovery aid, not a correctness mechanism.

It runs the same three phases as every other command — stat-only scan, plan,
resolve — via `HashPlan::plan_one_side`, which is the one-sided predicate: there
is no other side to be lazy *relative to*, so every file is a candidate and every
requested algorithm is asked for. That is also why `update` needs no exemption from
the coverage check that stops a `compare` against an incomplete record: it asks a
folder for digests of files the folder has, which a folder can always compute.
The `hashed` counter in the run's log line is the answer to "how much would this
read?", which is the question `--dry-run` exists to ask.

### compare

Each side is classified by basename: path whose final component starts with
`girpr-cache` is a **record** (DB used as-is, no FS access, no writes);
otherwise it is a **folder** (embedded `<root>/girpr-cache` loaded/created, live
stat + lazy hash, **cache created and updated as a side effect**). All 4 combos
work, subject to the one-cache-per-run rule below. What gets read is decided per
pair, not per side: a file on both sides with equal size and mtime is the only
thing that needs a digest (see "Cache is a cache, not truth"). Output classes, one
record each: `MISSING` (src-only), `EXTRA` (dst-only), `CHANGED`
(size/mtime/hash differ), `TYPE-CONFLICT` (file vs dir), `CASE-MISMATCH a <=> A`
(insensitive mode only), then a `SUMMARY` — text lines by default, JSON objects
under `--output json` (see "Output"). Exit `4` if any diff, `0` if
equal. `sync` accepts folders only.

`compare` never touches the two trees, but it *does* write the caches — so it
takes `--dry-run` like every other subcommand, and without it there is no way to
audit two folders and leave both caches exactly as they were. Under `--dry-run`
the caches come out byte-identical (an existing one is opened read-only, a
missing one is served from memory so none is left behind), and the report is
identical.

**A record side must be able to answer every pair it is asked about, and exits `3`
if it cannot.** Coverage is checked *before* either side reads a byte, over the
undecided set only. A folder can always go and compute a missing digest; a record
cannot, so a digest it does not hold is a question this run cannot answer — and
the alternative, which is what used to happen, was to fall back to size+mtime and
report a confident answer about content nobody compared. `--no-trust-cached-hashes`
on a record side is *not* a shortfall: it asks the record to re-read, and a record
has nothing to re-read. `sync` runs the same check and cannot trip it, because
both its sides are folders. Details, and the message shape, in `src/planner.rs`.

### compare-self

`compare` with one argument: the record side is `--dir`'s own `girpr-cache`, so
this reports what the cache has drifted from without repairing it in the
process. That repair is the reason it is not simply
`compare --src <DIR>/girpr-cache --dst <DIR>` — that spelling is rejected by the
one-cache-per-run rule, because a folder side rewrites its cache while scanning,
so the audit would fix the drift it was reporting and a rerun would come back
clean.

Output and exit codes are `compare`'s, with the **record as `src`**:
`MISSING` = a row the cache still holds for a path that is gone, `EXTRA` = a
path on disk the cache never recorded, `CHANGED` = stat or digest disagreement.
Exits `4` on drift, `0` when in step, `3` if there is no cache to compare
against (it never creates one — that would make the first run vacuously clean)
or if `--ignore-cache` was passed (it would delete the record under audit).

**The folder side is scanned with no cache to consult**, which is what makes this
an audit. It used to share the record's cache, and a folder side's only
cache-derived input is digests — so for every file whose size+mtime still
matched, the "content comparison" was the record's digest compared with itself.
Content that changed while preserving both was invisible, and
`--no-trust-cached-hashes` was the only thing that could surface it. Now the
folder is hashed against nothing, so it must be read to be compared, and a
`CHANGED` means the bytes disagree. Two independent reasons nothing is written:
the record is opened read-only (the handle has no write path at all), and the
folder side is resolved against an in-memory handle that is not the same file.

**It reads the undecided pairs, so the cost runs opposite to what you'd guess.**
A file whose size or mtime disagrees with the record is settled without a read,
which makes an audit of a drifted tree cheap. A folder that is *in step* has
every pair stat-equal, so every file is read — there is nothing else that could
settle them. `--hash-all-of none` is the stat-only audit and costs nothing, if that is
what you want. `--no-trust-cached-hashes` and `--dry-run` are both accepted and
do nothing here; the command warns rather than erroring, so a script passing them
to every subcommand keeps working.

**A record that cannot answer a pair it is asked about is now an error, not a
pass.** Exits `3`, naming the record, how many undecided paths are uncovered, a
few of them, the per-algorithm coverage, and the remedy. This is a behaviour
change and the commonest way to hit it is worth knowing:

```
girsync update src      # cache warm
# a file appears in src
girsync sync src dst    # ...and leaves a stat-only row for it in src's cache
girsync compare-self src   # exit 3: 1 of 3 undecided paths uncovered
```

The `sync` is not at fault. It decided that pair by presence, so it needed no
digest, so phase A recorded the file's stat and nothing else — and that row is
*fresh*, so nothing will ever fill it in on its own. The audit then finds a
stat-equal pair the record cannot digest, and before this rule it exited **0**:
`hashes_differ` skips an algorithm either side lacks, so the pair quietly fell back
to the size+mtime that had already agreed and the audit reported the newcomer as
in step. Three ways out, all named in the error: one command
(`girsync update --dir src`), a narrower request
(`--hash-all-of md5` if the record holds only md5), or
`--hash-all-of none` for a stat-only audit.

**`--hash-any-of` is the fourth, and the one that needs no new work on the tree.**
Where the record holds *some* of what you asked for, it reads one algorithm instead
of refusing:

```
girsync compare-self --dir src --hash-any-of md5 sha256
```

An audit of a `--hash md5` history then succeeds wherever the record has md5, without
repopulating. It reads one digest per undecided pair and cannot read zero — this
command's disk side has no cache by design, so there is no tier-1 "already have it"
to hit — which makes `any-of` here a way to *narrow the request to one algorithm*,
not a way to skip the work.

The same caveat as everywhere else applies to `--max-depth` and
`--include`/`--exclude`: a run whose filters differ from the ones the cache was
built with reports drift that is not there.

### `--output text|json`

Every subcommand reports through one writer (`report::Report`). `--output text`
(the default) prints a human line per record; `--output json` prints the *same
records* as JSON. The two are renderings of one value, not two implementations,
so they cannot drift.

JSON is **newline-delimited**: one object per line, no enclosing array, so it
streams and the record order is the report order. Every object has an `event`
key; the rest of the keys are the record's fields. Within an object, keys are
names — read them by name, never by position.

| event | emitted by | fields |
| --- | --- | --- |
| `missing` / `extra` / `changed` / `type-conflict` | compare, compare-self | `path` (plus `why` on `changed`, with `--why`) |
| `identical` | compare, compare-self | `path` — only with `--show-identical` |
| `case-mismatch` | compare, compare-self | `src`, `dst` |
| `summary` (diff) | compare, compare-self | `missing`, `extra`, `changed`, `type_conflict`, `case_mismatch`, `total_diff` (plus `identical` with `--show-identical`) |
| `update` | update | `dir`, `files`, `dirs`, `algos` |
| `rename` | sync | `from`, `to` |
| `mkdir` / `fix-dir` / `delete` / `rmdir` | sync | `path` |
| `copy` | sync | `path`, `because` (`missing` \| `changed` \| `type-conflict`) |
| `summary` (sync) | sync | `renamed`, `mkdir`, `copied`, `deleted`, `rmdir`, `missing_only`, `keep_extra`, `dry_run` |

Exactly one `summary` is emitted per run, and it is last. The event name is the
text label lowercased — `FIX-DIR` prints as `FIX-DIR` and is `"fix-dir"` — so
there is no parallel vocabulary to keep in step.

```powershell
# text
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live
CHANGED a.txt
SUMMARY missing=0 extra=1 changed=1 type_conflict=0 case_mismatch=0 total_diff=2

# json, same run
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live --output json
{"event":"changed","path":"a.txt"}
{"changed":1,"event":"summary","extra":1,"missing":0,"total_diff":2,"type_conflict":0,"case_mismatch":0}
```

### `--show-identical`

The complement of the diff report: report the pairs that came out **equal**, one
`IDENTICAL` record each, with its own `why=` tag.

```
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live --show-identical --why
MISSING a.txt
IDENTICAL b.txt why=digest-matches:md5
IDENTICAL c_dir why=dir-present
SUMMARY missing=1 extra=0 changed=0 type_conflict=0 case_mismatch=0 identical=2 total_diff=1
```

A `CHANGED` list cannot answer *"is this file in step, or did the tool simply not
look at it"* — that is the question this flag is for. It reports what the diff
already concluded and changes none of it:

- **A pair that came out equal is not a difference.** `total_diff` and the exit
  code are unaffected, so a clean tree still exits `0` with the flag on. This is
  the property to check first if you touch `Diff::total()`: counting the equal set
  would turn "these trees are identical" into exit `4`, the exact inverse of the
  truth.
- **`SUMMARY` gains `identical=N` only with the flag**, so an unflagged run is
  byte-identical to one from before the flag existed. A consumer must treat the
  field as optional.
- **Directories are included** (`IDENTICAL ... why=dir-present`). Dirs compare by
  presence, so "in step" is a real answer for them; leaving them out would make
  the flag's output look arbitrarily partial.

**Default off**, and for a real reason rather than tidiness: this is one record per
file on both sides, so on a matching tree it is the *entire* file list. It is
independent of `--why` — either alone is legal, together is legal.

### `--why`

`CHANGED` says *that* the two sides disagree; `--why` says **which check decided
it**. The tag is appended to the `CHANGED` record and to nothing else:

```
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live --why
CHANGED a.txt why=stat-size
CHANGED b.txt why=digest-differs:md5
SUMMARY missing=0 extra=0 changed=2 type_conflict=0 case_mismatch=0 total_diff=2
```

| tag | meaning |
| --- | --- |
| `stat-size` / `stat-mtime` | a stat field **this run trusts** differs, so no digest was read |
| `stat-size+stat-mtime` | both differ — still one stat decision |
| `digest-differs:<algos>` | the digests disagree; `<algos>` are the ones that **disagreed**, not the ones requested |
| `digest-matches:<algos>` | digests agree (identical pairs; see `--show-identical`) |
| `stat-match` | every trusted stat field agrees and no digest was consulted |
| `unverifiable:<fields>` | no digest was available and a difference this run distrusts is unresolved |
| `dir-present` | a directory on both sides — compared by presence |

Three things worth knowing:

- **One tag per *decision*, never per check.** The diff is a short circuit, so the
  tag names whatever settled the pair. A stat difference needs no digest at all, and
  mentioning the digest that was *skipped* would be reporting an effort, not a reason.
- **Only `CHANGED` gets one.** `MISSING`, `EXTRA`, `TYPE-CONFLICT` and `CASE-MISMATCH`
  are self-explanatory — the bucket name is the reason. (`IDENTICAL` records get one
  too, when both flags are passed.)
- **`unverifiable` means "cannot tell", not "differs."** It is reported as `CHANGED`
  because a pair whose only evidence of difference was ignored cannot be called equal,
  and folding it into a stat tag would claim a difference the run was told not to trust.

`digest-differs` names the algorithms that **actually disagreed**, which under
`--hash-any-of` is not the list you asked for — the planner settles a pair with one
digest, and the tag says which. A parser splits on the first `:`; the JSON form is a
flat string (`{"event":"changed","path":"a.txt","why":"stat-size"}`), not an object.

**Default off, and it changes nothing but the text** — not the exit code, not the
counts, not the work done. An unflagged run is byte-identical to one without the flag.

`--output` affects **stdout only**. Diagnostics stay on stderr at every
`--log-level`, so `--output json` is pipeable into a parser with no filtering:
```powershell
.\target\debug\girsync sync --src D:\game-old --dst D:\game-live --dry-run --output json |
    ConvertFrom-Json | Where-Object event -eq copy | % path
```

Each record is *also* logged, as a `debug` `tracing` event carrying the same
`event` name and fields, so a `--log-file` contains the same data plane without
being asked for it twice in two shapes.

### sync

Mirrors `src` → `dst`. It prints its **plan** first — the diff, exactly as
`compare` reports it — and then the actions, as it takes them.

#### The plan print, and why there is only one summary

```
.\target\debug\girsync sync --src D:\game-old --dst D:\game-live --dry-run --why
MISSING only_src.txt
CHANGED changed.txt why=stat-size
COPY only_src.txt because=missing
COPY changed.txt because=changed
SUMMARY renamed=0 mkdir=0 copied=2 ... dry_run=true
```

`because` is the **bucket**, not the reason: `missing`, `changed` or `type-conflict`.
It is coarser than `why=` on purpose — the reason a path is `CHANGED` lives on that
path's plan line, and repeating it on the action would put one fact in two places
with two vocabularies. The hard rule is that **an action line carries `because=` and
never `why=`**; anyone wanting the join has both records for the same path.

The stream is three segments, in this order:

| segment | what it is | when |
| --- | --- | --- |
| `RENAME` | dst's casing aligned to src's | before the diff — it has to be, or the diff has no exact keys to compare |
| the **plan** | the diff, through the same `report::verdict` `compare` uses | up-front, once the diff exists |
| the **actions** | what the apply phases do, in apply order | as they happen — so under `--jobs > 1` the `COPY` lines come out in **completion** order, not plan order |

Two things follow from that split, and both are the point:

- **The plan line answers *"why did it decide that"*, the action line answers *"what
  is this tool doing"*.** A `COPY` names the bucket that caused it and never repeats
  the reason, which lives on that path's plan line. A consumer wanting every
  `CHANGED` with its cause reads the plan; one wanting what the run did reads the
  actions. Neither has to join streams.
- **A `MISSING` file appears twice** — once in the plan, once as a `COPY`. That is
  deliberate: it is what makes the plan independently parseable without correlating
  it against what followed.

**The exit code stays `0`.** Printing a plan makes `sync` *look* like `compare`, and
`compare` exits `4` on a difference. `sync` must not: its exit code reports whether
the run *succeeded*, not whether it had anything to do, and returning `4` would break
every script that syncs a partly-different tree — the normal case. The diff never
reaches `report_diff`, so `Diff::total()` is not an exit code on this path.

There is exactly one `SUMMARY`, and it is `sync`'s own — counting actions. The diff's
own summary is dropped: two differently-shaped summaries in one stream would make
"the last line is the summary" true only by luck.

Mirrors `src` → `dst`:

1. Abort if `--src`/`--dst` resolve to the same cache, if either side is a record
   path, or on any IO error
   (dangling symlink, walk loop, locked file — first error aborts the run).
2. Backup `src` + `dst` DBs to `girpr-cache-backup-<ts>`, plus snapshot dst to
   `girpr-cache-old-<ts>` — before any write. `current` is authoritative; `old-*`/`backup-*` are never auto-read.
3. Stat both trees, then plan **across the pair** (see "Cache is a cache, not
   truth"): a file is read only if it is on both sides with equal size+mtime.
   Insensitive mode: rename dst paths to src casing (`RENAME`, two-step for
   Windows) — after the plan, before the diff, so both sides are on exact keys.
4. `COPY` missing + changed files (skipped when `--missing-only`), `MKDIR` missing dirs,
   `DELETE` extra files + remove dst-only dirs deepest-first (both skipped when
   `--keep-extra`, which spares directories as well as files).
   Type-conflicts resolve toward src kind.
5. Copy = truncate + write in place, preserve mtime, verify-after-copy by rehash
   (size+mtime under `--hash-all-of none`); the dst record entry is deleted *before* each
   file change. The `COPY` line is printed **as each copy completes**, so under
   `--jobs > 1` those lines come out in completion order. No resume.

**The plan in (3) runs before the rename in (3), and that ordering is load-bearing.**
The planner pairs the two sides as they are *on disk* — by lowercase in
insensitive mode — and the rename then collapses exactly those pairs onto exact
keys, which is what the diff compares. Pairing by exact key instead would make a
case-only pair look one-sided, skip the digest it needs, and leave the diff
comparing two entries where one has no digest; `hashes_differ` is silent on a
missing digest, size+mtime agree, and the run reports success having copied
nothing. `tests/sync.rs::a_case_only_difference_in_content_is_copied_not_merely_
renamed` exists for exactly that, because every other case-only fixture uses
identical bytes and would pass either way.

**Laziness moves work between runs rather than only removing it, so the honest
summary is a trade.** Measured on five src files / three dst files / two src-only,
cold caches:

| run | before | after |
| --- | --- | --- |
| `sync` #1, dst has drifted | 8 files read | **0** |
| `sync` #2, nothing changed | 0 read | 5 read |

Run #1 is the common case and the large win: the pairs size settled are never
opened. Run #2 is the cost — a lazy scan records stat without a digest, and the
only thing that writes a digest during a sync is `copy_one`, which records into
**dst's** cache, so a second run finds stat-equal pairs whose src side has no
digest and must rehash src. In the documented workflow (`update` src → `compare`
→ `sync`) the caches are already populated before `sync` runs and both versions
cost nothing, so the cost only appears when `sync` is run repeatedly with nothing
in between — which is when there is nothing to copy.

The pre-drop in (5) is belt-and-braces, not the safety mechanism — every way a
copy can die is already caught by size (truncation makes a partial file
strictly smaller), by mtime (stamped only after a complete write), or by the
pre-copy digest no longer matching. It stays because it costs one batched write
and depends on none of those. Do not treat a *missing* row as a crash signal,
though: a scan may legitimately record stat without a digest, so absence proves
nothing.

`--dry-run` reports `MKDIR`/`FIX-DIR`/`DELETE`/`COPY`/`RMDIR` + `SUMMARY` in
apply order and writes nothing (no backups, no cache updates, no FS changes). Its
records are the same ones a real run reports, with `dry_run=true` on the
`SUMMARY`; see the `--dry-run` section below.

### `--dry-run`

Every subcommand takes it, and it means one thing everywhere: **write nothing at
all** — no file tree change, no cache created, no cache updated, no backup.

The flag is on `CommonArgs`, not on one command, because that is the only way it
stays one flag. `sync` additionally threads it into the rename and apply phases,
which are the only filesystem writes in the crate; the cache half reaches every
command through `ScanMode::dry_run`. `compare-self` accepts it and warns, since it
opens the record read-only and resolves the folder side against an in-memory
handle — a script passing `--dry-run` everywhere should not break on the one
command that was already safe.

**A dry run must answer the same question, not a smaller one.** This is the part
that is easy to get wrong and had never been enforced here: a dry run still
stats the tree, still reads cached digests, and still hashes whatever stat alone
cannot settle. It does the same work and takes the same decisions; only the writes
are gone. Skipping the hashing would make it cheaper and *wrong* — it would report
the answer to a different question. That is also why `SideCapability`'s
`can_hash_from_disk` stays true under a dry run.

The testable form of that is **`ScanStats` is identical in both modes**, and it
is asserted as a whole struct rather than field by field, so a counter added
later cannot quietly start reporting a write's outcome instead of a decision.
Two bugs that assertion caught, both instances of one mistake — gating a
*decision* or a *count* behind the write handle instead of gating only the write:

- The prune set was computed inside `if let Some(w) = batch`, so a dry run never
  worked out which rows were stale and reported `pruned = 0` over a tree a real
  run prunes. How many rows *would* be dropped is an answer; only the `remove`
  calls are writes.
- A corrupt cache fell back to an in-memory one under `--dry-run`, "so a dry run
  still answers rather than dying on a cache it is not allowed to rebuild". A real
  run exits `3` with no answer at all, so this made a dry run report a confident
  verdict over a cache neither mode could read — worse than crashing, because it
  looks like an answer. It now refuses exactly as a real run does.

Enforcement is structural rather than per-command: `effective::open_folder_cache`
is the single place a folder's cache is opened, so no command can honour the flag
incorrectly by opening the wrong kind of handle. Under `--dry-run` it opens an
existing cache read-only, and serves a missing one — or one `--ignore-cache` would
have discarded anyway — from an in-memory DB; each of those still yields the same
effective map a real run would. `--dry-run --ignore-cache` is thus the one
combination that asks for something a dry run may not do, so the cache is treated
as *absent* rather than rebuilt; the folder is scanned from disk alone, which is
what a real `--ignore-cache` run does too. A **corrupt** cache is deliberately not
in that list: it is the one state where the cache is neither absent nor discarded,
and it is a hard error in both modes.

`sync`'s two outputs are the same document. The `SUMMARY` uses identical field
names — `renamed`, `mkdir`, `copied`, `deleted`, `rmdir`, `missing_only`,
`keep_extra` — and the action records share one vocabulary and one order
(`MKDIR`, `FIX-DIR`, `DELETE`, `COPY`, `RMDIR`, which is apply order). The dry
run's only addition is `dry_run=true`, so a caller reading either does not have to
know which it got.

`a_dry_run_reports_what_a_real_run_reports` compares the two modes and is now
explicitly a **two-segment** comparison. The plan and the renames are decided
before anything is written, so they are compared **ordered, field for field**. The
actions are reported as they complete, and completion order is a function of I/O
timing rather than of the plan, so they are compared as a **multiset** — with the
count asserted separately, so a duplicate cannot hide in a set. That weakening is
unavoidable rather than something to engineer around: the alternative is to keep
emitting after the pool joins, which is the behaviour the as-done printing removes.

**A dry run and a real run are no longer comparable as one ordered document.**
`COPY` lines are emitted inside the copy worker, so a run with `--jobs 4` prints
them in whatever order the files finish — 24 padded files under `--jobs 4` came out
shuffled on 8 of 8 runs. The *set* and the *count* are unchanged and are what the
tests assert; the order is not, and no test may depend on it.

The emit is serialised by a leaf mutex rather than left to each thread's
`println!`. `Report::emit` prints a line and logs a matching event as one unit, and
without it two workers could interleave between the two, putting a log event about
one file between the line and the event for another. The lock is held for exactly one
emit and never across the copy, so it does not serialise the work it protects.

`rmdir` is exact rather than omitted, which is why it is *planned*:
`build_plan` decides the set (see `Plan::rmdir`), so the same count is available
before anything is written as after.

That plan step is also where the one genuinely tricky case lives. `copy_one`
clears a dst *directory* out of the way when a planned copy lands on one, and it
does so recursively — so that directory's whole subtree is gone before the rmdir
pass could look at it. Those directories are excluded from the plan; counting
them would promise removals that cannot happen. `--keep-extra` spares the set
entirely, directories as well as files.

Tests pin the whole thing. `compare_dry_run_writes_nothing_and_answers_the_same`
asserts both caches are byte-identical and the verdict matches;
`compare_dry_run_computes_the_same_digests` asserts the two runs produce *equal
effective maps*, which is stronger than agreeing on printed verdicts — two runs
can print the same `CHANGED` lines while having hashed different files and reached
the same conclusion by luck, whereas the maps hold the digests themselves.
`dry_run_reports_the_same_counters_as_a_real_run` extends that to `ScanStats`,
and `the_dry_run_fixture_forces_hashing_of_exactly_the_undecided_pairs` guards the
fixture underneath, so neither equality can pass vacuously by hashing nothing.

## Core semantics (must-know for AI edits)

- **One writer per channel.** stdout records go through `report::Report` and
  nowhere else — there is no second `println!` a report can escape from.
  Diagnostics go through `tracing` with named fields and are never formatted into
  a message string: a value belongs in a field, not in prose. Per-command spans
  carry the whole run's config (including `output`), so every event inside is
  correlatable. `Report::emit` is where a record becomes both a stdout line and a
  `debug` log event, so those cannot disagree.
- **Cache = redb file** at `<root>/girpr-cache`. Key = `/`-separated
  relative path (UTF-8; case preserved as stored). Value = binary
  `{kind, size, mtime_ns (ns since epoch), hashes{algo→raw bytes}}` in the
  `entries` table + `meta{version, case_sensitive}` in the `meta` table.
  `girpr-cache*` (DB file, backups, olds) is always excluded from scans.
  Legacy sled directories are rejected (rebuild with `--ignore-cache` or
  convert once with `sled2redb <old-dir> <new-file>`).
- **Read-only vs read/write opens.** `open_db` takes a `CacheOpen`:
  `ReadWrite` may create the file, back it up, and reconcile `meta`;
  `ReadOnly` requires an existing, current-version cache and never opens it for
  writing, so the file comes out byte-identical and every write method on the
  handle fails. A read-only open cannot rewrite `meta` when the recorded
  `case_sensitive` flag disagrees with the run's mode; since that flag is
  informational, it warns and continues rather than refusing the cache. A wrong
  schema version is still fatal in both modes. Record sides and `sync --dry-run`
  both use read-only opens.
- **One cache per run.** A read/write open locks its file exclusively, so a run
  must never name the same cache twice. `compare` and `sync` both reject a
  `--src`/`--dst` pair that resolves to one cache file — the same folder twice,
  the same record twice, or a folder paired with the record inside it.
  Resolution is by *canonical* path, so two spellings of one target (`F/sub/..`
  vs `F`) collide too. Exit `3`.
- **Cache is a cache, not truth.** Disk governs. `update` always populates the
  requested algos. `compare` and `sync` decide **per pair**, by
  `planner::plan_pairs`: a path needs a digest only when it is a file on both
  sides with equal size and equal mtime. A `MISSING`, `EXTRA`, `TYPE-CONFLICT` or
  already-`CHANGED` path costs no read at all, and neither does a stat-differing
  pair under `--no-trust-cached-hashes` — distrusting the cache does not make an
  unequal size uncertain. A `MISSING` file is still copied, because `copy_one`
  rehashes both sides to verify and returns the record it caches; that rehash is
  the only hash such a file needs. Both commands therefore leave **stat-only
  rows** for paths they did not hash, which is a valid state for a *folder* side:
  the next run sees a fresh row with nothing cached and asks for a digest, which
  it is able to do. It is **not** a valid state for a *record* side, which is why
  the planner's coverage check exists — see below.

  The stat short circuit is `planner::StatTrust::settles`, **one predicate read by
  both the planner and the diff**. `--no-trust-size` and `--no-trust-mtime` switch
  parts of it off, and both together switch all of it off. They have to be the
  same rule in both places, and the mutation that matters is a disagreement:
  a digest the planner computes that the diff never consults, or — worse, and
  silently — a pair the planner called settled that needed a digest nobody
  computed. The planner's decision travels out in `PairPlan::stat` rather than
  being read from the flags a second time, so the two cannot drift. Both
  directions of that mutation are caught by `stat_trust.rs`.

  One consequence is load-bearing enough to state separately: the diff may only call a
  pair equal when it actually compared something. That holds for every run with a
  digest requested, and **stops holding when none is** — which is why the flags cannot
  be combined with `--hash-all-of none`. See the refusal, above.

  The same `plan_pairs` call that decides what to read also decides whether the run
  *can* be answered, and it does so over the undecided set only, before any read.
  A side's availability is `cached ∪ hashable`, with deliberately **no trust term**:
  `--no-trust-cached-hashes` asks a side to re-read rather than reuse, and a record
  has no filesystem to re-read, so the flag changes nothing a record owes. A
  non-hashable side short of a requested algorithm on an undecided pair is fatal
  (exit `3`). `resolve_record` therefore never has to check anything — the planner
  does not plan work for a side that cannot hash, so there is nothing it could have
  failed to deliver. That is what closes the last false clean: `hashes_differ` skips
  an algorithm either side lacks, so an uncovered pair used to fall back to the
  size+mtime that had already agreed and the run reported `CHANGED = 0` about
  content nobody read.

  `--hash-all-of` / `--hash-any-of` (above) is the *which* half of the same
  decision, and the planner's answer to it is also per pair:
  `PairPlan::required` maps each src path to the algorithms that settle it, and
  `diff_maps` consults that rather than a run-wide list — under `all-of` every entry
  *is* the whole requested list, so it is a superset of the old signature rather
  than a different mechanism. Availability bookkeeping is identical in both modes
  ("can this side obtain algorithm `a`" does not depend on the mode); only the
  requirement differs. Under `any-of` a side with a filesystem can therefore never
  be the short one, which is why an `any-of` coverage failure always names a
  record — by construction, not by a guess at which side it was.

  The two modes are not equally strict about *which* algorithms fail, and conflating
  that is a real trap: under `all-of` every algorithm a side cannot obtain is a
  failure, but under `any-of` a record short of `sha256` is perfectly answerable by
  `md5`. Recording every shortfall in both modes makes `any-of` fatal for exactly
  the records it exists to rescue. So `all-of` accumulates the shortfall and
  `any-of` defers entirely to `pick_one`, the one place that knows whether *nothing*
  is obtainable.

  Separately, and needing no knowledge of the other side, a folder side
  **corrects its own cache** as it walks: a stale row (stat no longer matches
  disk) loses its digests, and an orphan (file gone) is dropped. That is phase A
  work, so it finishes before the planner runs and the entries the planner reads
  are already self-consistent — a stat-differing pair can never be judged against
  a pre-change digest. `--no-trust-cached-hashes` cannot suppress it: distrusting
  a cache means do not *reuse* it, never keep the rows that are wrong. Both
  corrections go out through `merge_row`, which keys on the stat alone —
  unchanged stat merges over whatever the row had, changed stat drops every
  stored digest. Both corrections are decided unconditionally — a dry run reaches
  the same prune set and the same row contents — and only the *writes* are
  suppressed, so `ScanStats` is identical in both modes. Missing cache is created;
  corrupt cache errors (exit 3,
  `--ignore-cache` backs up + rebuilds).
- **Case rules.** Stored names keep their casing. `--case-sensitive` (default
  **false**): within one side, two live/record paths differing only by case is a
  fatal conflict; across sides it is `CASE-MISMATCH` (compare) or rename-dst-first
  (sync). If disk casing differs from the cached key, the key is fixed to the
  disk name first with hashes reused. Mode mixing never errors by itself — the
  meta flag is informational; the same record works in both modes.
- **Filters.** `glob`-crate patterns on relpaths; `--include` narrows, `--exclude`
  wins over include. Excluded paths are treated as nonexistent on **both** sides
  (skipped in walks, filtered from records, pruned from DBs, kept untouched in
  dst during sync). Glob case-sensitivity follows the case mode.
- **Symlinks** are followed (`--max-depth`, default 10, applies to the whole walk —
  raise for deep trees). Dangling links abort. Link targets outside the root are
  followed and materialized as plain files/dirs. Dirs compare by presence only.
- **Exit codes:** `0` ok/equal · `2` CLI parse error (clap) · `3` fatal at runtime
  (bad flag values, both sides naming one cache, IO/verify/corrupt — message on
  stderr;
  note runtime usage mistakes also exit `3`, unlike `girpr`'s `1`)
  · `4` compare found diff. `--jobs 0` is a runtime error → exit `3`.

## Typical dev workflow

```powershell
cargo build -p girsync
.\target\debug\girsync update --dir D:\game-old          # record the reference
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live   # exit 4 + diff list
.\target\debug\girsync sync --src D:\game-old --dst D:\game-live --dry-run
.\target\debug\girsync sync --src D:\game-old --dst D:\game-live --jobs 4
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live   # exit 0
```

Same, machine-readable — `--output json` on stdout, logs still on stderr:

```powershell
.\target\debug\girsync compare --src D:\game-old --dst D:\game-live --output json |
    ConvertFrom-Json | Where-Object event -eq changed | % path
```

Record-vs-folder without touching the folder's cache:

```powershell
.\target\debug\girsync compare --src D:\game-old\girpr-cache --dst D:\game-live
```

Is a folder's own cache still in step with it? (`compare-self` never writes, so
this can be run at any time; it rehashes whatever size+mtime cannot settle):

```powershell
.\target\debug\girsync compare-self --dir D:\game-live              # exit 0 = in step
```

## Gotchas

- Deletes extras **by default**; in-place truncate means a killed run leaves
  truncated files (entries were pre-dropped, so rerun re-copies — verify with `compare`).
- Backups/olds accumulate (`keep-all`, suffixed `_2…` on timestamp collision);
  clean them manually. They are `girpr-cache*` so never sync.
- `--max-depth 10` silently skips deeper files — set higher for real game trees.
- **A record that cannot digest an undecided pair exits `3`.** The easiest way to
  hit this: a lazy `sync` (or any run that decided a pair by presence) leaves a
  stat-only row in a folder's cache, and the next `compare`/`compare-self` against
  it cannot compare content for that path. It used to exit `0` and call the pair
  equal on size+mtime alone. `girsync update --dir <the folder>` is the fix, or
  `--hash-any-of` if the record holds *some* of what you asked for; the error names
  the folder, the count, the paths and the mode that failed.
- **A size-changed cache row can get stuck.** Editing a file's length drops its digests
  (a stale row's are all suspect), and a size difference settles the pair for free — so
  nothing ever rehashes it and the row stays stat-only permanently, describing content
  it cannot verify. `--no-trust-size` is the repair: it puts those pairs back in the
  undecided set. Worth knowing because the row looks healthy — it has the right size and
  mtime.
- **`hashes_differ` is silent, and silence used to read as "equal".** An empty result means
  *"no difference detected"*, which is not the same claim as *"identical"* — it is safe
  only while every pair reaching it has a digest to compare, which `Required` guarantees
  for an undecided pair. A run that requested **no** algorithm has no entry, so the
  silence became the whole of the answer and a changed pair came back clean. `pair_verdict`
  now distinguishes the two: with no digest consulted at all, a difference that was
  distrusted makes the pair `CHANGED` (*cannot confirm*, tagged `unverifiable:<fields>`)
  rather than equal. Worth knowing because nothing else surfaces it — the row looks fine,
  the counters look fine, and only the verdict is wrong.
- **`--hash` is gone.** Renamed to `--hash-all-of`, with `--hash-any-of` alongside
  it. The old spelling is rejected rather than aliased, so a script passing
  `--hash md5` fails loudly instead of quietly getting the new default. `--hash-all-of
  none` is the stat-only audit; `none` is refused by `--hash-any-of`.
- Tests: `cargo test -p girsync` (incl. case-adoption regression test, which uses a
  two-step rename since Windows FS can't hold `a.txt` + `A.txt` simultaneously).
  Integration tests live in `tests/` and are grouped by concern: `helpers.rs`
  (primitives), `cli_dispatch.rs`, `update.rs`, `compare.rs`, `compare_self.rs`,
  `sync.rs`, `sync_plan.rs` (the `sync` plan, asserted per path), `why.rs` (`--why` and `--show-identical`: a
  table of pair state -> classification -> reason tag, the two buckets' exclusions
  from the exit code, and both flags' surface through the real binary), `lazy.rs`
  (per-fixture expected verdicts *and* expected read counts, each stated before
  the code it pins), `hash_mode.rs` (`--hash-all-of` / `--hash-any-of`: the pick
  and its tiers, the per-path answer reaching the diff, and the flag surface
  through the real binary), `stat_trust.rs` (`--no-trust-size` /
  `--no-trust-mtime`: the stat short circuit switched off, measured in read
  counts, in verdicts, and in cache rows), `output.rs` (the stdout contract:
  `--output json` is the
  library's `verdict`, and stdout stays a clean NDJSON stream while the run
  narrates on stderr), with shared fixtures in `tests/common/mod.rs`. They run
  against the public API, so anything they touch must stay `pub`.
- **No goldens.** A whole-run transcript is a change detector wearing the costume
  of a specification: once no eager implementation exists to compare against, the
  only way to update one is to paste the actual output, which requires no
  understanding of whether that output is right. `sync_plan.rs` states a **table
  keyed by path** instead — each row says what a path is on each side and what the
  plan must therefore do about it — which survives a change of implementation, fails
  naming the path, and makes new behaviour a new row rather than an edited vector.
  Three things follow: the summary counts are **derived** from the plan rather than
  transcribed, so they cannot disagree with it; the `--case-sensitive` expectation
  is **derived** from the same rows by a rule, since that mode is the same fixture
  without the rename pass; and record **order** is asserted as a property (event
  kinds in apply order, each kind contiguous) rather than as a transcript — which is
  the check that can ask a new record kind where it applies. The one
  whole-document comparison left is dry-run against real-run, and it earns that:
  both are the same code path with only the write gate differing, so a divergence
  means the dry run took a different branch. It cannot catch a *systematic* error
  though — a summary wrong in both modes passes it — which is what the derived
  summary is for.
- **Tests read records, not lines.** Anything that used to match rendered stdout
  text now asserts `report::Record`s — as JSON objects via `json!`, or parsed from
  `--output json` when the binary has to run (the `RENAME`/`FIX-DIR` records only
  exist there). The text rendering has its own cases in `report.rs`. A test whose
  point is *what was reported* should not also restate *how it is spelled*.