# ADVERSARIAL REVIEW — eighth pass (`gircap`)

**Baseline:** HEAD `bdb351f` (`test(blobs): bound the loopback mock's accept`), branch `fix-gaps/gircap`, tree clean before this note. Reviewed 2026-10-28.

**Method:** full reads of the seventh-pass files (`select`, `budget`, `config`, `paths`, `manifest`, `verify`, `blobs`, `emit`, `http`), `rg` over `tools/capture/src` and `tests`, the binding decisions in `ADVERSARIAL_REVIEW.md`, and three executed probes outside the repository. The four standard checks were run on this tree (results at the end). Both findings were validated and fixed on `fix-gaps/gircap`; the fix commits are recorded per finding and in the verification section.

The seventh pass is in `ADVERSARIAL_REVIEW.md` (K1–K2). Its decisions are respected here. Both findings are residual gaps in rules the seventh pass established: L1 is the one path component the H3 device-name rule does not cover, and L2 is a measurement total next to K1's saturating ledger.

## Verdict

| ID | Severity | Class | Location | One-line summary |
|----|----------|-------|----------|------------------|
| L1 | low | bug | `src/config/mod.rs:576` (sink `src/manifest/dump.rs:27`, `:94`) | Tag validation accepts DOS device names, so `--new-tag CON` fails late on Windows and can produce a `manifests/CON/` bundle on hosts where it succeeds. |
| L2 | low | bug | `src/select/mod.rs:805` (sibling `src/select/budget.rs:128`) | `bytes_admitted` is summed with plain `.sum()` while the budget saturates, so crafted costs panic in debug and wrap in release, making the summary disagree with `diff/matrix.json`. |

Both findings are **valid on the current source** and were **reproduced before the
fix**; both are now fixed. No part of either finding was rejected.

| ID | Reproduced as | Fix | Commit |
|----|---------------|-----|--------|
| L1 | `--old-tag "CON"` (and `--new-tag`, `--probe-tags`) accepted by the unit test at `config/mod.rs:1011`; it failed with `--old-tag "CON" must be refused` | `tag_shape_problem` now calls the shared `paths::is_windows_device_name`; that helper is `pub(crate)`. Tests cover `CON`/`con`/`NUL`/`NUL.txt`/`aux.txt`/`COM1`/`COM1.bin`/`LPT9`/`CONIN$`/`CONIN$.txt`, keep `7.1.0`, `CONSOLE`, `console`, `COM0`, `LPT10` legal, and add the API path (`main.tag = "CON"`) plus a focused test pinning the `device` reason. | `63ba129`, `f049c12` |
| L2 | `select::tests::two_maximum_costs_report_the_saturated_ledger_not_an_overflow` panicked at `core/src/iter/traits/accum.rs:206:1: attempt to add with overflow`; release run of the budget test printed `18446744073709551612` (`u64::MAX - 3`) against the ledger's `u64::MAX` | Both `Selection::bytes_admitted` (`select/mod.rs:805`) and `budget::total_admitted` (`budget.rs:122`) fold with `saturating_add`. A `select` fixture with two `u64::MAX - 1` costs and a budget unit test pin `bytes_admitted == total_admitted == budget.spent() == u64::MAX`. | `06c650b` |

---

### L1 — A tag may name a Windows DOS device

**Severity: low. Class: bug. Location:** `src/config/mod.rs:576-617`; sink
`src/manifest/dump.rs:27-29`, `:94-99`; write
`src/emit/mod.rs:318-322`; the rule tags do not share is
`src/paths.rs:77-82`, `:99-108`.

`tag_shape_problem` rejects empty, over-long, control, separator, drive-prefix,
`* ? " < > |`, and leading/trailing-dot tags:

```rust
// src/config/mod.rs:601-611
if tag.contains(['*', '?', '"', '<', '>', '|']) {
    return Some(format!(
        "{tag:?} contains a character Windows cannot use in a directory name; \
         a tag is a bare version string"
    ));
}
if tag.starts_with('.') || tag.ends_with('.') {
    return Some(format!(
        "{tag:?} starts or ends with `.`; Windows strips a trailing dot from a \
         directory name, so two tags could name one directory"
    ));
}
```

