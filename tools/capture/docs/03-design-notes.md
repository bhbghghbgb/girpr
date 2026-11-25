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

A bundle is a **fixture other work depends on**. Re-capturing one is expensive
and produces a large bundle, and `girpr`'s `artifact_repair_bridge` runs against the
copies under `artifacts/`. So the risk axis for a change here is not "might it
break the build":

> **Risk = P(silently changing what a bundle says) × cost of noticing.**

Two changes with the same code-review footprint are not equally risky: one that
cannot move a byte is free, and one that moves a byte in a field `verify` does
not re-derive is close to invisible. Everything in this document is organised by
that formula rather than by "how much code does it touch".

The corollary is what to gate on: a change to the wire needs a live check, a
change to a recorded field needs a *written expectation* (§3), and a change that
only affects allocation needs nothing but `cargo nextest run`.

### The shape of the data

The magnitudes below are illustrative, drawn from a captured pair; they vary
with the pair, and the bundle's own documents are authoritative rather than these
values being constants to copy. They are here because several of the design
decisions below are only obvious against them. Re-measure before trusting them;
they are a scale reference, not a benchmark.

| quantity | scale |
|---|---|
| files and chunks in the `game` manifest | many files and chunks per tag — a description to be checked, not a resource to be loaded |
| total chunk bytes | large — far more than a capture retains |
| manifest archive | compressed, larger when decompressed |
| largest single chunk | bounds the main tool's whole-chunk buffering |
| a default capture | large on disk, with manifests the largest contributor, alongside stored chunks and request rows |
| admitted files per capture | a fixture file in `game/` with its ground-truth counterpart in `expected/`, per capture |
| assembled bytes per capture | the captured files' own sizes |

The ratio that matters: the `game` manifest describes **far more content than a
capture retains**. Everything in this tool is therefore written under
the assumption that the manifest is a *description* to be checked, not a
resource to be loaded.

---

## 2. Deliberately left alone

| | |
|---|---|
| **`biz.rs`** | all `const fn` tables and `match` over a small enum: `Biz::resolve`, `Host::hyp_base`/`sophon_base`, `HypEndpoint::path`. **Zero per-call table construction, zero repeated lookups.** The one note is `Endpoints::owned`'s `Box::leak` — see below. |
| **`exit.rs`** | `const fn` tables over the exit-code enum, a `const ALL` pinned by a test, and a `From<ExitCode> for i32`. The cleanest file in the crate. |
| **`select/budget.rs`** | **zero magic numbers.** `saturating_add`, a charge that saturates, a rejected candidate is never charged, and the tie-break is documented as the caller's. Use it as the model for the rest of `select`. |
| **`report.rs` serialization** | there is **no `serde_json::Value` construction** — `Record`/`Val` implement `serde::Serialize` and `to_json_line` is `serde_json::to_string(self)`; the `Serialize` impl emits an ordered map with `event` first to keep field order (`preserve_order` would be a workspace-wide feature-unification hazard). `serde_json` owns escaping, so backslashes and control characters keep their prior bytes. |
| **`assemble/ranges.rs`** | already a **single forward pass**; there is no sort-then-merge to improve. |
| **`assemble/assembly.rs` byte movement** | **no per-byte loops anywhere.** `copy_from_slice` per chunk and a zeroed `vec![0u8; size]` are already optimal. The waste is the *number of buffers*, not the copying. |
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
| `tests/bundle_diff.rs` | **captures of one binary** | a change applied **uniformly to every capture** — by construction |
| the per-document pins in `tests/capture_layout.rs` | a written expectation for the volatile documents | any **digest** in those documents, on purpose: A16 owns those |
| `tools/bundle_differ.py` | **captures from different commits** | a change that was wrong the same way at both ends |

**The rule that falls out of the second row:** a before/after comparison cannot
see uniform drift. Two captures of one binary both get the new behaviour, so
they agree and the gate is green. Closing it needs a *written expectation*, not a
third capture — which is why the volatile documents are pinned field by field
rather than only compared.

