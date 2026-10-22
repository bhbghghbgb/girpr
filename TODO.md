# TODO — the tests that scan a directory they do not own

**Two open problems, both the same shape: a test that reads a *directory* to find the file it
wrote, instead of reading the file it asked for.** Neither is fixed. This file is the reminder,
and it goes when the last one is fixed.

Neither problem was caused by the scratch-directory refactor (`17dfc98`, `crates/tempdir/`) and
neither is fixed by it. They are recorded here so they are not lost.

---

## 1. `target/debug/logs/` — never pruned, and scanned by three tests

**What happens.** Every run of every program writes a log file beside its executable:
`<exe-dir>/logs/<project>_<timestamp>.log` (`crates/log/src/plan.rs:224-239`,
`default_log_path`). Nothing ever prunes that directory, so it grows by one file per program run,
forever.

Three tests then find "the file this run wrote" by **listing that directory and reading every
file in it**:

| test | where |
|---|---|
| `a_log_file_is_always_written_beside_the_executable` | `tests/cli_contract.rs:373` (girpr) |
| `a_log_file_is_always_written_beside_the_executable` | `tools/capture/tests/cli_contract.rs:440` (gircap) |
| `a_log_file_is_always_written_beside_the_executable` | `tools/sync/tests/output.rs:571` (girsync) |

Each one reads **every** file's whole content and searches it for a marker unique to that run.

**Measured on this machine (2026-10-22), before the directory was cleared:**

| | |
|---|---|
| files in `target/debug/logs/` | **6152** |
| bytes in `target/debug/logs/` | **216 MB** |
| the three tests' cost | reported **SLOW** at >180 s, >480 s and >540 s |
| `cargo nextest run --workspace` | **380 s** in total, most of it those three |

**Why it matters.** The cost of a test is proportional to the *history of the machine*, not to the
code: the longer the checkout has been used, the slower the suite. A second run is slower than
the first, and a clean machine is a fast machine. That is not a property a test suite is allowed
to have.

**It is also the shape of the leak the refactor was about.** A scratch tree is removed when the
guard drops; a log file beside the executable is never removed, and the tests that read it pay
for every file that was ever written.

---

## 2. The system temp directory — walked recursively by two `gircap` tests

**What happens.** `TempDir` puts every scratch tree directly in the system temp directory
(`crates/tempdir/src/lib.rs:82`). Two tests then point `gircap::verify::list` at the *parent* of
their scratch root:

```rust
// tools/capture/tests/offline_bundle.rs:574
let found = gircap::verify::list(b.root.parent().unwrap()).expect("lists");

// tools/capture/tests/offline_bundle.rs:595
let parent = b.root.parent().unwrap();
```

`b.root` is the `TempDir` root, so `b.root.parent()` is **the system temp directory** — not a
directory the test owns. And `verify::list` is a **recursive** walk
(`tools/capture/src/verify/mod.rs:931-979`): it reads every entry under the root and descends
into every directory it finds.

So these two tests walk `%TEMP%` in full: every other program's leftovers, every installer
cache, every half-extracted archive on the machine.

They also **create** directories there, with **fixed names**, and remove them by hand at the end
of the test rather than through a guard:

| what | where | removed |
|---|---|---|
| `<temp>/not-a-bundle` | `offline_bundle.rs:570` | by hand, `:586` |
| `<temp>/broken-bundle` | `offline_bundle.rs:596` | by hand, `:613` |

Two runs of the suite collide on those names, and a panic before the removal leaves them behind
for the next run to inherit.

**Why it did not show up here.** The system temp directory was cleared before this session, so
the walk had almost nothing to walk. On a machine that has not been cleared it is minutes of I/O
per run, and it is not this repository's fault — it is every other program's leftovers that the
test pays for.

---

## 3. The immediate fix

### 3a. A debug build writes no log file by default; only a release build does

**Where the switch belongs:** `crates/log/src/plan.rs:157-174`, in `LogPlan::resolve`'s
`None => (default_log_path(project), false)` arm. With no `--log-file` and no
`<PROJECT>_LOG_FILE`:

- a **release** build gets today's default — `<exe-dir>/logs/<project>_<timestamp>.log`;
- a **debug** build gets **no file at all**.

`cfg!(debug_assertions)` is the switch. It is on for `cargo test` and `cargo nextest` (dev
profile) and off for `--release`, and it answers the question that is actually being asked —
"is a person running this locally, or is this a shipped binary" — rather than "which build
profile".