It never calls `is_windows_device_name`, the function the shared path rule uses:

```rust
// src/paths.rs:77-82
if value.split('/').any(is_windows_device_name) {
    return Some(format!(
        "{value:?} names a Windows device; such a component opens the device \
         instead of a file, so the bundle would not hold the bytes it lists"
    ));
}
```

and that function covers `CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`
and the `$` console aliases, with the extension ignored (`paths.rs:99-108`). The
path rule is tested for all of them (`paths.rs:247-269`); the tag's own bad list
does not include one:

```rust
// src/config/mod.rs:1017-1035 (abridged: every entry is a string literal)
for bad in [
    "C:", r"..\..", r"a\b", "a:b", "..", "7.1.0\u{7}",
    "7.1.0?", "7.1.0*", "7.1.0\"", "7.1.0<", "7.1.0>", "7.1.0|",
    "7.1.0.", ".7.1.0",
] {
```

**Scenario.** `gircap capture --new-tag CON` (equally `--old-tag CON`, a
`--probe-tags CON` entry, or an API-resolved `main.tag = "CON"`/`diff_tags[0] =
"CON"`, which `VersionPair::select` validates at `config/mod.rs:117-123`) passes
`validate_new_tag`/`validate_tag`. The tag later becomes a directory component
here:

```rust
// src/manifest/dump.rs:27-29
pub fn dir(tag: &str) -> String {
    format!("manifests/{tag}")
}
```

and `manifest::dump::write` re-runs the same validator (`dump.rs:94-99`) before
the first write. `write_parts` then does the path join:

```rust
// src/emit/mod.rs:318-322
let path = root.join(rel);
if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir)?;
}
```

On Windows that is `CreateDirectoryW("...\manifests\CON")`. Executed outside the
repository with the standard library call the writer uses:

```text
create_dir_all: Err(Os { code: 267, kind: NotADirectory,
                        message: "The directory name is invalid." })
file create:   Err(Os { code: 3, kind: NotFound,
                        message: "The system cannot find the path specified." })
```

So the run dies at manifest writing — after the API requests and manifest
downloads — with a generic I/O error rather than refusing the tag at argument
resolution. On a host where the directory name is accepted, the capture succeeds
and writes `manifests/CON/...`; `MANIFEST.json → files[]` records that path. A
Windows copy of that bundle cannot contain the directory at all, so A16 reports
the listed files as missing; the tool's write side accepted a name its shared
read-side path rule calls illegal.

`pair_dir` (`config/mod.rs:661-664`) is not the escaping sink here: its component
is `CON__7.1.0`, which is an ordinary name. The sink is `manifests/<tag>`.

**Why the standard fails.** The H3 decision says DOS device names are refused "on
every host" (`ADVERSARIAL_REVIEW.md:675`), and F6/G1 says a tag is a path
component on every host (`:667`). `tag_shape_problem`'s own doc says "The Windows
rules are here too" and explains that a late `create_dir_all` failure would waste
the run (`config/mod.rs:550-555`). A device name is exactly that case. The test
list at `:1017-1035` covers the Windows-reserved *characters* and the trailing-dot
fold, but not the reserved *names*, so the seventh-pass test gap is still open.

**Suggested fix.** Make `paths::is_windows_device_name` `pub(crate)` and add one
check to `tag_shape_problem` after the trailing-dot rule:

```rust
if crate::paths::is_windows_device_name(tag) {
    return Some(format!(
        "{tag:?} names a Windows device; a tag is a bare version string"
    ));
}
```

Then add `CON`, `con`, `NUL`, `aux.txt`, `COM1`, `LPT9`, `CONIN$` to the bad list
at `config/mod.rs:1017`, and keep `CONSOLE`, `console`, `COM0`, `LPT10` in the
good list. Add one API-path case (`VersionPair::select` with `b.tag = "CON"`) so
the resolved-tag validator is covered too; the dump writer already shares the
validator, so no second sink test is needed.

