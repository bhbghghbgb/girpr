# girpr — improvement report

**Scope.** The workspace root package `girpr` (`src/`), its `Cargo.toml`, and
its user-facing docs. `tools/` is out of scope; the shared `crates/log` and
`crates/tempdir` are mentioned only where they shape a finding (the file-log
sink in `src/logging.rs`, for example).

**What this is.** Pointers and context, not a work breakdown. Each finding
names the module/function that owns the issue, why it matters, its risk and
blast radius, the expected gain, and the trade-off that makes it a decision
rather than a bug fix. No line ranges and no step-by-step implementation plan;
those belong to the change that acts on a finding.

**Constraints assumed.** Behavior may change freely. Tests that pin current
observable behavior are not blockers and may be rewritten. No benchmarking was
performed; performance claims are theoretical (I/O count, round-trips, passes
over the data) and should be confirmed against a real install before being
treated as measured.

## How to read the risk/gain labels

| label | meaning |
|---|---|
| **Risk** | chance that the change introduces a regression *and* how hard it is to notice |
| **Blast radius** | how much of the pipeline the change can affect (one function, one phase, one mode, the output contract, the whole run) |
| **Gain** | wall-clock, I/O, memory, or correctness value, with the scale it applies at |
| **Trade-off** | what the change gives up or makes harder; the reason it is a choice |

Risk levels in this report are deliberately conservative: a change that is
mechanically small but sits on a data-loss path is **Medium**, not Low.

## Executive summary

The highest-value items, in the order the evidence supports:

1. **The data-plane writer can panic on a closed stdout pipe** (`report::emit`).
   That turns a successful repair into exit 101 with no `SUMMARY`.
2. **Metadata and manifest fetches are serialized even though they are
   independent.** This is the largest pure-latency win in the tool, and it is
   contained to two modules.
3. **`--scan` reads the whole install, then the repair reads it again.** The
   scan already computed the exact per-chunk verdicts the repair needs; reusing
   them (guarded by a stat check) can remove one full read per repaired file.
4. **Blocking filesystem work runs directly on async workers**, and
   `--check-only` is single-threaded despite `--io-threads`. The check path can
   reuse the scan path's existing bounded, `spawn_blocking`-based shape.
5. **A scoped run never takes the single-writer lock.** Scope is treated as
   "paths outside it do not exist", but that also means two mutating runs can
   overlap and write the same files.
6. **The in-place verify pass hashes the whole file *and* every chunk range**,
   even though the per-range verdicts are only needed after a mismatch; the
   final redo scan computes range verdicts that are never read.
7. **The temp path re-opens and re-stats the source once per chunk**, and the
   chunk sink issues a seek per decoded piece. Both are per-chunk syscall
   multipliers on files with thousands of chunks.
8. **Relative paths are re-keyed and re-stored in several side tables**
   (`url_prefix_by_file`, `DiskPlan::per_file`, `ScanReport::files`, the purge
   expected set). That is allocation and spelling-drift risk that a dense file
   id or a field on `PlannedFile` would remove.

Cheap cleanup with near-zero behavioral risk sits in the dependency/build and
documentation-drift sections; it is worth doing early because it shrinks the
surface every later change has to reason about.

## 1. Correctness and robustness

Ordered by "fix first" within the category: the first two can lose or corrupt
observable state; the rest are correctness edges or hardening.

**Recommended order within this category:** C1 → C2 → C3 → C5 → C4 → C6 → C7 →
C8 → C9.

### C1 — stdout writes can panic on a closed pipe

**Where:** `report::emit`, the single writer for every `REPORT` / `PROGRESS` /
`PLAN` / `SUMMARY` record (the progress ticker uses it too).

**Why it matters:** `println!` panics when stdout's reader has gone away
("failed printing to stdout: Broken pipe"). A launcher that closes the pipe, or
`girpr … | head`, turns a completed repair into a panic, exit `101`, and no
`SUMMARY` line — exactly the automation contract the README promises. Because
the panic happens after the install has been mutated, the failure is also
misleading about what happened on disk.

**Risk / blast radius:** Low to fix; whole output contract and process exit.

**Gain:** The documented exit code and a final `SUMMARY` survive a closed
consumer; the log mirror still carries the record.

**Trade-off:** None meaningful. Write through a locked stdout handle and treat
`BrokenPipe` as a benign stop (optionally recording it), rather than letting
the macro panic. The same treatment should be applied to the ticker, whose
panic is otherwise invisible.

### C2 — a scoped run does not take the single-writer lock

**Where:** the lock acquisition gate in `repair::execute_phase`; `repair::lock`
owns the policy.

**Why it matters:** The lock is skipped when the path scope does not admit
`girpr.lock`, on the principle that a path outside the scope "does not exist".
But the run still mutates every file the scope does admit, so a scoped run and
an unscoped run (or two scoped runs) can write conflicting bytes into the same
file. The lock's whole purpose — "two runs against one install can write
conflicting bytes" — is defeated for the mode most likely to be used
unattended.

**Risk / blast radius:** Medium to change (concurrency semantics); the whole
mutating half of the run.

**Gain:** Real mutual exclusion for every mutating run, not only unscoped ones.

**Trade-off:** A lock file inside the install is the thing scope hides. The
clean alternatives are an OS-level named object keyed by the canonical game
directory, or a lock file in the system temp directory keyed by a hash of that
path. Both also remove the "out-of-scope path does not exist" contradiction;
the named-object route additionally releases automatically on process death.
Changing this means revisiting the README's lock paragraph and the tests that
assert a scoped run takes no lock.

### C3 — the audio cache move can leave a truncated target and still bump `config.ini`

**Where:** `phases::post::move_audio_cache`; the `config.ini` bump later in
`phases::post::run`.

**Why it matters:** `rename` is tried first; on failure the fallback is
`std::fs::copy` followed by `remove_file(src)`. If `copy` fails partway, a
partial target remains, the source is kept, and the function only warns. The
post-phase then continues and bumps `config.ini`, so the launcher is told the
install is current while an audio pack may be truncated. The next run would
overwrite it, but between runs the game can consume the bad file.

**Risk / blast radius:** Low to fix; post-phase + audio only.

**Gain:** A best-effort step can no longer publish a partial file as if it were
complete, and the post-phase's "bytes are in place" premise stays true.

**Trade-off:** The copy fallback needs a temp sibling in the destination
directory plus a rename (and cleanup on failure), or at minimum a target
cleanup plus an explicit "not moved" outcome. Either adds one path manipulation
on a cold path. Report the outcome through the existing `moved`/`kept` log
line rather than silently retrying.

### C4 — `--plan-out` is written non-atomically and materializes the document twice

