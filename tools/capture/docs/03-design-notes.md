# Design notes — `gircap`

Findings, rationale and gotchas that are worth keeping but do not belong in the
spec or the schema. Nothing here is a contract: a reader who needs what a bundle
*means* wants [`00-spec.md`](00-spec.md) or
[`01-artifact-schema.md`](01-artifact-schema.md). What follows is the reasoning
behind the parts of the tool that look surprising, so that the next person does
not "fix" them.

Companions: [`02-plan.md`](02-plan.md) (shape, tests, risks),
[`04-bundle-differ.md`](04-bundle-differ.md) (the cross-binary instrument).

---

## 1. How this crate measures risk

A bundle is a **fixture other work depends on**. Re-capturing one costs a couple
of minutes and ~98 MiB, and `girpr`'s `artifact_bridge` runs against the copies
under `artifacts/`. So the risk axis for a change here is not "might it break the
build":

> **Risk = P(silently changing what a bundle says) × cost of noticing.**

Two changes with the same code-review footprint are not equally risky: one that
cannot move a byte is free, and one that moves a byte in a field `verify` does
not re-derive is close to invisible. Everything in this document is organised by
that formula rather than by "how much code does it touch".

The corollary is what to gate on: a change to the wire needs a live check, a
change to a recorded field needs a *written expectation* (§3), and a change that
only affects allocation needs nothing but `cargo nextest run`.

### The shape of the data

The numbers below are measured against `hk4e_global` 7.0.0 → 7.1.0 and are here
because several of the design decisions below are only obvious against them.
Re-measure before trusting them; they are a scale reference, not a benchmark.

| quantity | value |
|---|---|
| files in the `game` manifest | 2 744 per tag |
| chunks in the `game` manifest | 113 610 per tag |
| total chunk bytes in `game` 7.1.0 | 128.3 GB |
| manifest archive | 9 007 187 B zstd → 16 827 424 B |
| largest single chunk | 3 689 476 B compressed — this bounds the main tool's whole-chunk buffering |
| a default capture | ~98 MiB on disk: 81 files, 4 manifests, 25 stored chunks, 43 request rows |
| admitted files per capture | 6 captures, 5 in `game/` and 5 in `expected/` |
| assembled bytes per capture | 453 033 100 B |

The ratio that matters: the `game` manifest describes **128 GB** of content and a
capture retains ~48 MB of it. Everything in this tool is therefore written under
the assumption that the manifest is a *description* to be checked, not a
resource to be loaded.

---

## 2. Looked at and deliberately left alone

So nobody re-audits them.

| | |
|---|---|
| **`biz.rs`** | all `const fn` tables and `match` over a 2–3 variant enum: `Biz::resolve`, `Host::hyp_base`/`sophon_base`, `HypEndpoint::path`. **Zero per-call table construction, zero repeated lookups.** The one note is `Endpoints::owned`'s `Box::leak` — see below. |
| **`exit.rs`** | three `const fn` tables over a 5-variant enum, a `const ALL` pinned by a test, and a `From<ExitCode> for i32`. The cleanest file in the crate. |
| **`select/budget.rs`** | **zero magic numbers.** `saturating_add`, a charge that saturates, a rejected candidate is never charged, and the tie-break is documented as the caller's. Use it as the model for the rest of `select`. |
| **`report.rs` serialization** | there is **no `serde_json::Value` construction** — `to_json_line` hand-rolls the object to keep field order, which is the right call (`preserve_order` would be a workspace-wide feature-unification hazard). Values are borrowed, not cloned. |
| **`assemble/ranges.rs`** | already a **single forward pass**; there is no sort-then-merge to improve. |
| **`assemble/mod.rs` byte movement** | **no per-byte loops anywhere.** `copy_from_slice` per chunk and a zeroed `vec![0u8; size]` are already optimal. The waste is the *number of buffers*, not the copying. |
| **`assemble_side`'s three-way `buffer` / `Layout::of` / `zip`** | correct. |
| **`prost` decode** | sound. The wrapper is decoded and its `chuncks` immediately moved out, so the outer struct is not retained. |
| **Chunk-store keying** | `BTreeMap<String, Want>` keyed by `chunk.id` gives id-uniqueness *and* the deterministic order the schema requires (`into_values()`), and shared chunks correctly merge into one `Want` with multiple sources. |
| **the "re-derive, don't trust" philosophy** | correct, and the reason `verify` is trustworthy. It is *not* an argument against re-deriving: it is an argument against trusting, and the two are only the same thing when the thing being re-derived is not the thing that was read. |
| **`scan-and-verify` independence (A16)** | keep pass 2 independent of pass 1. That is what makes A16 an assertion rather than a tautology — a single pass, or two passes that share a buffer, would compare a value with itself. |
| **the `unknown_field6` passthrough** | the upstream schema carries it and its meaning is unknown; dropping it would make the full decode a lossy transform. |

### `Endpoints::owned` leaks on purpose

It is `Box::leak`, called **once per run**. The leak is bounded by construction
and nothing accumulates. `Cow<'a, str>` would be the other way to spell it and
would need every call site to thread a lifetime; `Cow` is not worth it for one
call per process, and the leak is stated in the function's own doc so nobody adds
it to a loop later without reading why.

---

## 3. The gates, and what each one cannot see

Three mechanisms, three different blind spots. Confusing them is how a real
change slips through, so they are named here rather than left to whoever hits
them.

| gate | compares | blind to |
|---|---|---|
| `cargo nextest run` | the code against written expectations | nothing about the bytes, and a lot about fields only a live capture produces |
| `tests/bundle_diff.rs` | **two captures of one binary** | a change applied **uniformly to every capture** — by construction |
| the per-document pins in `tests/capture_layout.rs` | a written expectation for the volatile documents | any **digest** in those documents, on purpose: A16 owns those |
| `tools/bundle_differ.py` | **two commits** | a change that was wrong the same way at both ends |

