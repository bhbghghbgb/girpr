# ADVERSARIAL REVIEW NOTE — eleventh pass (O1–O4)

**Baseline:** `bcb4c38` (`docs(review): fold the tenth pass into the consolidated
review`), clean working tree; reviewed 2026-10-29 on `fix-gaps/gircap`.
**Scope:** `tools/capture/src/**`, `tools/capture/tests/**`,
`tools/capture/docs/00-spec.md`, `tools/capture/docs/01-artifact-schema.md`;
source-verified by reads and `git grep`, plus the four commands in
[Verification](#verification).
**Result:** four open findings. No source, test, doc, Cargo or existing review
file was changed; this note is the only file added.

| ID | Finding | Severity | Status |
|---|---|---|---|
| O1 | A success body allocates `Content-Length` bytes **before** the response ceiling rejects the declared length. | high | open |
| O2 | A17 accepts an incomplete or invalid matrix, so `verify` certifies a bundle missing a required class. | medium | open |
| O3 | `--require-class` prints the Rust `Debug` spelling of `MatrixStatus`, not the artifact's spelling; its doc example names a variant that does not exist. | low | open |
| O4 | The manifest archive is zstd-decoded without a bound before its declared uncompressed size is checked. | medium | open |

---

## O1 — the untrusted `Content-Length` sizes the buffer before the ceiling checks it

**Evidence.** The in-memory success path creates the body buffer from the
response header and *then* calls the function that applies the ceiling:

- `src/http/fetcher.rs:909-911` —
  `Vec::with_capacity(resp.content_length().unwrap_or(0) as usize)`, immediately
  before `drain_capped(...)`.
- `src/http/fetcher.rs:149-152` — the declared-length check lives inside
  `drain_capped`, so it runs only after that allocation.
- `src/http/transport.rs:151-154` promises the header is checked "before a byte
  is read (so a server claiming 10 GiB is refused immediately)"; the same promise
  is repeated at `src/http/fetcher.rs:123-126`. The promise is false for the
  in-memory path.

**Scenario.** A loopback origin answers `200 OK` with
`Content-Length: 10737418240` (10 GiB). Reqwest accepts the header and reports
`content_length() == Some(10737418240)`; `Vec::with_capacity` attempts the
allocation and the process aborts with

```
memory allocation of 10737418240 bytes failed
```

before `drain_capped` can return the typed `too_big`/`Shape` error. A declared
length at or above `isize::MAX` reaches `Vec::with_capacity` as a capacity
overflow panic instead; Hyper accepts values up to `u64::MAX - 1` (2^63 was
accepted in the probe; only `u64::MAX` was refused as a parse error). This was
reproduced with the workspace's reqwest version and a raw TCP listener outside
the repository. The docs' own threat model is a "broken or hostile origin"
(`src/http/transport.rs:132-135`), so this is not an academic header: one line
defeats the ceiling.

**Why it violates the standard.** The ceiling is supposed to be enforced before
the body is read *and* before the tool commits memory to it. Instead an
untrusted decimal becomes an allocation size, so the outcome is abort/panic
rather than the documented typed refusal. `Destination::File` is not affected:
it writes through `drain_capped` and never sizes a buffer from the header.

**Fix.** Check `declared_over_ceiling(&resp, cap)` before constructing the
buffer (or cap the capacity at `cap` and let `drain_capped` reject the response),
then create `Vec::new()`/a bounded buffer. Keep `drain_capped`'s second check as
defence in depth. Add a loopback test that serves a `2xx` with a 10 GiB
`Content-Length` and asserts a typed `Shape` error, no abort.

---

## O2 — A17 checks row states, not matrix completeness or class validity

**Evidence.**

- `src/emit/index.rs:1299-1326` (`Manifest::a17`) iterates the rows, rejects a
  status outside `captured | covered_by | over_budget | unavailable`, and
  rejects a duplicate class. It never compares the rows with `Class::ALL`, never
  checks the row count, and never rejects an unknown class string.
- `src/verify/mod.rs:603` makes that function the whole of `verify`'s A17
  re-derivation; `Bundle::open` (`src/verify/mod.rs:172`) deserialises
  `MANIFEST.json → matrix` without any completeness check.
- The contract says the opposite: `docs/00-spec.md:624` — "each required matrix
  class is `captured` | `covered_by` | `over_budget` | `unavailable` — never
  absent"; `docs/00-spec.md:682` — "Every class has a row"; and
  `docs/01-artifact-schema.md:149-153` — "A class is never simply absent
  (assertion A17)".

**Scenario.** Take a valid capture and delete the `added` row from
`MANIFEST.json → matrix`, or replace the matrix with a single
`{"class":"bogus","status":"captured"}` row. `Manifest::a17` returns:

```
five of six:  (true, "5 classes, each present and in an acceptable state")
one unknown:  (true, "1 classes, each present and in an acceptable state")
```

`verify` therefore records A17 as a pass, and `check_agreement` stays green
because the capture's recorded A17 also passed. A bundle whose matrix omits a
required class (or invents one) verifies clean. Reproduced through the public
`Manifest::a17` with a minimal deserialised manifest; `Bundle::open` adds no
check that could catch it.

**Why it violates the standard.** A17 is the assertion that makes "the matrix is
complete" checkable after the fact. A green A17 whose own detail says "5 classes"
is a claim that cannot be trusted, and `verify` is explicitly the command for a
bundle someone else may have produced.

**Fix.** In `Manifest::a17`, compare the set of `row.class` values with
`Class::ALL.map(Class::id)`: reject a missing class, an unknown class and a
duplicate, and name the offending class in the detail. Add tests for a five-row
matrix, an unknown class and a duplicate. The capture-side construction is
already exhaustive (`src/select/mod.rs:595` loops `Class::REQUIRED_ORDER`), so no
legitimate bundle needs an exception.

---

## O3 — the requirement failure text uses `Debug`, not the schema spelling

**Evidence.** `src/select/require.rs:49-58` formats the row with
`format!("{:?}", row.status)`. `MatrixStatus` derives `Debug`
(`src/select/mod.rs:108`) and provides the wire spelling through
`MatrixStatus::id` (`src/select/mod.rs:131-138`: `over_budget`, etc.). The
function's own doc example at `src/select/require.rs:48` says
`identical: NotFundated (over budget)`; `NotFundated` is not a variant.

**Scenario.** A run with `--require-class reuse_moved` whose row is over budget
prints:

```
--require-class not met: reuse_moved: OverBudget (the cheapest candidate cost 9000000 B)
```

An operator grepping the log for the artifact/schema spelling `over_budget`
does not find it, and the doc points at a type that does not exist. Reproduced
with the public `require::check`.

**Why it violates the standard.** The message is operator-facing and should use
the same vocabulary as `MANIFEST.json`, `docs/00-spec.md` and the schema. This is
text only: the exit code remains `4`, and `unmet` already selects the right rows.

**Fix.** Use `row.status.id()` (or give `MatrixStatus` a `Display` impl that
delegates to `id`) and correct the doc example to `OverBudget`/`over_budget`.

---

## O4 — manifest zstd output is unbounded; the declared size is checked afterwards

**Evidence.**

- `src/manifest/mod.rs:407-408` calls `hash::zstd_decompress(&fetched.body, ...)`
  as soon as the body has the zstd magic.
- `src/hash/mod.rs:333-335` is `zstd::decode_all`, which materialises the frame's
  entire content.
- `src/manifest/mod.rs:427-437` compares `decompressed.len()` with
  `declared.uncompressed_size` only after that allocation, and the checksum
  comparison follows at `:439-444`.

The compressed body is bounded by `--max-response-bytes` (`drain_capped`), but
the decoded buffer is not. The transport docs' "the ceiling is the only bound on
the path between a socket and this tool's buffers"
(`src/http/transport.rs:147-154`) is therefore not true for this path.

**Scenario.** A valid zstd frame of repeated bytes compresses enormously: a
probe with `zstd::encode_all` produced an **8 209-byte** frame that
`hash::zstd_decompress` expanded to **268 435 456 bytes** (256 MiB). A 64 MiB
compressed body under the default ceiling can expand to many GiB; the first
allocation failure aborts the process before the declared-size check, the
checksum check, or any typed `ApiError`. The same body passes `is_zstd` and the
declared `compressed_size` check.

The same helper is reachable from a crafted bundle:
`src/assemble/mod.rs:627-631` calls `zstd_decompress_from` with the index's
`uncompressed_size` only as a capacity hint; `src/hash/mod.rs:362-366` then
`read_to_end`s without a limit, so a small declared size plus a bomb blob can
abort `verify`/`materialize` too. The chunk *store* measurement is safe because
`zstd_measure_from` streams into a hasher and never materialises the output.

**Why it violates the standard.** Untrusted input controls a buffer size, and the
failure is an allocation abort rather than the fail-fast typed error the rest of
the fetch path provides. The declared size is available before the decode, but
even a lying declaration must not be able to remove the bound.

**Fix.** Decode through a limited reader (for example
`Decoder::new(bytes)` wrapped in `Read::take(limit)`) in both
`zstd_decompress` and `zstd_decompress_from`, returning a typed error when the
output exceeds the limit; derive the limit from the declared uncompressed size
plus a small allowance, and/or a configured ceiling. Add a test with a small
frame expanding past the limit and assert the typed refusal, and a test that a
declared `uncompressed_size` smaller than the frame is refused rather than
materialised.

---

## Checked and found sound

- **The tenth-pass N1 fix holds.** `tasks::join_blocking` preserves a panic
  payload, names a cancellation, and both direct joins use it; the measurement
  arm calls `discard_published` on both failure arms
  (`src/blobs/mod.rs` measurement path, `src/run/mod.rs` request-log path).
- **O1 is confined to the in-memory success path.** `Destination::File` and
  `Fetcher::observe` do not size a buffer from the header (`observe` uses
  `Vec::new` at `src/http/fetcher.rs:577`), and the error-body path counts with
  `drain_error_body`.
- **A17 is green on a complete matrix.** `Manifest::a17` accepts all four states
  and rejects duplicates; the gap is completeness/validity, not the state set.
  Capture-side `Selection` is exhaustive by construction.
- **The requirement gate still fails closed.** `unmet` excludes
  `over_budget`/`unavailable`, `check` returns `ExitCode::RequirementUnmet`, and
  a missing row is deliberately silent for the flight path; O3 is only the text.
- **Build, tests, clippy and fmt are clean on Windows** (see below); no test is
  ignored.

## Considered and not reported

- **Windows-invalid path characters (`* ? " < > |`).** `paths::relative_path_problem`
  refuses separators, absolute paths, `:`, traversal, control characters,
  folding names and DOS devices, but not these five. A hostile manifest can name
  `game/a|b`, which fails at the fixture write on Windows after the capture has
  paid for its downloads. This is a hardening candidate, not a contradiction:
  the documented rule (F6/G1) lists the covered set, the shape does not leave the
  bundle, and a legitimate Windows game manifest cannot contain the character.
  Reported here only so the judgement is visible.
- **`Fetcher::observe` does not push a row for a transport failure.** `attempt`
  records the same failure (`src/http/fetcher.rs:804-825`), but `observe`
  returns before `log.push` (`:543-555`). A probe transport failure is fatal and
  the artifact is not written, so there is no bundle in which the missing row
  could be read; the `reserve`/`push` split is deliberately non-dense. Not a
  defect in the produced artifact.
- **`verify/selfcheck.json` completeness.** `check_agreement` iterates the rows
  the recording carries, so a deleted recorded row cannot hide a *current*
  defect: `verify` re-derives its own A6/A7/A8/A11/A13/A16/A17/A18/A20 rows. The
  one-directional cross-check is the documented design; the missing-row question
  belongs to O2, where the document is itself the claim being checked.
- **`Store::fill`'s generic panic row.** The payload is logged through
  `tasks::join_failure`, while the persisted row keeps the stable generic text.
  This is the accepted F4 trade-off in `OPEN_FOR_MAINTAINERS.md`; not re-reported.

## Verification

| Command | Result |
|---|---|
| `cargo build -p gircap` | finished, no warnings (48.51s clean; rerun 0.46s cached) |
| `cargo test -p gircap` | **701 passed, 0 failed, 0 ignored** (557 lib + 144 integration/doc) |
| `cargo clippy -p gircap --all-targets` | finished, no diagnostics (17.30s) |
| `cargo fmt -p gircap --check` | exit 0, no output |

Integration totals: 3 + 8 + 32 + 2 + 30 + 10 + 29 + 15 + 15 + 0 doc-tests. The
build, test, clippy and fmt runs are all native Windows; no `#[cfg(unix)]` test
was skipped, and the `#[cfg(windows)]` tests ran.

## Not verified

- O1 and O4 were reproduced with loopback/standalone probes **outside** the
  repository (reqwest + a raw `TcpListener`; `hash::zstd_decompress` on a crafted
  frame), not through a full `gircap capture`.
- O2 was reproduced by calling the public `Manifest::a17` on a minimal
  deserialised manifest, not by running `gircap verify` on an edited bundle.
  `Bundle::open` has no matrix validation, so the two paths reach the same
  function.
- No live CDN or real game manifest was sampled this pass; the existing
  `OPEN_FOR_MAINTAINERS.md` live-input unknowns still stand.