**Where:** `scope::report::PlanDocument::write_to`.

**Why it matters:** The JSON is serialized to a `String`, then copied again by
`format!`, then written with `std::fs::write`. A full disk or a crash leaves a
truncated artifact where the caller asked for a parseable plan. Everything else
the tool publishes (`config.ini`, the audio scan file) uses temp-then-rename;
this one does not, which is inconsistent for an artifact consumers may treat as
input.

**Risk / blast radius:** Low; `--plan-out` only.

**Gain:** No truncated artifacts; lower peak memory on large plans (one
serialization buffer instead of two).

**Trade-off:** Serialize straight to a buffered writer on a temp path in the
target directory and rename. This adds a temp file beside the requested path
(which the user may not expect) and needs the same cleanup-on-error discipline
as the other writers.

### C5 — a JSON number larger than `i64::MAX` silently becomes negative

**Where:** `hyp::num_from_string`, specifically the unsigned visitor.

**Why it matters:** Manifest `compressed_size` / `uncompressed_size` accept
either a number or a numeric string. The unsigned path casts `u64` to `i64`,
which wraps. A corrupt or hostile server value becomes negative and is then
clamped to zero by `.max(0)` throughout the size and estimate code (free
download accounting, zero reservations); some range validation paths may treat
it differently. The failure is silent, which is the problem.

**Risk / blast radius:** Low; metadata parsing, then every size consumer.

**Gain:** Malformed sizes become a visible metadata error rather than a silent
accounting lie.

**Trade-off:** A legitimate size above `i64::MAX` would now fail the run. That
is not a real manifest, so the trade is acceptable; use checked conversion and
a descriptive error.

### C6 — the byte meters trust their callers not to underflow

**Where:** `scope::ledger::Meter` (`refund_live`, `adjust_live`, and the
no-ceiling path of `settle`).

**Why it matters:** `fetch_sub` is unchecked. Any future caller that refunds
twice, or refunds a reservation that was refused, wraps the live figure to a
value near `u64::MAX`, which switches the ceiling off for the rest of the run
— the exact opposite of a budget. Today's call sites appear balanced, so this
is hardening, not an observed bug, but the invariant lives in a comment rather
than in the code.

**Risk / blast radius:** Low; ceiling accounting only.

**Gain:** A logic error becomes a loud debug assertion (or saturating
subtraction plus a warning) instead of a silently disabled limit.

**Trade-off:** One extra compare on a cold-ish path. Debug assertions cost
nothing in release.

### C7 — `path_key` folds with Unicode lowercasing, not Windows path rules

**Where:** `util::path::path_key`, used by the plan's dedup and the purge's
keep comparison.

**Why it matters:** On Windows, `to_lowercase` is not the filesystem's
case-insensitivity relation. It can map distinct characters onto one string
(for example the Kelvin sign and `k`), while Windows considers the underlying
names different. The effect is an over-merge in the plan or an over-keep in the
purge for non-ASCII names. The game manifests are ASCII in practice, so this is
an edge, but the function is the single authority for path equality and should
not be subtly wrong.

**Risk / blast radius:** Low; plan dedup and purge classification.

**Gain:** Path equality matches the platform the install lives on for the
spellings that actually occur.

**Trade-off:** An ASCII-only fold is the pragmatic choice for manifest paths,
but it is less "correct" for a hypothetical non-ASCII install. Document the
choice and keep it in one place.

### C8 — temp-file naming is inconsistent between the two atomic writers

**Where:** `util::ini::write_config_ini` (`config.ini.girpr_tmp`) and
`repair::audio::write_scan_file` (`scan.girpr_tmp`, because `with_extension`
replaces the existing extension).

**Why it matters:** Two "atomic publish" implementations with two naming rules
mean a future sweep, a scope glob, or an operator looking for leftovers has to
know both. `with_extension` also surprises on multi-dot paths. Neither is a
live data-loss bug, but this is the kind of inconsistency that becomes one.

**Risk / blast radius:** Low; two writers and any test that names the temp.

**Gain:** One rule for temp artifacts; simpler cleanup and documentation.

**Trade-off:** Changing the suffix changes what a leftover from an older run
looks like. Leftovers are unexpected files and the purge already removes them,
so the migration cost is low.

### C9 — lock recovery is a 24-hour wall after a crash

**Where:** `repair::lock` (`STALE_AFTER`, `is_stale`), and the `pid=` body it
writes but never reads.

**Why it matters:** The staleness test is the file's mtime only. A killed run
blocks every later run for up to 24 hours even though the body names a pid that
is obviously gone. The lock is safe, but operationally it can look like a hang
or a permanent refusal.

**Risk / blast radius:** Low if solved with C2's lock redesign; otherwise
medium (pid liveness is racy on pid reuse).

**Gain:** Recovery in seconds, and the diagnostic body finally does something.

**Trade-off:** Reading the pid and probing liveness is best-effort and can be
wrong under pid reuse; an OS-level lock or a shorter, documented staleness
window avoids the guesswork. If the file lock stays, consider at least logging
the remaining time until takeover in the refusal.

## 2. Performance and concurrency

Ordered by expected wall-clock value per unit of risk. None of these was
benchmarked; the reasoning is about serialized round-trips, passes over the
data, and syscall counts.

**Recommended order within this category:** P1 → P2 → P3 → P4 → P6 → P7 → P8 →
P10 → P9 → P11 → P13 → P12 → P5. P14 is listed only to record the decision not
to change it.

### P1 — metadata calls are serialized although most are independent

**Where:** `phases::metadata::fetch` (`getGameConfigs` → `getGameBranches` →
`getBuild(latest)` → `getBuild(local)`), plus the local ignore/blacklist reads.

**Why it matters:** Only two dependencies are real: `getBuild` needs the branch,
and the local-version lookup needs the branch and the local `config.ini`
version. `getGameConfigs` and `getGameBranches` can start together, and the two
`getBuild` calls can run together. On a high-latency link this collapses two
or three request/response waves into one, and it overlaps the local file reads
with the network instead of doing them after.

**Risk / blast radius:** Low–Medium; metadata phase only, but every run pays it.

**Gain:** One or two full RTTs saved on every invocation, plus overlap of local
state reads. This is the cheapest large latency win in the tool.

**Trade-off:** The `download_mode != chunk` refusal currently happens before
anything else; with concurrency, the run may have already issued a branch/build
call it is about to discard. Decide which error wins (keep the mode refusal
authoritative and let the other futures finish or be dropped) and keep the
logging order readable. Retry sleeps are per-call and would overlap, so a
failing endpoint no longer blocks the others' retries — that is a gain, but it
changes log interleaving.

