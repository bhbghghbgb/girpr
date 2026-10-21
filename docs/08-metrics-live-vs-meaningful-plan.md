# Metrics: live vs meaningful — implementation plan

The counters in `girpr` do two jobs at once. A counter is **live** while the run is in flight
(so a runtime hard stop can read it mid-flight) and it is **reported** at the end (so the
`SUMMARY` line can say something about the run). When the two answers differ, the reported
number is either a lie or a number nobody can act on.

This plan makes the two explicit, for every counter that has the property, and adds the CLI
limits that the split makes possible. It is the worklist; it is **deleted when the work is
done**, and its content lives in the documents that own it (section 9, the fold).

**When.** This is a **separate piece of work from the streaming zstd decode**, and it lands
**after** it. The streaming branch's review note
(`docs/07-streaming-zst-decode-review.md`, section 3) fixes back the one metric the streaming
change broke — `SUMMARY write_bytes` — and deliberately leaves the output contract alone. This
plan starts from that stable base and changes the contract on purpose.

**Scope.** `src/report.rs` (the counters and the `SUMMARY` keys), `src/repair/scope/ledger.rs`
(the runtime ledger), `src/repair/scope/limits.rs` (the ceilings), `src/cli.rs` (the flags), and
the documents that describe them (`docs/02`, `docs/04`, the README).

---

## 1. The problem, measured

The streaming zstd decode moved the chunk write from "decode into a buffer, verify, then write"
to "decode into the file as it arrives, verify when the stream ends". The bytes now land
**before** they are validated, which splits the write counter in two: the bytes that *landed*
and the bytes that *validated*.

The effect on `SUMMARY write_bytes`, measured over the same two-file fixture and the same
install, `main` vs the streaming branch:

| origin | `main` | streaming branch |
|---|---|---|
| clean | **176** | **176** |
| one corrupt-but-well-formed chunk | **176** | **224** |
| origin failing every chunk request | 0 | 0 |

On `main` the number was a **pure function of the input state**: the corrupt chunk contributed
**0**, a transport error contributed **0**, and a retry contributed **0**. On the branch it
**grows with the number of failures**. The same install, the same API, the same files, a
different number, because the network was flaky.

That is the general defect this plan fixes, and the streaming branch is where it became
visible. It is worth being precise about why it was invisible until then: before the streaming
change, the two numbers **coincided** — a validated write was the only kind of write there was,
because the download path charged the ceiling *after* the chunk's digest passed
(`main:src/repair/file.rs`, `reserve_write` below `md5::compute`). The coincidence was an
accident of the ordering, not a design, and nothing documented it, so nothing tested it.

## 2. The inventory: which counters have the property

| counter | live | reported | has the property? |
|---|---|---|---|
| `download_bytes` | bytes in flight, plus the reservation the hard stop reads | the bytes of the files that were **actually repaired** (an abandoned attempt hands its bytes back) | **yes**, and it is already split — by a hand-back at the end of a failed attempt |
| `write_bytes` | bytes that landed in the install | *should be* the bytes written from validated sources | **yes**, and it is the one the streaming branch broke |
| `kept_bytes` | none | the bytes that were already correct and were not written | **yes, in the wrong place**: it is *derived* from the per-attempt `stats.bytes_written` (`file.rs:213-215`), so when an attempt fails after writing part of a file, the earlier attempt's writes are not in the total and `kept_bytes` **over-reports** |
| `deleted_extra_bytes` | none | the bytes the cleanup deleted | no — a delete either happened or did not |
| `ram_bytes` | a high-water mark | the same | no — a peak is inherently live; there is no "meaningful peak" |
| `files_*` | none | counts | no |
| `est_download_bytes`, `est_write_bytes`, `scan_bytes` (the `PLAN` line) | none | pre-flight estimates | **adjacent, not the same**: this is *estimate vs actual*, not *live vs meaningful*. `--scan` makes the estimate exact; the pair is worth a look but it is not this plan's subject |
| `limited`, `scoped`, `plan_files`, `plan_download_bytes`, `config_bumped`, `local_manifest`, `stopped` | none | flags | no |