What stays unpinned, deliberately: every digest in `MANIFEST.json`,
`verify/checksums.json` and `README.md`. A16 covers those, and they move for
`api/requests.ndjson` and its cascade rows by design; pinning them would
mean pinning a timestamp.

The cross-binary instrument is described in
[`04-bundle-differ.md`](04-bundle-differ.md). Its own governing rule is the same
one: **a self-check that only ever compares one binary to itself cannot find a
field that one binary happens to share with itself.** That is a real failure mode
here, not a hypothetical — the mock's ephemeral port is recorded throughout a
bundle, so a volatile value shared by both ends can make unrelated commits look
like they changed many documents.

### Mutating to prove a gate works

A gate that has never been seen to fail is not known to work. Every new
expectation in this crate was checked by making the mutant it is supposed to
catch, and both ends were confirmed: the mutant fails the pin, and every
other suite passes with it. Some that were caught only by the new pin, with
nothing else noticing:

- a total in `MANIFEST.json` moved slightly
- an assertion's **wording** narrowed

The second is the interesting one. `A10` can read "every reused chunk's
old-file slice MD5 matches" or "every reused chunk's MD5 matches", both under the
same id. An `id`-only check passes both. That is why `verify/selfcheck.json` is
pinned by exact wording as well as by id.

### A trap worth writing down

PowerShell's `Copy-Item` **preserves `LastWriteTime`**. Restoring a mutated file
with it therefore leaves the source file *older* than the binary cargo last built
from the mutation, cargo considers the target fresh, and `cargo build --release`
reports `Finished` in a fraction of a second without compiling anything.

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

**`verify` memoises each blob's decode.** The verifier has to check every blob
*and* rebuild the files from them, which needs the same bytes for both purposes.
A memoised decode, taken *after* `check_blobs` has had the blob in hand, covers
both without touching what is asserted. The same argument does **not** extend to
`capture`, which rebuilds files repeatedly and prefers the page cache to resident
copies — so the memo is opt-out, and the opt-out's reasoning is recorded at the
method.

**The request log's mutex is not the concurrency ceiling.** A process-wide lock
around the log record does not serialise the workers. What is left under the lock
is a map lookup.

**What is left is the filesystem.** On Windows, per-blob latency grows sharply
as `--jobs` rises while throughput barely moves, total CPU stays well under
one core, and the time is inside `spawn_blocking(measure_file)` rather than in the
thread handoff. A micro-benchmark on a single small frame settles it: the zstd
decode is a small fraction of that call, and creating, renaming and reopening a
fresh file — with no decode at all — is several times the decode.

Consequences:

1. The cost is **per file, not per directory entry**. A `chunks/blobs/` directory
   growing from a small to a large number of entries leaves the per-blob cost
   flat, so "the directory gets slower" is not the explanation and a fix aimed at
   it would achieve nothing.
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
| a blob whose `stored_path` is `null` | has several causes: failures (a source that refused, a write that failed, a task that panicked mid-measure), which say which in `fetch_error`, and a `--no-chunk-blobs` row, which is null there and means "not measured". |
| `verify` reporting a blob as red under `--no-chunk-blobs` | correct. The index lists a blob with no path, which is exactly "this bundle cannot be materialized". Red, not skipped. |
| a `probe` row with `attempts: 1` on every healthy capture | correct, and not evidence that nothing was ever retried. A `getBuild?tag=` probe *is* retried when the status calls for it; the negative probes behind `error_shapes.json` never are. |
| `identical` appearing in both `game/` and `expected/` with equal bytes | that is the class. `diff -r game/ expected/` shows it as unchanged, on purpose. |
| a `--no-chunk-blobs` capture exiting `0` | it recorded rows it could not prove as **skipped** and says `result: "incomplete"`. Read `result`, not the exit code. |
| `chunks/index.json` listing blobs that no `game/` file needs | the index is the closure of `materialize_old ∪ materialize_new` over every admitted capture, including `removed`, which has a `materialize_old` side and no `expected/`. |