### P2 — manifests are fetched one at a time, twice

**Where:** `phases::manifest::fetch_latest` and
`phases::manifest::local_reuse_map`; `sophon::fetch_manifest` is the unit of
work.

**Why it matters:** A build has several selected manifests (the base game, res
categories, each selected audio language). Both functions loop and `await` each
fetch. The latest fetch is on the critical path for the plan; the local fetch is
on the critical path for the download estimate and the scan. This is usually
several multi-megabyte blobs per run, so serial latency is visible.

**Risk / blast radius:** Low–Medium; manifest/plan phases, memory under
concurrency.

**Gain:** Near-linear reduction in wall clock for the manifest stage, bounded by
`--io-threads` or a smaller dedicated limit. The local map's per-manifest work
is already independent and best-effort, so it benefits without any semantic
change.

**Trade-off:** Each in-flight manifest holds its decoded buffer (the largest is
tens of MiB), so unbounded concurrency trades latency for peak memory. Bound it
explicitly and say so in the log. Error semantics need care: `fetch_latest`
currently fails fast on the first error; with concurrent futures, decide
whether to cancel the rest, drain them, or report the first error in selection
order — selection order is the deterministic choice. `build_plan` sorts the
resulting files, so completion order does not leak into the plan.

### P3 — `--scan` computes the repair's scan and then the repair computes it again

**Where:** `repair::scan::scan_plan` / `scan_one` versus the initial
`util::scan_reader` call in `repair::file::inplace` (and the temp path's
whole-file gate).

**Why it matters:** Both compute the same three things from the same inputs:
whole-file MD5, per-destination-chunk verdicts, and per-source verdicts
(`reuse::sources_for` + `scan_ranges` are deterministic functions of the chunk
list and the local reuse map). Under `--scan`, the plan phase reads every
planned file, then the repair reads it again to reach the same answer. For a
100 GB install that is a second full read of the damaged portion, and `--scan`
exists to make a run *more* informed, not slower.

**Risk / blast radius:** Medium; `--scan` only, but touches the repair
entry/skip gate.

**Gain:** One full read per repaired file removed, and the scan's cost is
amortized instead of duplicated. On an SSD this is a direct wall-clock win; on
an HDD it avoids seek-heavy re-reads.

**Trade-off:** The verdict is a snapshot; the file can change between plan and
repair (purge does not delete planned files, but an external process could).
Guard the reuse with a cheap `metadata` check (length and modification time)
and fall back to a fresh scan on any mismatch. The temp/discard paths need the
same treatment or an explicit "scan only feeds in-place" rule. This also
tightens the coupling between `ScanReport` and the writer, so it pairs naturally
with the D1 keying cleanup.

### P4 — blocking phase work runs on async workers, and `--check-only` is single-threaded

**Where:** `phases::purge::plan_purge` (walk + stat), `phases::post::
move_audio_cache` (walk + rename/copy), `phases::check::verify_plan` (read +
hash), and `scope::report::DiskPlan::new` (stat loop). All are called directly
from async phase functions.

**Why it matters:** These are synchronous `std::fs` loops. While they run, a
worker thread is unavailable for timers, network progress, or other file tasks.
Worse, `verify_plan` is a plain `for` loop, so `--check-only` ignores
`--io-threads` entirely and verifies one file at a time — while `--scan`, which
does almost the same I/O, already runs bounded and in parallel on the blocking
pool.

**Risk / blast radius:** Low for the phase wrappers; Medium for a full
`check`/`scan` unification (shared code path, shared progress accounting).

**Gain:** `--check-only` goes from single-threaded to device-bound on an SSD;
the purge and post walks stop stalling the runtime; progress ticks and network
timeouts keep working during long filesystem passes.

**Trade-off:** `spawn_blocking` moves work off the runtime but does not make it
free; the pool is bounded and sharing it with the repair loop can queue work
behind other blocking tasks. Parallel verification changes progress ordering
and the log order of failures (today it is plan order). If order matters, keep
results in plan order even when work completes out of order, the way `scan_plan`
collects.

### P5 — the repair loop performs synchronous file I/O inside async tasks

**Where:** `file::repair_all` / `file_task` → `inplace` and `temp` writers.

**Why it matters:** File tasks are `tokio::spawn`ed async tasks but every
open/read/write/seek is blocking. With `--io-threads` greater than the runtime's
worker count, tasks compete for workers and the runtime can stall its own
timers and network polling. The default is small (`--io-threads 4`), so this is
mostly a ceiling on scaling rather than a current failure, but the knob's
documented meaning ("concurrent files") is not what the runtime actually
provides.

**Risk / blast radius:** Medium–High; the core repair loop, every mode.

**Gain:** Predictable file concurrency independent of the runtime size, and no
runtime starvation when the user raises `--io-threads`.

**Trade-off:** The per-file loop mixes async network with sync disk, so
`spawn_blocking` around the whole file is not straightforward without an async
network bridge or a separate worker model. The cheap version is to document and
enforce `jobs <= worker_threads`, or to raise the runtime's worker count in
`main`; the real version is a redesign of the per-file driver. Do not reach for
this before P1–P4.

### P6 — the in-place verify pass hashes every chunk it just verified

**Where:** `repair::file::inplace`: the post-write `scan_reader` call and the
redo path's `final_scan`.

**Why it matters:** `scan_reader` always builds a digest context for the whole
file *and* for every destination range (and source range, if given). The first
verify after patching only needs the whole-file MD5 to decide success; the
per-range verdicts are consulted only when that MD5 fails, to name the chunks
to redo. The final scan after the redo takes `dest_ranges` but only reads
`file_md5` — its per-range digests are computed and thrown away. That is
roughly a second MD5 over the file's bytes on the success path.

**Risk / blast radius:** Low; in-place verification only.

**Gain:** Roughly halves the hashing CPU of the verify pass (the read still
happens once). On large files where the data is in page cache, CPU is the
bottleneck; on cold data the read dominates less than one might think because
the range digests are computed from the same buffer.

**Trade-off:** On mismatch, locating the still-bad chunks needs a second read
(or a second pass over the same buffer if retained). Mismatch is rare, so this
is a good trade, but the redo path's diagnostic ("which chunks are still
wrong") must remain accurate. Also return `Md5` from the scan rather than a hex
`String` that callers immediately re-parse.

### P7 — the temp path re-opens the source and re-stats the temp once per chunk

**Where:** `repair::file::temp::try_reuse_slice` and its caller.

**Why it matters:** Every chunk does `out.metadata()` to read the resumed
length and, when a reuse candidate exists, `File::open(src)`. Files with
thousands of chunks therefore pay thousands of opens and stats. The length only
grows monotonically within an attempt, and the source is the same file for
every chunk, so both can be tracked/opened once per file.