So the family is **three counters** — `download_bytes`, `write_bytes`, `kept_bytes` — and the
three are the same family: each of them answers "how much did the repair *do*", and each of them
is currently derived from a number that moves with the transport.

`kept_bytes` is the one nobody has looked at, and it is the one with a real bug in it. Its
definition in `src/report.rs:288-291` is "bytes that were already correct where they belonged
and were **not** written", and it is computed as `file.size - stats.bytes_written`. But
`stats` is **per attempt** and is dropped when the attempt fails, so in the in-place path an
attempt that wrote chunks 1..k and then failed on chunk k+1 produces a second attempt whose
`stats.bytes_written` counts only chunk k+1 — the file's `kept_bytes` then claims that k
chunks' worth of bytes were "kept", when the run actually rewrote them.

## 3. The design

### 3.1 One mechanism, not three ad-hoc counters

The split is the same shape for all three counters, so it should be **one type**, not three
special cases. In `src/repair/scope/ledger.rs`:

```rust
/// A counter with two numbers.
///
/// - **live** is charged *before* the resource is consumed. It is monotone, it
///   is what a runtime hard stop reads, and it is what a budget is about:
///   the bytes are spent whether they turn out to be good or not.
/// - **settled** is charged *after* the resource is validated, and it is what
///   the SUMMARY reports: the bytes the repair was *allowed* to spend.
///
/// The two are equal in the happy path and differ exactly when something
/// failed. `download_bytes` has always been this pair with the settle done
/// by handing the bytes back; `write_bytes` becomes it here.
pub struct Meter {
    live: AtomicU64,
    settled: AtomicU64,
}
```

with the operations the three call sites need:

| operation | when |
|---|---|
| `charge_live(n) -> bool` | before the resource is consumed; `false` means the ceiling refused |
| `settle(n)` | after the resource validated; moves `n` from "live only" into "settled" |
| `refund(n)` | the resource failed and its bytes are not counted; subtracts from `live` |
| `live()`, `settled()` | the readers: the hard stop reads `live()`, the SUMMARY reads `settled()` |

**The rule that makes it work**, and that the streaming branch breaks and this plan restores:

> A byte is **settled** when it has been validated. It is **live** from the moment it is
> committed until it is settled or refunded.

### 3.2 The three migrations

| counter | live | settled | the migration |
|---|---|---|---|
| `download_bytes` | the reservation, as today | the bytes of the files that were actually repaired | **already this**: the hand-back at `file.rs:1043-1044` and `:487-489` is a `refund` of the abandoned attempt's bytes. Move it onto the type; the reported number does not change |
| `write_bytes` | the bytes that landed | the bytes written from validated sources | the streaming branch's review fix already introduces the second number; this plan makes it a `Meter` and gives it a name |
| `kept_bytes` | none | the bytes of the install the repair did not have to touch | **derived from the settled write meter, not from the per-attempt stats**: `kept = file.size - (the file's settled writes)`, where the file's settled writes are accumulated across attempts, not taken from the last attempt's `ChunkStats` |

### 3.3 The reported keys

The house rule is that a key is emitted **only when the corresponding limit is active**, "which
is what keeps the machine contract non-breaking: an unflagged run's output is byte-identical to
a run that never passes the flag" (`docs/04` section 6). The new keys follow it.

| key | when | what |
|---|---|---|
| `download_bytes=<N>` | `--max-download-bytes` active | **unchanged**: the bytes of the files that were actually repaired |
| `write_bytes=<N>` | `--max-write-bytes` active | **unchanged** (after the review fix): the bytes written from validated sources |
| `download_bytes_spent=<N>` | `--max-download-bytes` active | **new**: the bytes that crossed the wire, including the abandoned attempts' and the in-flight ones |
| `write_bytes_spent=<N>` | `--max-write-bytes` active | **new**: the bytes that landed in the install, including the bytes of an attempt that later failed its digest |
| `kept_bytes=<N>` | unchanged (always) | **unchanged in name, fixed in value**: it becomes the complement of the settled writes |

