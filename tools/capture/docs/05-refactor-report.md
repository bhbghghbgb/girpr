# Refactoring and optimisation report

Scope: `tools/capture` (`gircap`), all of `src/` plus the integration tests that
pin its output. Read against the constraints in
[`02-plan.md` §6](02-plan.md#6-what-is-not-on-the-table) and the "looked at and
deliberately left alone" table in
[`03-design-notes.md` §2](03-design-notes.md#2-looked-at-and-deliberately-left-alone).
No files were modified; this document is the only artifact.

## How to read this

Two different risks are named for every item, because they point in opposite
directions and conflating them is how a good cleanup gets rejected:

- **Priority** — the risk of *leaving it alone*: impact × likelihood. This is
  what the document is ordered by, highest first.
- **Change risk** — the risk of *fixing it*: the chance the change moves a byte
  in a bundle, breaks a gate, or is wrong in a way the gates cannot see. This is
  the column that decides whether a change needs a re-capture, a pin update, or
  nothing at all.

The gates that matter are the four in
[`03-design-notes.md` §3](03-design-notes.md#3-the-gates-and-what-each-one-cannot-see).
Almost everything below is byte-neutral by construction: it moves code, not
field values. The one change visible outside the crate is `F8` (a clap error
message replaces a hand-written one); `F1` changes no bundle byte, because the
round-trip path never reaches the panic.

Nothing here proposes a `SCHEMA_VERSION` change, a document field change, an
A-id renumbering, a `girpr` dependency, a crate-wide `pedantic` switch, a
release-profile change, or a `rustfmt` sweep.

## Findings at a glance

| ID | Area | Finding | Priority | Change risk | Effort |
|---|---|---|---|---|---|
| F1 | `encoding` | `percent_decode` panics on a `%` followed by a 3- or 4-byte character | high (real panic in a public helper) | none — not on the capture path | 20 min |
| F2 | `http::fetcher` | `attempt` carries two byte-identical `fail` closures | medium (one edit site for a record shape) | none | 20 min |
| F3 | `blobs` | `Store::fill` re-implements `Fetcher::map_concurrently`'s join loop | medium (two concurrency policies; one can drift) | low — behaviour-preserving if the panic arm is preserved | half day |
| F4 | `report` | `emit` panics on a poisoned stdout lock, unlike every other lock in the crate | medium (panic where the crate's policy is recovery) | none | 5 min |
| F5 | `run` | `verify` and `materialize` duplicate the verdict/exit-code tail | medium (exit-code contract is load-bearing; two copies can disagree) | low — tests pin both | 1 h |
| F6 | `blobs::props` | four `BlobProps` constructors repeat a 14-field literal | medium (a new field is four edits) | low–medium — field semantics differ per constructor | 2 h |
| F7 | `select` | `notes` / `coverage` are parallel vectors zipped in phase 3, and four `MatrixRow` literals repeat the same four `None`s | low–medium | low | 1 h |
| F8 | `cli` | `--output` is a `String` parsed after clap, unlike `--log-level` | low | low — error text and exit code are unchanged; only *when* it fails moves | 30 min |
| F9 | `cli` (tests) | the non-Unicode flag table is hand-maintained and can silently miss a new flag | low (test coverage gap) | none | 1 h |
| F10 | `emit::index` | A17's doc comment states "Every state is acceptable" twice | low (reader confusion in the contract doc) | none | 2 min |
| F11 | `manifest::dump` (tests) | one doc line duplicated | cosmetic | none | 1 min |
| F12 | `rejections` | `snapshot` and `document` carry the same sort closure twice | low | none | 15 min |
| F13 | `blobs`, `http`, `emit` | avoidable clones of `Want`, of the whole request log, and of the file list | low (theoretical, not measured) | none | 2 h |
| F14 | `report` | `Val::from(u64)` narrows with `as i64`; the comment's justification is wrong | low (theoretical wrap) | none | 15 min |
| F15 | tests | three integration-test copies of `percent_decode` | cosmetic | none | 20 min |

## 1. Correctness and robustness

### F1 — `percent_decode` can panic on valid UTF-8 (priority: high)

`src/encoding.rs:46` slices a `str` by byte offsets computed from the *byte*
scan:

```rust
if bytes[i] == b'%' && i + 2 < bytes.len() {
    let hex = &s[i + 1..i + 3];   // panics if i + 3 is not a char boundary
```

`percent_decode("%中")` has bytes `25 E4 B8 AD`; `i + 3 == 3` falls inside the
three-byte character, and `&s[1..3]` panics with "byte index 3 is not a char
boundary". Any three- or four-byte character immediately after a `%` does it.

This is **not reachable from a value the tool itself encoded**: `percent_encode`
turns `%` into `%25` first, so the round trip is safe, and that is why no test
has caught it. It *is* reachable for a consumer, which is exactly who the
function is exported for (`report::percent_decode` is `pub`, and the doc says
"so a consumer — and a test — can read a value back").

The fix is the byte-oriented decode the mock server already uses
(`tests/common/mod.rs:1404`): accumulate `Vec<u8>`, push the raw byte when the
two hex digits do not parse, and finish with `String::from_utf8_lossy`. That is
also one allocation instead of one `&str` slice + one `String` per escape.

Change risk: none. The output for every string `percent_encode` produces is
byte-identical; add `percent_decode("%中") == "%中"` and
`percent_decode("%zz中") == "%zz中"` to `src/report/tests.rs`.

### F4 — the stdout lock recovers differently from every other lock (priority: medium)

`src/report/mod.rs:755` does `let _guard = STDOUT_LOCK.lock();`. Every other
lock in the crate deliberately recovers from poisoning:
`RequestLog::lock` (`http/log.rs:33`), `PathMap::record` (`pathmap.rs:97`),
`Rejections::record` (`rejections.rs:117`). `RequestLog::lock`'s doc argues the
policy at length: a panic in one worker must not take down the run's ability to
report.

The stdout lock is held for one `writeln!`, so a panic *while holding it* is
nearly impossible — but "nearly" is the same argument that was made about the
request log before it bit. `unwrap_or_else(|e| e.into_inner())` is a one-line
change that makes the policy uniform and cannot alter output.

### F14 — `Val::from(u64)` narrows with a justification that does not hold (priority: low)

`src/report/mod.rs:142-148`:

```rust
impl From<u64> for Val {
    fn from(v: u64) -> Self {
        // Byte counts are `u64` in the API types; the wire form is JSON, where a
        // negative number cannot appear, so the narrowing is exact.
        Val::Int(v as i64)
    }
}
```

The comment says a negative number cannot *appear*, which is true of a correct
encoding and says nothing about `as i64` producing one. A `u64` above
`i64::MAX` becomes negative in the summary line. No current value can get
there (counts and byte totals are bounded by the disk), so this is a comment
problem more than a code problem. Two honest fixes:

- `Val::Int(i64::try_from(v).unwrap_or(i64::MAX))` — clamps instead of wrapping;
- or keep the cast and rewrite the comment to say "counts are far below
  `i64::MAX`; the cast is a narrowing by contract, and a value that large is a
  bug elsewhere".

The first is one line and removes the class. Do the same for `From<usize>`.

## 2. `blobs`: the store, its props, and its clones

### F3 — one ordered-join loop, written twice (priority: medium)

`Fetcher::map_concurrently` (`src/http/fetcher.rs:481-560`) and
`Store::fill` (`src/blobs/mod.rs:343-421`) implement the same thing:

1. an `out: Vec<Option<_>>` sized to the input,
2. prime a `JoinSet` with `jobs.max(1)` tasks,
3. `join_next` in a loop, writing each result to its own slot,
4. refill one slot per completion,
5. fill any hole left by a panic.

They differ in two ways, and only one is essential:

- `map_concurrently` records `task::Id -> index` so a **panic keeps its slot
  and its payload**; `fill` cannot recover the slot from a `JoinError`, so it
  logs the payload and fills the hole afterwards with a generic sentence plus
  the chunk id it can infer from the hole's position. Both end up naming the
  chunk, so this is an attribution-quality difference, not a correctness one.
- `fill` has a domain-specific `Ok((i, Ok(entry)))` arm that also harvests
  `entry.0.props.fetch_error` into the error list.

A shared `join_ordered` in `src/tasks.rs` — which already owns the
`JoinError` rule — would take `items: &[T]`, `jobs`, and a
`Fn(usize, T) -> Fut` returning `(usize, R)`, and return the per-item results
after mapping holes. `fill` then passes a closure that wraps `Store::one` and
flattens its `Result` into an index row (it already has `unfilled` for that),
and `map_concurrently` becomes a thin wrapper.

Value: one concurrency policy instead of two; `fill` gains panic-payload
attribution for free; roughly 40 lines of skeleton disappear. The existing
tests for both paths (`http::tests`, `blobs::tests`) already cover the panic
arms, which is the safety net this needs.

Change risk: low, but this is the one item where a mistake is silent — an
off-by-one in slot refilling changes which blob lands in which index row.
Land it alone, run `cargo nextest run -p gircap`, and compare one bundle with
`tools/bundle_differ.py` before/after.

### F6 — four `BlobProps` constructors, one 14-field literal (priority: medium)

`src/blobs/props.rs` builds the same struct four times:
`undecodable_from_digests` (line 420), `assemble` (463), `unmeasured` (504) and
`measurement_failed` (530); `measure` and `measure_file` both reach `assemble`,
so every row in the index comes from one of these four literals.
Each repeats `chunk_id`, `stored_path: None`, `container: "zstd"`,
`unknown_field6`, `fetch_error: None`, `partial: false`, `verified: None` (or
the same shape of `Some`) and `measurement_error: None`. The *differences* are
meaningful — `unmeasured` deliberately
leaves both compressed digests empty rather than zero, `undecodable` keeps the
declared `md5_decompressed`, `assemble` uses the measured one — and those
differences are documented at each constructor.

The DRY-preserving shape is a private skeleton plus struct update:

```rust
fn skeleton(chunk: &SophonChunk, compressed: Option<&hash::Digests>) -> BlobProps { … }

pub fn unmeasured(chunk: &SophonChunk) -> BlobProps {
    BlobProps { md5_decompressed: chunk.uncompressed_md5.clone(),
                uncompressed_size: chunk.uncompressed_size.max(0) as u64,
                ..Self::skeleton(chunk, None) }
}
```

This keeps every constructor's doc comment and every deliberate difference
visible, and turns "add a field" from four edits into one. The one thing to
watch: `assemble`'s `md5_decompressed` is *measured or empty*, while
`undecodable`'s is *declared*, so the skeleton must not pre-fill it.

Change risk: low–medium, purely because the field values are the artifact.
`blobs/tests.rs` and the A6 selfcheck rows pin them; run the suite and diff a
bundle.

### F13 — three clones that buy nothing (priority: low)

- **`wants[i].clone()`** (`blobs/mod.rs:364,404`). `Want` owns `chunk`,
  `sources`, `references`, `sets`, `conflicts` — several allocations per chunk.
  At the measured 113 610 chunks that is tens of MB of allocation churn per
  capture, purely so each task can own its input. Either take `wants: Vec<Want>`
  (the caller `fill_or_plan` owns the plan and only needs it afterwards for the
  index rows) or pass `Arc<[Want]>` and clone the `Arc`. The first is simpler;
  the second keeps `&[Want]` call sites. `wants` is still needed for
  `unfilled(&wants[i], …)` in the failure arms, so the `Arc` form is the
  smaller diff.
- **`RequestLog::records()`** (`http/log.rs:86`) clones and sorts the whole log.
  `hyp::attach_seq` (`hyp/mod.rs:434`) uses it to find six rows, and the log at
  that point holds six records — so this is fine *today*. But the API invites
  the O(n) clone from anywhere, and a future caller at 113 610 records pays
  23 MB and a sort. A `records_matching(&self, f: impl Fn(&RequestRecord) ->
  bool) -> Vec<RequestRecord>` (filter under one lock, sort only the matches)
  gives `attach_seq` the same answer and makes the cost proportional to what
  it asked for. Keep `records()` for tests.
- **`write_manifest_files`** (`emit/mod.rs:232-237`) does
  `Ok((c.entries.clone(), c.entries))`. It exists to hand the same list to
  `MANIFEST.json` and to seed `checksums.json`. Returning
  `(Vec<FileEntry>, Checksums)` and letting the caller take `entries` once is
  one fewer full copy of 2 744 rows; better, `Checksums::entries` could be
  `Arc<[FileEntry]>` and both consumers share it. Not worth a commit on its
  own; take it with F13's first bullet.

## 3. `http`: the fetcher's record paths

### F2 — two identical `fail` closures in `attempt` (priority: medium)

`src/http/fetcher.rs:1019-1094` defines the same closure twice, once per
`Destination` arm:

```rust
let fail = |err: ApiError, bytes_in: u64| {
    let record = self.record(seq, kind, url, range, status,
                             resp_headers.clone(), &final_url, redirected,
                             bytes_in, attempts, started);
    (err, record)
};
```

The arms differ only in whether they need `resp_headers` afterwards
(`Memory` clones it into the record and keeps the original for the final
record; `File` does the same). Hoist one closure above the `match` and the two
copies collapse; the borrow checker is satisfied because both arms only *call*
it. A helper method `fn fail(&self, …, err, bytes_in) -> (ApiError,
RequestRecord)` would be even clearer, but it needs the same eleven
arguments as `record`, so the closure is the better shape.

Value: one place that decides what a failed attempt records. This is the
function whose job is "no branch can log a request without recording it"
(see `record`'s doc at line 1112); two copies of the failure record are exactly
the drift risk that sentence is guarding against.

Change risk: none. No output changes.

## 4. `run` and `cli`: orchestration and the flag surface

### F5 — `verify` and `materialize` duplicate the verdict tail (priority: medium)

`run::verify` (`src/run/mod.rs:598-670`) and `run::materialize`
(`679-790`) each end with the same four decisions:

```rust
let (total, passed, failed) = report.counts();
let ok = failed == 0 && report.agrees;
… .set("checks_total", total) … .set("exit_code", if ok { Ok } else { AssertionFailed })
if let Some(h) = report.headline() {
    return Ok((summary.set("detail", h), ExitCode::AssertionFailed));
}
Ok((summary, if ok { ExitCode::Ok } else { ExitCode::AssertionFailed }))
```

The duplicated part is not the summary *fields* — those legitimately differ —
it is the **verdict**: `failed == 0 && report.agrees`, the `exit_code` value
that follows from it, and the "headline means exit 3 even when `ok`" rule.
That rule is the command's contract with a harness, and it is currently stated
twice. A small `struct Verdict { ok: bool, total: u64, passed: u64, failed:
u64, headline: Option<String> }` with `Verdict::of(&report)` and
`Verdict::exit_code()` removes the second statement without touching either
summary.

Change risk: low. `tests/cli_contract.rs` and `verify/tests.rs` pin both
commands' exit codes; `report.headline()`'s precedence is the only subtle
part, and keeping it inside `Verdict` makes it *more* likely to stay right.

### F8 — `--output` is validated later than every other flag (priority: low)

`cli::CommonArgs.output` is a `String` (`src/cli/mod.rs:112-113`) and
`resolve_common` (`363-374`) parses it into `OutputFormat`. Every other typed
flag — including `--log-level`, the first flag in the same struct — is typed by
clap and rejected at parse time. So `gircap capture --output yaml` is a
`UsageError`
from `resolve_common` rather than a clap error, and `resolve_common` exists
partly to carry that one failure.

Make `OutputFormat` a `ValueEnum` (or give the arg a `value_parser`) and the
field becomes `OutputFormat`, `resolve_common` becomes infallible, and the
`--output` typo is reported by the same machinery as `--biz hk4e_turkey`.
The exit code stays `1` in both paths (`clap_error_code` maps every non-help
kind to `Usage`), so no contract moves.

Change risk: low, and visible: the error text changes from
`unknown output format "yaml"; expected text or json` to clap's
`invalid value 'yaml' for '--output <FORMAT>' [possible values: text, json]`.
That is a *better* message, but it is a test-visible change —
`cli::tests::common_flags_resolve_to_typed_forms_or_a_usage_error` asserts on
the old one and should be rewritten to assert on the clap kind.

### F9 — the flag table in `cli` can silently under-cover (priority: low)

`every_value_flag_either_parses_or_refuses_a_non_unicode_value_but_records_its_bytes`
(`src/cli/mod.rs:601-668`) hand-lists 24 flag/command pairs. A 25th flag added
tomorrow is simply not in the table, and the test that exists to prove "every
value-taking flag either accepts or refuses a non-Unicode value, never panics"
passes without ever looking at it.

The fix is a coverage assertion, not more rows: walk `Cli::command()`'s
subcommands and arguments (`Command::get_arguments`, `Command::get_subcommands`)
and assert every argument with a `long` appears in the table's prefix list
(and that every table entry names a real flag). That turns a silent gap into a
red test at the moment the flag is added.

Change risk: none — test-only, and the existing rows stay.

## 5. `select`: the matrix's two lists

### F7 — `notes` and `coverage` are one fact in two vectors (priority: low–medium)

`select` builds `notes: Vec<MatrixRow>` and `coverage: Vec<Coverage>`
(`src/select/mod.rs:580-585`), pushes to both in lockstep in four places
(595/604, 630/639, 660/669, 671/680), and zips them in phase 3 (line 741).
The two vectors are always the same length and index-aligned by construction;
nothing enforces that, and a `continue` added between a `push` and its partner
would desynchronise the matrix silently — the matrix is the document that says
which classes a bundle answers.

Replace both with `Vec<PendingRow { row: MatrixRow, cover: Coverage }>`, push
once per class, and phase 3 consumes `PendingRow`s. The phase-1 code gets
shorter, not longer, because each branch becomes one push.

While there: the four `MatrixRow` literals repeat `capture_id: None,
covered_by: None, path: None, reason: None`; a
`MatrixRow::new(class, status, considered)` constructor plus `.with_reason(…)`
removes sixteen lines of `None`.

Separately, phase 1's inner loop (`646-657`) is
`O(per_class × pool × wants)` because each pick re-walks the whole pool and,
for each candidate, re-walks `admitted` and `wants`. With the default
`per_class = 1` and a 2 921-candidate pool this is noise, and
`03-design-notes` is right that `budget.rs` is the model to follow rather than
this. But `--per-class 50` is a supported flag, and the fix is small: keep a
`HashSet<&str>` of taken paths and one pass of per-class candidate indices
built once, rather than a fresh `find` per pick. Theoretical, so it belongs in
the same commit as F7 or not at all.

Change risk: low. `select/tests.rs` pins the matrix row-for-row.

## 6. `emit` and docs: two duplicated paragraphs

### F10 — A17's doc says the same thing twice (priority: low)

`src/emit/index.rs:1395-1406`:

```
/// **Every state is acceptable.** A17 is about the matrix being
/// *complete* — … A matrix that records it is more honest than one that fails.
///
/// **Every state is acceptable.** `over_budget` is one of those answers: a
/// candidate existed and this run's budget would not pay for it. Excluding
/// it would make a class the budget refused end the run with exit `3` …
```

The first paragraph and the first sentence of the second are the same claim.
Merge them: keep the "completeness, not acceptability" point from the first
and the "`--require-class` is the flag that turns an unmet class into a failed
run" point from the second, which is the part a future reader needs. Pure
doc change; `README.md` is not rendered from this comment.

### F11 — a duplicated doc line in a test (priority: cosmetic)

`src/manifest/dump.rs:488-489` has the line
"The one decision that shapes the tree, asserted in both directions." twice.
Delete one.

## 7. Smaller DRY items

### F12 — `Rejections` sorts the same way twice (priority: low)

`src/rejections.rs:134-139` (`snapshot`) and `154-169` (`document`) each build
the same `sort_by(|a, b| (&a.source, &a.value, &a.rule).cmp(…))` closure over a
clone of the rows. A private `fn sorted(state: &State) -> Vec<Rejection>`
called by both removes the second closure. This is a two-line function, but the
sort *is* the determinism guarantee ("two captures of one input write the same
document"), and one spelling of it is better than two.

### F15 — three integration-test copies of `percent_decode` (priority: cosmetic)

`tests/bundle_shapes.rs:92`, `tests/capture_layout.rs:85` and
`tests/common/mod.rs:1404` each define their own decoder. The first two exist
only because the helper is not imported; `gircap` is a library
(`Cargo.toml [lib] name = "gircap"`), so `gircap::report::percent_decode` is
available to integration tests. Delete the two copies and import it — which
also means F1's fix is exercised by every integration test that reads a
`SUMMARY` line.

The `tests/common` copy should **stay**: it decodes query strings from the mock
server and additionally maps `+` to a space, and the mock is deliberately
independent of the code under test (its module doc says so).

## 8. Reaffirmed: not findings

These were checked and are correctly left alone; they are listed so a future
reader does not re-open them.

| | why it stays |
|---|---|
| `hash::zstd_measure` / `zstd_measure_from` duplicated loop | documented at `hash/mod.rs:530-535`: sharing it would downgrade the slice version's error text, and `a_streaming_measurement_agrees_with_decompressing_the_same_bytes` requires byte-identical messages. Ten lines beat a caller that lies about what it knows. |
| `emit::closeout`'s two-pass A16 fixpoint | pass 2 must not share pass 1's buffer, or A16 becomes a tautology. `two_passes_converge_and_the_pass_the_comment_claims_would_change_nothing` and `a16_is_recorded_exactly_once_despite_two_passes` pin the ordering. |
| the two `paths.tsv` writers and the two budget sentences in `select::unfunded` | already documented as deliberate; each pair states one fact for two audiences. |
| `config`'s tag rule vs `paths`' path rule | `validate_tag_shape`'s doc says the rules are stated independently so relaxing one cannot silently reopen the other. Do not fold them together. |
| `RequestLog::write_ndjson_to`'s per-record lock | documented at `http/log.rs:139-150`: holding the lock across the disk write would stall fetches, and cloning 113 610 records to avoid it costs more than the locks. |
| `report`'s hand-rolled JSON | no `serde_json::Value`, no `preserve_order` feature hazard; `03-design-notes` §2. |
| `Endpoints::owned`'s `Box::leak` | once per process, stated in its doc; `Cow` would thread a lifetime through every call site. |
| `assemble/ranges.rs`, `assemble/mod.rs` byte movement | single forward pass, `copy_from_slice` per chunk, no per-byte loops. |
| `emit`'s `write_parts` / `write_newline_doc` | already the one writer for 23 call sites; the newline-as-second-write avoids a 27 MB copy. |
| `verify`'s memoised `ChunkSource` | the measured trade-off is in `assemble/source.rs:82-94`; capture deliberately does not memoise. |

## 9. Suggested sequence

Each batch is independently reviewable and independently revertable.

1. **Safe fixes, no output change** — F1, F4, F10, F11, F12, F14. One commit
   or five; none needs a re-capture. `cargo nextest run -p gircap` is the
   whole gate.
2. **Local dedup, no output change** — F2, F5, F6. Bundle diff should be
   empty (`tools/bundle_differ.py` against a capture from before and after).
3. **`cli` surface** — F8, F9. F8 changes one error message; F9 is test-only.
   Update `cli::tests` in the same commit.
4. **`select` restructure** — F7. `select/tests.rs` is the gate.
5. **Concurrency** — F3, alone, with a bundle diff. This is the only item
   where a mistake is silent, and it is worth the extra gate even though the
   suite covers the panic arms.
6. **Clone removal** — F13, only if a profile or a reading of the allocation
   counts justifies it. `03-design-notes` §4 already concludes that filesystem
   latency, not allocation, is the capture ceiling; this batch is about making
   the code honest, not about making it faster.

## 10. What this analysis did not find

- **No hot loop worth rewriting.** The measured ceiling is filesystem latency
  (`03-design-notes` §4); the request-log mutex is not a bottleneck, and
  `verify`'s decode is already memoised. The only quadratic shapes left are
  `select`'s per-pick scan (F7) and `retention`'s identity set, which is
  already a `HashSet` for exactly that reason.
- **No dead module.** Every file in `src/` is reachable from a command.
- **No schema hazard in any proposal.** F1–F15 move code and comments; none
  changes a field, a document, or an A-id. The bundle still carries everything
  `girpr`'s `artifact_bridge` test reads: `MANIFEST.json`, the API documents,
  the manifests, `chunks/index.json`, the patchmaps, the selfcheck and the
  checksums.
- **No reason to split a module.** The largest files (`emit/index.rs` 61 KB,
  `manifest/diff.rs` 54 KB, `http/fetcher.rs` 48 KB) are large because their
  tests live beside them; the production code in each is a coherent unit and
  the `mod.rs` facades are already the right boundaries.