**Risk / blast radius:** Low–Medium; temp path only, with a sharing-violation
fallback.

**Gain:** Removes two syscalls per chunk on the reuse-heavy path; on a cold
cache the repeated opens also cost path resolution. The effect scales with
chunk count, which is exactly the damaged-file case.

**Trade-off:** A handle held across chunks can fail differently from a fresh
open (file replaced or deleted mid-repair); keep a lazy handle and fall back to
per-chunk open, or treat a failed reuse read as "download this chunk". The
length counter must be updated only after successful writes and reset on
rollback/truncate.

### P8 — the chunk sink seeks before every decoded piece and writes unbuffered

**Where:** `repair::file::temp::ChunkFileSink::write`.

**Why it matters:** The sink is the only user of the file handle during a
streamed chunk and it writes sequentially, yet every 32 KiB piece does a seek,
then a write. The seek is defended as "one syscall per 32 KB is nothing next to
the decode", which is true of the decode but not of the syscall when the chunk
is already in memory or the disk is fast. There is also no buffering: each
piece is a separate `write_all`.

**Risk / blast radius:** Low; temp streaming path only.

**Gain:** One syscall per piece removed, plus fewer writes if buffered; the
seek only needs to happen at construction and on `restart`.

**Trade-off:** Removing the seek relies on the exclusive borrow (which the type
system already guarantees). Buffering requires a flush before the chunk digest
is finalized and before any seek, which complicates the sink's error paths; a
buffer in the sink is optional, the seek removal is not.

### P9 — the file log sink is unbuffered and mutex-guarded per event

**Where:** `logging::init_tracing`, the `Mutex<Box<dyn Write>>` writer.

**Why it matters:** A release build writes a debug-level log file by default.
Every event takes the mutex, formats, and issues a write syscall. In a run with
per-file and per-chunk debug events this is measurable, and it serializes
worker threads on the log mutex.

**Risk / blast radius:** Low–Medium; observability only, but the wrong
solution loses logs.

**Gain:** Fewer syscalls and far less contention on long runs, without changing
what is logged.

**Trade-off:** `std::process::exit` skips destructors, so a naive `BufWriter`
would lose the tail of the log. Buffer with an explicit flush at the end (and
on the fatal paths), or use a non-blocking appender with its own guard; the
latter adds a dependency and can drop records under load. Whichever is chosen,
the "log file is the record of the run" promise needs a documented durability
story.

### P10 — `--scan` clones the whole local reuse map

**Where:** `repair::plan_phase`, the call that builds `Arc::new(local_map.clone())`
for `scan::scan_plan` while keeping the owned map for later phases.

**Why it matters:** The map is one entry (plus index vectors) per reusable
chunk. Cloning it duplicates every `ReuseEntry` and the `by_key` indices. On a
large install this is a real allocation and copy burst at the worst time (just
before the plan is asserted), and it is pure duplication: both consumers only
read.

**Risk / blast radius:** Low; `--scan` and the phases that hold the map.

**Gain:** Removes a large clone and lowers peak memory, with no semantic
change.

**Trade-off:** `local_map` must become `Arc<LocalChunkMap>` from its producer
through `FileCtx` and the scan. That is a mechanical signature change, but it
touches the file context and tests. It composes well with D1 (side tables keyed
by file id).

### P11 — purge/scope bookkeeping re-keys and re-allocates per pass

**Where:** `purge::plan_purge` (expected/server-keep/blacklist sets),
`purge::mark_kept_ancestors`, `purge::keep_set`, `scope::PathScope::is_visible`.

**Why it matters:** The expected set is built by lowercasing every planned rel
on every purge pass (up to twice per run with both flags). Every kept file
allocates a `PathBuf` for each ancestor level just to insert it into a set.
`keep_set()` allocates a fresh two-element set per call. `is_visible` rebuilds
`MatchOptions` per path (cheap, but it is a value construction in the hottest
predicate) and linearly tries every pattern for every walked path.

**Risk / blast radius:** Low; purge and scope only, no behavior change intended.

**Gain:** Removes per-pass O(files) allocations and makes the walk cheaper on
large installs; a const `MatchOptions` and cached keyed expected set are
straightforward. Ancestor marking can use path prefixes/components rather than
allocating a `PathBuf` per level.

**Trade-off:** Caching the keyed expected set means it must be rebuilt if the
plan changes; today the plan is immutable after construction, so that is safe,
but it couples purge to the plan's lifetime. The `is_visible` pattern loop is
inherent to the `glob` crate; a combined matcher or a faster hasher is an
option, not a drop-in.

### P12 — every `PathRoot::join` still resolves the leaf path

**Where:** `util::path::PathRoot::join` / `resolve_existing_prefix`, called per
file in plan, scan, check, repair, and disk reporting.

**Why it matters:** The base is resolved once, which was the big win, but each
join still walks the joined path to its nearest existing ancestor and
canonicalizes it, then compares against the cached base. That is the
containment check working as designed, but it is also a filesystem walk per
file per phase; the same file can be resolved several times in one run.

**Risk / blast radius:** Medium; path containment is security-relevant, so a
cache can weaken it.

**Gain:** Per-file path resolution is removed or amortized, helping `--scan`,
`--check-only`, and big plans.

**Trade-off:** A per-directory cache assumes the directory's resolved form does
not change during the run (a symlink could be swapped), which is a TOCTOU
window the current code deliberately does not have. If the install is trusted
during a run (it is single-writer after C2), a cache with a documented
assumption is defensible; otherwise leave it. Do not "optimize" this into a
lexical check.

### P13 — the meters take a CAS even when no ceiling was requested

**Where:** `scope::ledger::Meter::charge_live` / `settle`.

**Why it matters:** The no-ceiling sentinel is `u64::MAX`. `charge_live` still
enters the compare-exchange loop; `settle` still CASes to report "within
limit". These run per reserved chunk and per settled write, and the write meter
is touched per copied piece, so under `--io-threads N` the contention is
nonzero even though the answer is always "yes".

**Risk / blast radius:** Low; accounting only.

**Gain:** A branch to `fetch_add` / `fetch_add` + `true` when the limit is the
sentinel removes the loop and the contention on unflagged runs (the common
case).

**Trade-off:** Race with a limit installed later; the code already installs
ceilings before tasks start, so the fast path can be entered only after that
point, or re-check the sentinel each call (which is still cheaper). No
behavioral change.

### P14 — per-file chunks are sequential by design; do not change it blindly

**Where:** the repair loop's chunk iteration.