`spent` is the word for the live figure: it says "what the run actually spent", which includes
the waste. The alternatives are `_live`, `_raw` and `_total`; `spent` is the one that reads as a
cost rather than as an implementation detail.

**The `stopped=` vocabulary has to grow**, and this is the sharpest edge of the contract change.
Today it is `stopped=bytes|write|delete` (`docs/04` section 6), one value per ceiling. With two
more ceilings it becomes `stopped=bytes|write|delete|verified_bytes|verified_write`, which is
where the flat vocabulary starts to hurt.

**The better answer is to make the value the flag name**: `stopped=--max-write-bytes`. It is
self-describing, it scales to any number of ceilings, and it is the shape a script wants
(`case "$stopped" in --max-write-bytes) ...`). It is a **breaking change** to the `SUMMARY`
line, so it needs its own note in `docs/04` section 5 and a line in the README's contract
section. It is listed in section 7 as an open decision rather than a settled one, because it
touches every consumer of the `SUMMARY` line — including `gircap` and the artifact bridge.

### 3.4 The CLI limits

This is the part the split makes possible, and it is the part with the user-visible value.

**The problem today.** `--max-download-bytes N` bounds the **live** download bytes, and
`--max-write-bytes N` bounds the **live** write bytes. Both are budgets on *traffic* and
*disk*. Neither bounds **the repair**: how much of the install the run actually had to rewrite.
So a run against a flaky origin gets stopped early — the budget was eaten by retries that
produced nothing — while a run that legitimately rewrote 40 GB of the install is not bounded by
anything except the disk.

**The new limits**, one per family, both runtime hard stops (neither can be a pre-flight
refusal: the live/settled split is only known at runtime):

| flag | bounds | shape |
|---|---|---|
| `--max-verified-download-bytes <N>` | the **settled** download bytes: the bytes of the files that were actually repaired | runtime hard stop, exit `5`, `stopped=…` |
| `--max-verified-write-bytes <N>` | the **settled** write bytes: the bytes the repair actually wrote into the install | runtime hard stop, exit `5`, `stopped=…` |

`verified` is the house's word for "passed its digest" (`verified local slice`, the
`uncompressed_md5` gate), and it is what the two flags are named after. The **counters** use the
shorter names the review note's section 3.3 fixes: the settled figure is `write_bytes` /
`download_bytes`, and the live one is `*_spent`. The two documents must not drift apart on the
words.

**Why two flags and not one.** A user who wants "do not rewrite more than N bytes of my
install" wants the write one; a user who wants "do not spend more than N bytes of network"
already has `--max-download-bytes`, and a user who wants "do not fetch more than N bytes that
actually land in files" wants the download one. Collapsing them into a single `--max-repair-bytes`
would have to pick one of the two quantities, and the other would stay unbounded.

**They compose with the existing ceilings, and the order matters.** The live ceilings are
checked first (they bound the disk and the network); the verified ceilings are checked when a
resource is settled. A run with both `--max-write-bytes` and `--max-verified-write-bytes` stops
on whichever is reached first, and `stopped=` says which.

## 4. The worklist

Five commits, each verified the way the house verifies (`cargo test -p girpr`,
`cargo clippy -p girpr --all-targets`, and the **artifact bridge** against the bundle), and each
carrying the message below.

### M1 · `girpr: the ledger's meter — a counter with a live and a settled number`

**Changes.** `src/repair/scope/ledger.rs`: the `Meter` type and its four operations. No
behaviour change: `download_bytes` moves onto it, with the existing hand-back expressed as a
`refund` and the existing reservation as a `charge_live`. The reported `download_bytes` is
unchanged, bit for bit.

**Verifies.** The whole suite; `download_bytes_counts_only_promoted_files_even_after_a_retry`
(the hand-back is the interesting part) and `max_download_bytes_refuses_on_the_estimate_and_passes_at_the_boundary`.