**The plan has to be able to say "no file".** It cannot today: `LogPlan` carries
`file: PathBuf` and `file_named: bool` (`crates/log/src/plan.rs:140-148`), so there is no third
state. It needs either `file: Option<PathBuf>` or a `file_enabled: bool` beside the path. The
state has to be **visible in the plan**, because all three projects open the file from it and
each has to be able to skip the sink:

- `src/logging.rs:87` — `match open_log_file(&plan.file)`, falling back to `io::sink()`
- `tools/sync/src/logging.rs:110` — `match open_log_file(&plan.file)`
- `tools/capture/src/logging.rs:141` — `match open_log_file(&plan.file)`

`--log-file` and `<PROJECT>_LOG_FILE` still name a file in **either** build. A test that asks
for a file gets one.

### 3b. The three tests that scan stop scanning

Each of them already knows how to ask for an explicit path: `tests/cli_contract.rs:309-334`
(`an_explicit_log_file_is_echoed_to_stderr_and_created`) passes
`--log-file <a path inside the test's own scratch>` and then reads **that file**.

So the one fact "a log file is always written beside the executable" splits into two facts, and
each gets its own test:

- **the default path** — a *policy* fact, assertable without running anything:
  `default_log_path(project)` has the shape `<exe-dir>/logs/<project>_<timestamp>.log`. That is
  already asserted by `crates/log/src/plan.rs:320`
  (`the_default_file_is_beside_the_executable_in_logs`), `src/logging.rs:133` and
  `tools/sync/src/logging.rs:210` (`the_default_file_is_named_for_the_project`). Move the
  assertion there and be done.
- **the file is written** — a *behaviour* fact, assertable with an explicit `--log-file` inside
  the test's own scratch, which the test named above already does.

What must **not** come back is "run the program with no `--log-file`, then find the file it
wrote by listing the directory". That assertion cannot be made without a scan, because the name
carries a timestamp and the path is therefore not knowable in advance.

### 3c. The policy documents change with the code, not after it

These currently state the opposite of what 3a makes true:

| where | what it says today |
|---|---|
| `README.md:300` | "The file is **always** written — there is no 'no log file' mode." |
| `README.md:314` | "**A log file is always written.** There is no 'no log file' mode" |
| `crates/log/src/lib.rs:7-8` | "**Where does the log file go?**" |
| `src/cli.rs:209` | the `--log-file` help |
| `tools/sync/src/cli.rs:38-40` | "The file is always written, as JSON lines, and appended to." |
| `tools/capture/src/cli.rs:89-91` | "The file is always written, as JSON lines, and appended to." |
| `crates/log/src/plan.rs:224-239` | `default_log_path`'s doc |

The honest statement after the change is: **a release build always writes a log file; a debug
build writes one only when it is asked to.**

---

## 4. The temp-directory walk has its own fix (not the log policy)

The two `gircap` tests must not point `list` at the system temp directory. The fix is to give
the scratch a **test-owned parent**:

- the bundle goes under `<scratch>/artifacts/<biz>/<old>__<new>/<capture_id>/`, and `list` is
  pointed at `<scratch>/artifacts` — never at `<scratch>`'s parent;
- `offline_bundle.rs:547` deliberately uses a **sibling** of the bundle root, because the tool
  picks that path itself. That is the `TempDir::at` case and it is legitimate — it just has to be
  a sibling *inside the test-owned parent*, not a sibling in `%TEMP%`;
- the fixed-name directories (`not-a-bundle`, `broken-bundle`) then live inside the test-owned
  parent too, and go away with the guard instead of by hand.

---

## 5. Done when

- [ ] `cargo nextest run --workspace` writes nothing into `target/debug/logs/`.
- [ ] `target/debug/logs/` stops growing between runs, and the existing contents can be deleted.
- [ ] No test lists a directory to find a file it wrote. `rg -n "read_dir" --glob '*.rs'` shows
      no scan of a `logs/` directory and no scan of the system temp directory.
- [ ] The three "a log file is always written beside the executable" tests are replaced by the
      two facts in §3b, and the documents in §3c say "a release build always writes one; a debug
      build writes one when asked".
- [ ] The two `gircap` `list` tests walk a tree the test owns.
- [ ] This file is deleted.