**Why it matters / non-finding:** Fetching a file's chunks concurrently would
increase network parallelism, but the design deliberately keeps chunks
sequential per file for HDD friendliness and for the in-place writer's
overwrite/salvage ordering. This should stay a documented choice unless a real
measurement on SSD shows otherwise.

**Risk / blast radius:** High if changed; core writer semantics.

**Gain:** Potentially better link utilization on SSD/NVMe for large files.

**Trade-off:** The in-place path's source survival depends on write order;
concurrent writes would invalidate `reuse::salvage_order`'s proof. Keep this out
of scope unless the writer is redesigned.

## 3. Clean code, duplication, and structure

Ordered by how much future change each one blocks: the first item is the reason
several other cleanups are hard to do safely.

**Recommended order within this category:** D1 → D3 → D2 → D7 → D4 → D5 → D6 →
D8 → D9 → D10.

### D1 — the relative path is copied into several side tables

**Where:** `RepairPlan::url_prefix_by_file`, `scope::report::DiskPlan::per_file`,
`scan::ScanReport::files`, the `PlanDocument` entries, the purge expected set,
and the local reuse map are all keyed by the same rel string that
`PlannedFile::rel` already owns.

**Why it matters:** Every producer must clean the path the same way, every
consumer pays a `String` hash and lookup, and a spelling difference between two
side tables is a silent miss (the code's comments repeatedly justify why a key
is cleaned "here"). It also makes `--scan` clone-heavy (P10) and makes per-file
metadata awkward: to add "the URL prefix" or "the disk class" to a file you
have to add a parallel map.

**Risk / blast radius:** Medium; many modules and test constructors.

**Gain:** One source of truth per file, fewer allocations, and cheaper
per-file access. New per-file facts become fields on `PlannedFile` rather than
new maps.

**Trade-off:** `PlannedFile` grows, and tests that construct it by hand need
updating (allowed). A dense `FileId` index is the alternative and is better for
`ScanReport`/`DiskPlan`, but it makes debug output less direct. Choose one
convention and apply it consistently; do not end up with both a field and a map
for the same fact.

### D2 — retry/backoff scaffolding is written four times

**Where:** `hyp::get_node` and `hyp::get_direct`; `sophon::fetch_manifest` and
`sophon::stream_chunk_to` plus their `Attempt` enum; the per-file retry loop in
`repair::file`.

**Why it matters:** Each copy re-implements the same shape (attempts, last
error, linear sleep, terminal-vs-retryable classification) with slightly
different details. Divergence is already visible: the HTTP family has retcode
classification, the blob family has status classification, and the file family
adds its own cap. A new retry site will copy one of them and inherit whichever
quirks are nearby.

**Risk / blast radius:** Low–Medium; network and file retry behavior.

**Gain:** One place to add jitter, one place to change the attempt count, one
place to log attempts consistently. Backoff currently has no jitter; N files
hitting a transient error retry in lockstep, which is both a thundering-herd
risk and a log-spike risk.

**Trade-off:** A generic async retry helper over different error classifiers can
become harder to read than the four straight-line loops. Keep the policy
(tables of retryable statuses/retcodes, attempt count) separate from the loop
and let each module supply only the classifier; that preserves readability.
Jitter changes timing, so tests that assert sleep durations need to move to the
`Sleep` seam and assert counts, not values.

### D3 — two atomic-write implementations, two naming rules

**Where:** `util::ini::write_config_ini` and `repair::audio::write_scan_file`
(also the `--plan-out` writer, C4).

**Why it matters:** Each has its own temp name, error cleanup, and flush
policy. The scan-file writer does not fsync; the config writer does. That may be
correct for their different durability needs, but the difference is implicit.

**Risk / blast radius:** Low; three call sites.

**Gain:** One helper with an explicit `durable: bool` (or two named helpers),
consistent temp naming, and one place to get cleanup right.

**Trade-off:** A shared helper has to accept the "dry-run" case or exclude it
(the config writer's `dry_run` parameter is unreachable in production — see
D6). Keep the helper small; do not fold `--plan-out`'s serialization into it.

### D4 — three rel-path joining helpers

**Where:** `purge::rel_path` (strip + lossy + slash), `post::join_rel`
(dir string + `Path`), and `util::clean_rel`/`PathRoot` for server rels.

**Why it matters:** They answer slightly different questions, but each is a
place where separators and lossy conversion can differ. The purge and the post
phase both walk the same tree and should agree on the spelling they report.

**Risk / blast radius:** Low.

**Gain:** One spelling rule for on-disk paths, matching the server-rel rule
where they meet.

**Trade-off:** `rel_path` works from an absolute walk path; `join_rel` works
from a rel dir and a `Path`. A shared helper needs both entry points, and the
lossy conversion is deliberate for non-UTF-8 trees; keep it explicit.

### D5 — the per-range → per-chunk verdict remap is duplicated

**Where:** `repair::file::inplace::source_ok_by_chunk` and the equivalent step
inside the scan's own estimate logic.

**Why it matters:** The mapping is small but subtle (a chunk with no source
must stay `false`, and the discard path substitutes an `absent` scan). Two
copies means the next change to source collection has to find both.

**Risk / blast radius:** Low.

**Gain:** One tested definition of "which chunk owns this range".

**Trade-off:** The scan and the writer use the ranges differently; keep the
helper in the scan/range module and let both call it rather than forcing a
shared struct.

### D6 — test-only API and unreachable parameters in production types

**Where:** `report::Record::event`, `RepairPlan::expected_paths`, and
`write_config_ini`'s `dry_run` parameter.

**Why it matters:** `event()` duplicates the lowercase/JSON event naming that
the JSON renderer builds inline, and is used only by tests; `expected_paths`
is a convenience used mainly by tests; `dry_run` is always `false` from the
post-phase because read-only runs never reach it. They read as production
surface and invite misuse (someone will call `event()` instead of the renderer
and get a second spelling).

**Risk / blast radius:** Low.

**Gain:** A smaller, honest public surface; one event-name implementation.

**Trade-off:** Removing them means changing tests. Either make the production
renderer call `event()` (then keep it) or mark them `#[cfg(test)]`/remove; do
not leave the duplication.

### D7 — documentation has drifted from the code

**Where:** several module docs, listed below.

- `phases::metadata` says the deprecated-file list is lazy and lives in the
  post-phase; the list is fetched in the plan phase (`repair::plan_phase`) and
  passed to the post-phase.
- `phases::manifest` says manifests are fetched "in order"; that is true today
  and would be false after P2, so the doc is also a coupling point.
- `inplace` says verification is "one more read"; it is one read but also a
  second digest for every chunk range (P6).
- The README's `--dry-run` paragraph describes purge listing but not the fact
  that deprecated deletes and the audio move are not even logged (A3).