**Does not verify.** The write path, which is untouched in this commit.

### M2 · `girpr: write_bytes — the settled figure is what the SUMMARY reports`

**Changes.** `src/repair/file.rs`: the live write counter becomes the meter's live number and
the validated writes become its settled number (the review note's fix, on the type). The
`write_bytes=` key reads the settled number. **The key's name and its position in the line do
not change**, and no new key is emitted — the *reported* split is M3.

**Verifies.** The whole suite, the new determinism test from the review note (a clean origin
and a corrupt origin give the same `write_bytes`), and `max_write_bytes_stops_mid_flight_and_leaves_the_work_for_the_next_run`
(the ceiling still does not overshoot).

**Does not verify.** The `spent` key, which is M3.

### M3 · `girpr: the spent figures — what the run actually spent`

**Changes.** `src/report.rs`: the two new keys, `download_bytes_spent=` and
`write_bytes_spent=`, emitted under the same conditions as their settled counterparts. The
`stopped=` vocabulary grows.

**Verifies.** The suite; the `SUMMARY` line's shape is pinned by `src/report.rs`'s own tests
and by `tests/cli_contract.rs`.

**Does not verify.** The new flags — M4.

### M4 · `girpr: the verified ceilings — two more limits, on the settled figures`

**Changes.** `src/cli.rs` and `src/repair/scope/limits.rs`: `--max-verified-download-bytes` and
`--max-verified-write-bytes`, both runtime hard stops, both counting towards `limited=1`, both
answering with exit `5` and a `stopped=` value that says which one bound.

**Verifies.** The suite, plus a new offline test per flag: the run stops when the settled figure
crosses the line, and the `SUMMARY` line carries the right `stopped=` value.

**Does not verify.** The `kept_bytes` fix, which is M5.

### M5 · `girpr: kept_bytes — the complement of the settled writes, not of the last attempt`

**Changes.** `src/repair/file.rs` and `src/report.rs`: `kept_bytes` is computed from the
settled write meter instead of from the per-attempt `stats.bytes_written`. This fixes the
over-reporting described in section 2.

**Verifies.** A new offline test: a run whose first attempt fails after writing part of a file
must report the same `kept_bytes` as a run that wrote the file in one go.

**Does not verify.** The docs, which is the fold.

### M6 · `girpr: fold the metrics plan into the docs that describe the tool`

The fold — section 9.

## 5. The risks

| risk | how it is handled | where |
|---|---|---|
| the reported `write_bytes` and the ceiling disagree (the stop fired at N, the key says N-48) | the log line at the stop reads the **live** number and says so; the key reads the settled number | M2, and the `stopped=` key tells the reader which ceiling bound |
| a consumer parses `stopped=` and breaks | the vocabulary change is the sharpest edge; it is called out as an open decision in section 7 and gets its own note in `docs/04` section 5 | M3 |
| `kept_bytes` changes value | it is a bug fix, but it is a **reported number moving**; it needs the same treatment as any other contract change | M5 |
| the live ceiling overshoots because the settle is late | the live number is charged **before** the resource is consumed, exactly as today; the settle only moves bytes between the two numbers | M1, M2 |
| a `--max-verified-*` flag is set and never fires | the settled figures are only known at runtime, so the flag can only be a hard stop; a run that never settles that many bytes is simply not limited by it. `docs/04` must say this, the way `--max-write-bytes` does | M4 |
| the two flags are set and the interaction is unclear | the live ceilings are checked first; the verified ones are checked at settle time; `stopped=` says which bound | M4 |
| the bridge or `gircap` reads a `SUMMARY` key that moved | the bridge reads `download_bytes` and the byte totals, which do not change; the `stopped=` vocabulary is the only contract change, and it is the open decision | M3, section 7 |

## 6. What this plan does not settle

- **The `stopped=` vocabulary.** Section 3.3 proposes the flag name and flags the cost. The
  decision needs the consumers' input (`gircap`, the artifact bridge, anything that greps the
  `SUMMARY` line), so it is an open question rather than a settled design.