**Status: fixed — valid, not rejected.** Reproduced at the validator: the new
`--old-tag "CON"` case failed on the pre-fix tree with `--old-tag "CON" must be
refused`. `paths::is_windows_device_name` is now `pub(crate)`
(`paths.rs:99`) and `tag_shape_problem` calls it after the trailing-dot rule
(`config/mod.rs:618`), so the flag path (`validate_tag`/`validate_new_tag`), the
API path (`VersionPair::select`, `config/mod.rs:121`) and the writer
(`manifest/dump.rs:94`) all share one rule. Tests: the bad list at
`config/mod.rs:1043` gains `CON`, `con`, `NUL`, `NUL.txt`, `aux.txt`, `COM1`,
`COM1.bin`, `LPT9`, `CONIN$`, `CONIN$.txt`; the good list keeps `7.1.0` and adds
`CONSOLE`/`console`/`COM0`/`LPT10`; `api_resolved_tags_are_validated_before_they_name_a_directory`
asserts `main.tag = "CON"` is refused naming `main.tag` and `device`;
`a_tag_cannot_name_a_windows_device` pins the `device` reason for each name and
both extension forms. Commits `63ba129`, `f049c12`. Scope note: the
device-name rule is unchanged and now shared; the `pair_dir` component
`CON__7.1.0` is still an ordinary name, as the finding says.

---

### L2 — `bytes_admitted` is not the saturating total the ledger is

**Severity: low. Class: bug. Location:** `src/select/mod.rs:805`, `:811-815`;
sibling `src/select/budget.rs:122-129`; reporting split
`src/emit/mod.rs:237-246`, `src/run/mod.rs:400`.

The budget's own arithmetic is deliberately total:

```rust
// src/select/budget.rs:51-64
pub fn admits(&self, cost: u64) -> bool {
    match self.limit {
        None => true,
        Some(limit) => self.spent.saturating_add(cost) <= limit,
    }
}

pub fn charge(&mut self, cost: u64) {
    self.spent = self.spent.saturating_add(cost);
}
```

with "a budget is a convenience and must never be the thing that panics". The
selection total next door is a plain sum:

```rust
// src/select/mod.rs:805
let bytes_admitted: u64 = admitted.iter().map(|a| a.budget_cost).sum();
```

The debug assertion ties it to the saturated ledger only while the sum itself
does not overflow:

```rust
// src/select/mod.rs:811-815
debug_assert_eq!(
    bytes_admitted,
    final_budget.spent(),
    "the admitted sum and the charged budget must be the same number"
);
```

`budget::total_admitted` repeats the plain sum (`budget.rs:122-129`).

**Scenario.** The manifest decoder rejects only negative costs; a chunk may
declare `compressed_size = i64::MAX` (`manifest/proto.rs:70-97`). For one
`identical` file, `old_bytes` and `new_bytes` are each that value, and the scoring
add saturates:

```rust
// src/manifest/diff.rs:505
total_cost: old_bytes.saturating_add(new_bytes),
```

so one admission costs `2 * i64::MAX == u64::MAX - 1`. With `--per-class 2` and
two such paths (or one path in each of two classes), both are admitted under the
default unlimited budget, and `sum()` of two `u64::MAX - 1` values is
mathematically `2^65 - 4`. `Budget::charge` saturates to `u64::MAX`; the plain sum
does not. Executed outside the repository:

```text
debug:   thread 'main' panicked at .../iter/traits/accum.rs:206:1:
         attempt to add with overflow
release: 18446744073709551612        # u64::MAX - 3
```

In a debug build the capture panics before the summary; in a release build
`run/mod.rs:400` reports `capture_bytes_admitted = u64::MAX - 3` while
`diff/matrix.json → budget.spent` reports the saturated `u64::MAX`
(`emit/mod.rs:244`). The debug assertion cannot catch it: in debug the sum
panics first, and in release the assertion is compiled out.

This is the same class the seventh pass fixed: K1 says "measurement totals
saturate; address-like sums and plan costs use `checked_add`"
(`ADVERSARIAL_REVIEW.md:681`). `bytes_admitted` is a measurement total printed in
the run summary, and the saturated ledger beside it is the number the artifact
already treats as authoritative.