**The rule that falls out of the second row:** a before/after comparison cannot
see uniform drift. Two captures of one binary both get the new behaviour, so
they agree and the gate is green. Closing it needs a *written expectation*, not a
third capture — which is why the volatile documents are pinned field by field
rather than only compared.

What stays unpinned, deliberately: every digest in `MANIFEST.json`,
`verify/checksums.json` and `README.md`. A16 covers those, and they move for
`api/requests.ndjson` and its three cascade rows by design; pinning them would
mean pinning a timestamp.

The cross-binary instrument is described in
[`04-bundle-differ.md`](04-bundle-differ.md). Its own governing rule is the same
one: **a self-check that only ever compares one binary to itself cannot find a
field that one binary happens to share with itself.** That is a real failure mode
here, not a hypothetical — the mock's ephemeral port is recorded throughout a
bundle, and it reads as 13 of 58 documents changed between two commits that
changed nothing at all.

### Mutating to prove a gate works

A gate that has never been seen to fail is not known to work. Every new
expectation in this crate was checked by making the mutant it is supposed to
catch, and the two ends were confirmed: the mutant fails the pin, and every
other suite passes with it. Two that were caught only by the new pin, with
nothing else noticing:

- a total in `MANIFEST.json` moved by one
- an assertion's **wording** narrowed by three words

The second is the interesting one. `A10` can read "every reused chunk's
old-file slice MD5 matches" or "every reused chunk's MD5 matches", both under the
same id. An `id`-only check passes both. That is why `verify/selfcheck.json` is
pinned by exact wording as well as by id.

### A trap worth writing down

PowerShell's `Copy-Item` **preserves `LastWriteTime`**. Restoring a mutated file
with it therefore leaves the source file *older* than the binary cargo last built
from the mutation, cargo considers the target fresh, and `cargo build --release`
reports `Finished in 0.50s` without compiling anything.

The symptom is worse than a stale binary is usually: a capture that looks fine —
exit `0`, all selfcheck rows passing, `verify` green — while carrying none of the
field the mutation was about to delete. Every digest was correct, because the
mutant was not in anything `verify` re-derives.

**A mutation check that does not confirm the binary was rebuilt proves nothing.**
After restoring a file from a backup, force the rebuild —
`(Get-Item f.rs).LastWriteTime = Get-Date` — or delete the binary. And when a
capture's *content* disagrees with what the source says it should be, rebuild
before investigating anything else.

---

## 4. Performance: where the ceiling actually is

The tool's speed work is allocation- and algorithm-shaped rather than
codegen-shaped, and these three answers are the ones worth not asking again.

**`verify` decompresses each blob once, not twice.** The verifier has to check
every blob *and* rebuild the files from them, and those are the same bytes twice
over. One memoised decode, taken *after* `check_blobs` has had the blob in hand,
covers both without touching what is asserted. The same argument does **not**
extend to `capture`, which rebuilds each file up to three times and wants the
page cache rather than three resident copies — so the memo is opt-out, and the
opt-out's reasoning is recorded at the method.

**The request log's mutex is not the concurrency ceiling.** A process-wide lock
around the log record does not serialise the workers. What is left under the lock
is a map lookup.

**What is left is the filesystem.** On Windows, per-blob latency grows several
fold as `--jobs` rises while throughput barely moves, total CPU stays well under
one core, and the time is inside `spawn_blocking(measure_file)` rather than in the
thread handoff. A micro-benchmark on one 2 KB frame settles it: the zstd decode
is ~4% of that call, and creating, renaming and reopening a fresh file — with no
decode at all — is several times the decode.

Two consequences:

1. The cost is **per file, not per directory entry**. A `chunks/blobs/` directory
   growing from 1 000 to 8 000 entries leaves the per-blob cost flat, so "the
   directory gets slower" is not the explanation and a fix aimed at it would
   achieve nothing.
2. The file operations are **not obviously removable**, because the
   temporary-file-and-rename step is what lets `verify` trust a blob: without it
   a crash mid-write leaves a truncated file at the final path that nothing can
   tell from a good one. No change is proposed, and nothing measured suggests one
   would pay.

This is a localised finding, not an open ticket. It is written down so the next
person does not spend a day concluding that concurrency is broken.

---

## 5. Things that look like bugs

| | |
|---|---|
| `fs::rename` overwriting an existing destination | works. On Windows it is `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`, and it is pinned by a test. It is worth knowing because the opposite is a common belief, and acting on it here would mean deleting an atomic step the code needs. |
| a blob whose `stored_path` is `null` | has four causes: three failures (a source that refused, a write that failed, a task that panicked mid-measure), which say which in `fetch_error`, and a `--no-chunk-blobs` row, which is null there and means "not measured". |
| `verify` reporting a blob as red under `--no-chunk-blobs` | correct. The index lists a blob with no path, which is exactly "this bundle cannot be materialized". Red, not skipped. |
| a `probe` row with `attempts: 1` on every healthy capture | correct, and not evidence that nothing was ever retried. A `getBuild?tag=` probe *is* retried when the status calls for it; the negative probes behind `error_shapes.json` never are. |
| `identical` appearing in both `game/` and `expected/` with equal bytes | that is the class. `diff -r game/ expected/` shows it as unchanged, on purpose. |
| a `--no-chunk-blobs` capture exiting `0` | it recorded rows it could not prove as **skipped** and says `result: "incomplete"`. Read `result`, not the exit code. |
| `chunks/index.json` listing blobs that no `game/` file needs | the index is the closure of `materialize_old ∪ materialize_new` over every admitted capture, including `removed`, which has a `materialize_old` side and no `expected/`. |