- **The estimate-vs-actual family.** `est_download_bytes` / `est_write_bytes` / `scan_bytes` are
  a different kind of pair (a prediction and an outcome) and are left alone. `--scan` already
  makes the estimate exact; the question of what the *non*-`--scan` estimate should promise is
  its own.
- **Whether the `PLAN` line should carry the settled figures.** It cannot: the plan runs before
  the repair. It carries estimates, and that is right.
- **`ram_bytes`.** A peak is inherently live. There is no meaningful peak to report beside it.
- **The plan's own measurement.** The numbers in section 1 come from the review note's
  measurement over the synthetic fixture. The bundle gives the real ones, and the artifact
  bridge is the place to take them.

## 7. The open decision, spelled out

**`stopped=` — flat vocabulary or flag name?**

| | flat (`stopped=bytes|write|delete|verified_bytes|verified_write`) | flag name (`stopped=--max-write-bytes`) |
|---|---|---|
| change to today's consumers | none for the three existing values; two new ones to learn | **every existing value changes** |
| scales with the number of ceilings | no | yes |
| what a script wants | a `case` over five values | a `case` over flag names, which is what a script already has to write |
| the house's taste | `docs/04` section 6 says the value exists "so the three cannot share a value" — it says *which* ceiling bound | the same, more precisely |

The recommendation is the flag name, taken with the cost: it is a breaking change to one key, it
has to be announced in `docs/04` section 5 and in the README, and it touches `gircap`. The
alternative is to keep the flat vocabulary and accept that it needs a new value per ceiling.

## 8. The house pattern this follows

The house has folded a plan into the documents three times in this repo — `be5fc4b` ("girpr: fold
the low-disk plan into the docs that describe the tool"), `516d710` ("gircap: fold this plan into
the documents, and delete it") and `818f8ee` ("docs: merge the option/plan pairs into two specs,
and drop the archaeology") — and the shape is always the same:

1. the plan document lives **while the work is in flight**;
2. at the end it is **deleted**, and its content is distributed to the documents that own it;
3. the closing commit message carries an **"it was in | it is now"** table;
4. the fold **changes no code**.

The same shape applies here, and the fold is section 9.

## 9. The fold

The last commit is the fold, and it is not optional: a plan document that outlives its work is
archaeology. The house pattern is `be5fc4b` / `516d710` / `818f8ee` (section 8), and it has two
rules this fold follows:

1. **the account of how the thing got here is `git log`'s job.** The measurements and the
   rejected alternatives are the argument for the change, not a description of the tool; what
   survives is the *behaviour*.
2. **the fold changes no code.** The source comments that pointed into the deleted file are
   rewritten in the commits that make the change.

The table the closing commit message carries:

| it was in | it is now |
|---|---|
| the live/settled distinction and why a counter has two numbers | `docs/04` section 6 (the `SUMMARY` additions) and the `Meter` type's doc comment in `src/repair/scope/ledger.rs` |
| the three counters and what each of them now means | `docs/04` section 6's key table, and `src/report.rs`'s per-counter doc comments |
| the two new limits and what they bound | `docs/04` section 3 (a new `B5` row beside `--max-download-bytes` and `--max-write-bytes`), the README's flag table, and `src/cli.rs`'s flag help |
| the `stopped=` decision and its cost | `docs/04` section 5 (exit codes) and section 6, and the README's contract section |
| `kept_bytes`'s definition and the bug that was fixed in it | `src/report.rs`'s counter doc comment and `docs/02`'s disk model, which is where "the bytes the repair did not have to touch" is described |
| the measurement that started it | `docs/04` section 6's note on what the byte counts are worth, in one sentence — the number itself is `git log`'s job |
| the plan itself | deleted |

**Verifies.** The whole suite, the bridge, and a read-through: every sentence in the shipped
docs that describes a counter or a ceiling is true of the code as it stands.

**Does not verify.** Anything about the code — it changes no code.