**Suggested fix.** Use a saturating fold at both sites:

```rust
let bytes_admitted: u64 = admitted
    .iter()
    .fold(0, |total, a| total.saturating_add(a.budget_cost));
```

and the same in `total_admitted`. The debug assertion then holds by construction.
Add one regression test that builds two candidates with `total_cost: u64::MAX - 1`
(or drives `select` with `per_class = 2` and an unlimited budget) and asserts
`selection.bytes_admitted == selection.budget.spent() == u64::MAX` without a
panic; add the equivalent for `budget::total_admitted`.

**Status: fixed — valid, not rejected.** Reproduced through the arithmetic path
and through `select` with a fixture. Before the fix,
`select::tests::two_maximum_costs_report_the_saturated_ledger_not_an_overflow`
(two `identical` rows with `total_cost: u64::MAX - 1`, `per_class = 2`,
`Budget::unlimited()`) panicked in debug at
`core/src/iter/traits/accum.rs:206:1: attempt to add with overflow`, and the
release run of `select::budget::tests::a_total_of_maximum_costs_saturates_with_the_ledger`
reported `18446744073709551612` (`u64::MAX - 3`) against the ledger's
`u64::MAX` — exactly the wrap the finding describes. `bytes_admitted`
(`select/mod.rs:805-807`) and `total_admitted` (`budget.rs:122-135`) now fold
with `saturating_add`, so the debug assertion at `select/mod.rs:817` holds by
construction. Both tests pass in debug; the release run of the two tests also
passes (`2 passed; 0 failed`). Commit `06c650b`.

---

## Verification

Verified on the fixed tree (`f049c12`, after both fixes):

| Command | Result |
|---|---|
| `cargo build -p gircap` | finished, no warnings (0.76 s incremental) |
| `cargo test -p gircap` | **693 passed, 0 failed, 0 ignored** (549 lib + 144 integration; 12 suites; doc-tests 0) |
| `cargo clippy -p gircap --all-targets` | finished in 36.64 s; no diagnostics |
| `cargo fmt -p gircap --check` | exit `0`; no diff |

Integration total: 3 + 8 + 32 + 2 + 30 + 10 + 29 + 15 + 15 = 144; no test is
`#[cfg(unix)]`, and the Windows-only tests ran.

The two probes were first executed on the review tree (outside the repository,
temp directory) and then pinned as in-tree regression tests:

| Probe | Result |
|---|---|
| `std::fs::create_dir_all("<temp>/gircap-con-probe/manifests/CON")` on this Windows host | `Err(Os { code: 267, kind: NotADirectory, message: "The directory name is invalid." })`; a following `File::create` gives code 3 |
| `[u64::MAX - 1, u64::MAX - 1].iter().sum::<u64>()` | debug builds: panic `attempt to add with overflow` (exit `101`); `-O`: `18446744073709551612` (exit `0`) |

The in-tree reproductions before the fixes were: `--old-tag "CON" must be
refused` failing at `config/mod.rs:1046`; `select`'s two-maximum-cost fixture
panicking at `core/src/iter/traits/accum.rs:206:1` (`attempt to add with
overflow`); and the release budget test printing `18446744073709551612`
against `u64::MAX`. After the fixes both tests pass in debug and
`cargo test -p gircap --release --lib maximum_costs` passes (2 passed).

## Not verified

- **L1 end to end.** No capture was run with `--new-tag CON`: it is now refused
  at argument resolution, before any API call, which is the fix; the late sink
  failure was reproduced at the exact standard library call the writer makes.
- **L1 cross-host.** A Linux-produced `manifests/CON/` bundle was not copied to
  Windows and run through `verify`; the missing-file consequence is reasoned
  from A16's walk/checksum contract, not executed.
- **L2 end to end through the command.** The crafted cost is driven through
  `select` with a `PathScore` fixture, not through a hand-edited manifest read
  from disk by `gircap capture`. The decode boundary is K1-tested separately.
- **Live inputs.** No live tag, manifest, or API response was sampled for a DOS
  device name or an extreme `compressed_size`; both findings are about locally
  accepted values, not observed production values.