- `write_config_ini` documents a dry-run mode that production cannot reach.

**Why it matters:** This codebase uses comments as design records; a stale one
is worse than none because it is trusted. The metadata/post contradiction is
exactly the kind of thing that makes a future refactor move a network call to
the wrong phase.

**Risk / blast radius:** None to change; high value for maintainability.

**Gain:** The next reader can trust the design notes.

**Trade-off:** None. Update docs in the same change that alters behavior, not
as a separate "doc pass" that will drift again.

### D8 — a few functions are doing too much

**Where:** `repair::plan_phase` (the whole plan half, many numbered steps and
early returns), `repair::execute_phase` (every mode's exit path),
`file::inplace::attempt_inplace_inner` (open → scan → discard → tier → capture →
patch → gaps → verify → redo), `purge::plan_purge` (walk + classify + stats +
dirs), and `util::ini::write_config_ini` (parse + merge + render + publish).

**Why it matters:** These are the functions most likely to change, and their
tests are mostly end-to-end because the pieces are not separable. The numbered
comments are excellent, but they describe a sequence that only exists inside
one function.

**Risk / blast radius:** Medium; refactor churn in a heavily tested pipeline.

**Gain:** Testable seams for the plan steps, the write-tier decision, and the
purge classification; smaller diffs for future phase changes.

**Trade-off:** Extracting a phase struct or an enum of plan outcomes is easy to
over-engineer. Prefer small pure helpers (decision tables, builders) over new
trait objects, and keep the existing comments next to the extracted step.

### D9 — `#[allow(clippy::too_many_arguments)]` marks four parameter bundles

**Where:** `report::report_record`, `file::temp::try_reuse_slice`,
`sophon::stream_chunk_to` / `try_stream_chunk`, `file::temp::write_downloaded_chunk`.

**Why it matters:** Each allow is a small admission that the function takes a
bag of related inputs. In the streaming path the bag is the fetch context; in
the report path it is the provenance fields. A params struct documents the
group and makes call sites readable.

**Risk / blast radius:** Low; local signatures and call sites.

**Gain:** Fewer positional mistakes, easier extension, and the allow comments
can disappear.

**Trade-off:** Structs add a name and a construction site. For the streaming
path, a `ChunkFetch` struct also makes it natural to carry the retry policy
(D2).

### D10 — exit codes are magic integers spread across module boundaries

**Where:** `config::RunFailure` (typed), but `repair::run`, `execute_phase`,
`plan_phase`, `finish`, `apply_run_flags`, and `check::verify_plan` return or
compare bare `i32`s (`0`, `3`, `4`, `5`).

**Why it matters:** The exit-code contract is the product's automation API, and
it is enforced by remembering which integer means what at each return. A new
early return can silently pick the wrong code; the README and the code can
drift.

**Risk / blast radius:** Low–Medium; every exit path.

**Gain:** One enum with named variants, `From<RunFailure>` for the fatal path,
and exhaustiveness checking at the summary site.

**Trade-off:** A little ceremony at each return; keep the `i32` conversion in
one place (`main`) so the process boundary stays simple.

## 4. API and behavior design

These are choices, not defects: they define what the tool promises. Ordered by
how much they constrain future changes.

**Recommended order within this category:** A1 → A2 → A3 → A4 → A5.

### A1 — the library exposes almost every module as `pub`

**Where:** `lib.rs` (`pub mod biz`, `cli`, `config`, `hyp`, `logging`,
`repair`, `report`, `sleep`, `sophon`, `util`).

**Why it matters:** The binary is the product; the integration tests are the
only other consumer. Exposing `hyp`, `sophon`, and the repair internals makes
every internal type a compatibility surface, discourages moving code between
modules, and lets tests couple to implementation details (which then slows the
refactors in sections 2–3).

**Risk / blast radius:** Medium; integration tests import these modules today.

**Gain:** A small, intentional surface (`RunCtx`, `run`, `Summary`,
`OutputFormat`, `Biz`, the CLI types) with internals free to change.

**Trade-off:** Tests that construct `SophonChunk`, call `sophon::fetch_manifest`,
or drive `RepairPlan` directly must move behind a test feature or use the public
seam. That work is worth doing once, but it is real work; do it after the
behavioral fixes so the tests are not rewritten twice.

### A2 — `--io-threads` names and bounds something the runtime does not control

**Where:** `cli::Args::io_threads` and the README's "Concurrent files" row.

**Why it matters:** The flag bounds in-flight file tasks, not tokio worker
threads. That is a fine user-facing meaning, but the name suggests otherwise,
and there is no upper bound: `--io-threads 1000` creates a thousand concurrent
file tasks, each doing synchronous I/O and network work, which is a resource
exhaustion foot-gun rather than a speed setting. The argument-level validation
only rejects zero.

**Risk / blast radius:** Low–Medium; CLI contract and the repair loop.

**Gain:** A name (or help text) that matches the semantics, plus a sane cap or
warning.

**Trade-off:** Renaming is a breaking CLI change; an alias (`--jobs` with
`--io-threads` deprecated) avoids it. A hard cap removes a legitimate power-user
setting; a warning above the worker count preserves it.

### A3 — `--dry-run` does not exercise the post-phase

**Where:** `repair::execute_phase`'s `readonly` gate around
`phases::post::run`, and the dry-run-only purge listing that replaces it.

**Why it matters:** A dry run logs what the repair would write and what the
purge would delete, but it never calls the deprecated-delete or audio-move
steps, so it cannot answer "what would the post-phase do?" — even though
`post::run` already threads a `dry_run` flag through each destructive step. The
only reason that flag is reachable today is `--purge-only`, not `--dry-run`.

**Risk / blast radius:** Low; dry-run only, but it changes what the mode
promises.

**Gain:** A dry run becomes a complete "what would this run do" answer, which is
the whole point of the mode; the `dry_run` plumbing in the post-phase stops
being dead for the common case.

**Trade-off:** The post-phase currently assumes the repair loop ran (it bumps
`config.ini`). A dry-run call must stop before the bump and must not count
deleted/moved bytes. That is a small gate, but it changes the mode's output
(one more set of "would …" lines), and tests that assert dry-run silence need
updating.

### A4 — emission ownership is spread across deep modules

**Where:** `report::emit` is called from `metadata`/plan, `check`, and `main`;
the progress reporter owns its own loop; `check` also emits progress inline.

**Why it matters:** There is no single place that knows the record order or
guarantees the final `SUMMARY`. That is why C1 (a panic in the writer) is a
whole-contract bug, and why progress emission is duplicated between the ticker
and the check loop. Testing "the run emits exactly these records in this order"
requires driving the pipeline.

**Risk / blast radius:** Medium; output contract.

**Gain:** One writer with one error policy, one progress driver, and an
integration-testable record stream.

**Trade-off:** Passing an emitter through the phases is more plumbing; a global
writer is simpler but harder to test. The current free function is fine as the
writer — the missing piece is the error policy and a single progress owner, not
a new abstraction.

### A5 — process exit and cleanup ownership

**Where:** `main` calls `std::process::exit` after `SUMMARY` and on fatal
errors; locks and files rely on `Drop` earlier in the pipeline.

**Why it matters:** `process::exit` skips destructors. Today that is mostly
safe because the lock is owned inside `execute_phase` and dropped before the
return, but it becomes unsafe the moment the log sink is buffered (P9) or any
other RAII cleanup is added at the top level. It is also the reason the exit
code is a bare `i32`.

**Risk / blast radius:** Low today; Medium if buffering/cleanup is added.

**Gain:** A single exit path that flushes logs, and a named exit code (D10).

**Trade-off:** Returning `ExitCode` from `main` is the idiomatic form; it still
runs destructors? It does not run destructors of values still in scope at the
exit, but it lets the async runtime shut down normally. Combine with an
explicit flush guard rather than relying on `Drop` alone.

## 5. Dependencies and build

Ordered by benefit per unit of risk. All are compile-time/binary-size wins
unless noted.

**Recommended order within this category:** B1 → B2 → B3 → B4 → B5 → B6 → B7.

### B1 — `tokio` is pulled with `features = ["full"]`

**Where:** `Cargo.toml`.

**Why it matters:** The crate uses the multi-thread runtime, macros, time,
`spawn_blocking`, `JoinSet`, and task ids; it does not use tokio's net, process,
signal, fs, or sync surfaces directly (reqwest brings its own). `full` compiles
and links far more than is used.

**Risk / blast radius:** Low; build only, but a missing feature is a compile
error (caught immediately).

**Gain:** Faster clean builds and a smaller dependency graph.

**Trade-off:** Feature lists must be kept honest as code changes; a missing
feature is obvious at build time, so the maintenance cost is low. Verify which
`rt-*` feature `tokio::task::try_id` and `JoinSet` require on the pinned
version.

### B2 — `regex` exists for one literal pattern

**Where:** `util::ini::game_version_re` (`(?m)^game_version\s*=\s*(.+?)\s*$`),
compiled once via `OnceLock`.

**Why it matters:** The INI reader already splits lines and parses `key=value`;
the regex adds a heavy dependency (and its compile time) for one anchored
pattern whose semantics are "start of line, key, `=`, trimmed value, last
match wins". It also brings a panic-on-literal `expect` into the parse path.

**Risk / blast radius:** Low; `read_game_version` only.

**Gain:** One fewer dependency; the parser and the version reader share one
line-splitting rule; no regex compile at first use.

**Trade-off:** The hand-written version must preserve the documented edge cases
(no leading whitespace, whitespace around `=`, at least one value character,
trailing whitespace trimmed, last match wins). Those are easy to test and
already tested; keep the tests and replace the implementation.

### B3 — `futures` is used for a handful of items

**Where:** `sophon::stream` (`StreamExt`, `stream::iter`), `repair::scan`
(same).

**Why it matters:** `futures-util` provides the stream extension and iterator
constructors without the `futures` facade's executor/channel/sink surface.

**Risk / blast radius:** Low; import paths only.

**Gain:** Smaller dependency graph and faster builds.

**Trade-off:** The `Stream` trait itself comes from `futures-core`; add it if
the bound needs naming. Keep the direct dependency explicit rather than relying
on a transitive re-export.

### B4 — the workspace builds the dev tools by default

**Where:** the root `Cargo.toml` workspace `members` include `tools/sync` and
`tools/capture`.

**Why it matters:** `cargo build` / `cargo test` from the root operate on the
default members, which is every member when `default-members` is unset. The
main binary's build therefore also builds the tools, and their dependencies
leak into workspace-wide feature unification (`md5`, for example, is a workspace
dependency).

**Risk / blast radius:** Low; build configuration only.

**Gain:** `cargo build` and `cargo test` at the root mean "the product", which
is what most invocations intend; CI can still build the tools explicitly.

**Trade-off:** A plain `cargo build` no longer proves the tools compile; add a
workspace-wide CI job or a second command. This is a convention change, so put
it in the README/CI at the same time.

### B5 — no release profile tuning

**Where:** root `Cargo.toml` (no `[profile.release]`).

**Why it matters:** The tool is I/O- and MD5-bound, so `lto`, `codegen-units`,
and `strip` mostly affect binary size and a little CPU; the default profile is
not wrong, but it is untuned for a single-binary CLI.

**Risk / blast radius:** Low; build only.

**Gain:** Smaller, somewhat faster release binary.

**Trade-off:** `panic = "abort"` would remove unwinding (smaller/faster) but
would bypass `Drop`-based cleanup on panic (locks, temp files) — do **not**
enable it without a panic audit. `lto = "thin"` is the low-risk choice; full
LTO trades build time for a marginal gain.

### B6 — no workspace lint policy

**Where:** root `Cargo.toml` (no `[workspace.lints]` / `[lints]`).

**Why it matters:** The code is comment-rich and internally consistent, but
`clippy` is only run manually; a few `allow`s (D9) are the visible ones. A
shared lint table keeps new code aligned without a review reminder.

**Risk / blast radius:** Low; CI only.

**Gain:** Consistent style and earlier detection of the classes this report
lists (unchecked casts, needless clones, `unwrap` in non-test code).

**Trade-off:** Lint policy churn is noise; start with `clippy::all` at warn and
a small deny list, not a blanket deny.

### B7 — `md5 0.7` is old

**Where:** `Cargo.toml`; used heavily by `util::md5` and chunk verification.

**Why it matters:** The `md5` crate's `Context` API is stable and works, but the
crate is not the maintained RustCrypto `md-5`. Performance is comparable; the
reason to consider a change is maintenance and a possible hardware-accelerated
backend.

**Risk / blast radius:** Medium; digest correctness is load-bearing.

**Gain:** Maintained dependency, potentially faster hashing (the dominant CPU
cost in scan/verify).

**Trade-off:** This is a mechanical but wide replacement with test churn, and
any performance claim needs measurement. Low priority; do it only with a
benchmark.

## 6. Tests and observability

**Recommended order within this category:** T1 → T2 → T4 → T3.

### T1 — add regression tests for the failure paths this report names

**Where:** `tests/` (binary-level) and the module test modules.

**Why it matters:** The suite is extensive on the happy and refusal paths, but
the failure modes above (closed stdout, partial copy, concurrent manifest
failure ordering, reuse of a stale scan verdict) are exactly the ones a
refactor can break silently. Some are currently untestable without driving the
whole pipeline, which is itself a signal (A4).

**Risk / blast radius:** Low; test-only.

**Gain:** The fixes in section 1/2 stay fixed, and the intended semantics
(especially P3's stat guard) are pinned.

**Trade-off:** A broken-pipe test needs a process-level harness (spawn the
binary with a pipe and close the reader); that is a new test shape. The partial
copy test needs an injectable copy/rename seam, which the post module does not
have today (the lock module does have one).

### T2 — phase timing is not observable

**Where:** every phase logs start/end facts but no durations.

**Why it matters:** The performance findings are theoretical without a way to
see where a real run spends its time. A `--io-threads`/`--scan` user cannot tell
whether the scan, the repair loop, or the purge dominates.

**Risk / blast radius:** Low; logging only.

**Gain:** One `duration_ms` field per phase (and per-file totals) makes the next
optimization decision evidence-based and validates P1–P8.

**Trade-off:** Another log field to keep stable; put it at `info` on phase
boundaries and `debug` per file to avoid noise.

### T3 — the largest integration test is hard to navigate

**Where:** `tests/repair_offline.rs` (very large) versus the smaller
`repair_machinery.rs` / `artifact_bridge.rs`.

**Why it matters:** A single huge file makes it expensive to find the case that
pins a behavior, and a change to one area recompiles all of it. It also hides
duplicate fixtures across the other test files.

**Risk / blast radius:** Low; test organization.

**Gain:** Faster iteration and clearer ownership of each contract (plan,
purge, scan, post, output).

**Trade-off:** Splitting tests is mechanical churn and merge-conflict-prone
right after a behavior fix; do it when the code changes land, not before.

### T4 — no golden test for the output contract's ordering

**Where:** `tests/cli_contract.rs` and `tests/artifact_bridge.rs` assert field
presence/values; the order and the one-record-per-event rule are implicit.

**Why it matters:** The README calls the data plane an automation contract.
Reordering `REPORT`/`PLAN`/`SUMMARY`, or emitting a second object for the same
event, is a breaking change that the current tests may not catch.

**Risk / blast radius:** Low; test-only.

**Gain:** A compact golden sequence for a scripted run (`--print-plan`, a tiny
repair, a check-only) pins the contract without pinning the implementation.

**Trade-off:** Golden output is brittle to harmless wording changes; restrict
it to event names/field keys and ordering, not free-form log lines.

## Recommended overall ordering

Weighing risk first, then value, then how much each item blocks the next:

1. **C1 (closed-pipe panic) + C6 (meter underflow) + C5 (numeric cast)** — a few
   small, independent robustness fixes that protect the contract and the
   accounting. Do these before anything touches the writer.
2. **C3 (partial audio target) + C4 (non-atomic plan-out) + C8 (temp naming)** —
   finish the "never publish a partial file" rule in the three places that do
   not follow it yet.
3. **P1 (metadata concurrency) + P2 (manifest concurrency)** — the biggest
   latency win, contained to two modules; behavior is unchanged, only timing
   and error ordering.
4. **P4 (blocking phases / check-only parallelism)** — reuses the scan's
   existing shape and makes a documented mode honor a documented flag.
5. **P6 (verify hashing) + P7 (temp source handle) + P8 (sink seek) + P10 (Arc
   local map)** — hot-path I/O and CPU removals with narrow blast radii.
6. **P3 (reuse `--scan` verdicts)** — the largest single-read win, but it needs
   the stat guard and the keying cleanup to be safe; do it after P6/P10.
7. **C2 (scoped-run lock) + A1 (public surface) + A2 (`--io-threads`
   semantics)** — the concurrency/API decisions that are better made once the
   cheap fixes have settled and the tests have been adjusted.
8. **P9 (log buffering) + P11/P13 (purge/meter micro-allocations)** — safe,
   measurable cleanup that benefits every run.
9. **D1 (relative-path side tables) + D3 (atomic writers) + D2 (retry
   scaffolding)** — the structural refactors that make the rest cheaper; they
   are intentionally late because they touch many tests.
10. **D7 (doc drift) + D6 (test-only APIs) + D8/D9/D10 (structure and exit
    codes)** — maintainability work, best bundled with the change that makes
    each doc stale.
11. **B1–B6 (dependencies/build/lints)** — independent, low-risk, and worth
    doing whenever a build happens to be touched; B4 is a convention change and
    belongs with CI.
12. **T1–T4 (tests/observability)** — add the regression tests alongside the
    fixes they cover (T1) and the timing seam (T2) early enough to validate
    steps 3–8; do T3/T4 last.

A reasonable stopping point after step 6 is a tool that is materially faster
and no less safe; steps 7–12 are about keeping it that way.

## Deliberate choices that should not be "fixed"

These are worth restating because several findings brush against them:

- **Same-file-only chunk reuse** (`sophon::map`, README S2). Cross-file reuse
  is a different feature with a different correctness argument.
- **No manifest cache inside `game_dir`** (README S1). The streamed-fetch
  design is deliberate; a cache would change disk use and invalidation rules.
- **Purge keep rules** (`purge::keep_set` + classification). They are a
  documented Collapse parity choice, including what is *not* kept. Add a
  `--keep` flag if that changes; do not quietly widen the set.
- **The audio `len == 5/10 && contains('-')` heuristic** (`sophon::filter`).
  Verbatim upstream parity; the module says not to "fix" it without checking
  Starward.
- **Chunks sequential within a file** (P14). The in-place salvage proof depends
  on write order.
- **`follow_links(false)` in the walks**, and the refusal to follow a server
  path that resolves outside the install. These are containment guarantees, not
  performance choices.
- **`kept_bytes` semantics** (bytes the repair did not have to write, including
  unverified resumed prefixes). The doc comment is precise; a refactor that
  "corrects" it would change the data plane.
- **The `readonly()` split** (dry-run/check-only vs `--print-plan` returning
  earlier). It is deliberate and documented; a new write site must still ask
  `!readonly()` and be unreachable from the plan-only returns.

## Note on tests

Several findings above change observable behavior (C1's exit path, C2's lock
rule, C3's failure outcome, P1/P2's completion order, P3's reuse guard,
A3's dry-run output). Tests that pin the old behavior are expected to change;
the recommendation is to update them with the change, not to preserve the old
behavior to keep them green. The `Sleep`/`NoSleep` seam, the temp-directory
helper, and the `Plan`/`Summary` records are the right places to keep assertions
deterministic while the implementation moves.





