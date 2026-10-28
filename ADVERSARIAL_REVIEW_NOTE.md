# Eighth adversarial pass — review note

Scope: `girpr` on `fix-gaps/girpr` at `1d63841`, after the seventh-pass fixes
S-1…S-3. Two unrecorded boundary gaps remain. Neither is destructive; both
discard a failure the review standard says must be visible.

## Summary

| ID | Severity | Class | Location | Summary |
|---|---|---|---|---|
| T-1 | Low | omitted error / fail-open config | `src/util.rs:479-486`, `src/util.rs:506-511` | An unparseable JSON line in the launcher's ignore/blacklist file is dropped with no log; a wholly malformed file degrades to an empty set. |
| T-2 | Low | stat failure read as absence | `src/util.rs:268-270` → `src/repair/file/temp.rs:35-39`, `src/repair/phases/check.rs:61-64`, `src/repair/phases/check.rs:90-93` | `file_len` maps every `metadata` error to `None`, so an unreadable file is logged as "missing" and reported as a size mismatch. |

## Progress

| ID | Verdict | Change | Commit |
|---|---|---|---|
| T-1 | **Valid, fixed.** Reproduced: a BOM-prefixed valid blacklist line plus a truncated second line returned `Ok(Some({}))`; an all-malformed ignore file returned `Ok(Some({}))`; both with no log. `src/util.rs` now strips one leading UTF-8 BOM, warns per unparseable line with the path and 1-based line, and refuses a file whose non-blank text produced no entry and had a parse failure (matching R-8's rule for the audio scan file). A valid entryless `{}` and a blank file stay empty. | `parse_config_list` shared by both readers; unit tests `config_lists_strip_a_bom_and_warn_per_unparseable_line`, `an_all_malformed_config_list_is_refused`; `test_support` is now `pub(crate)` so `util` tests can capture logs. | `ad0a09c` |
| T-2 | Validating | — | — |

## T-1 — Low — an unparseable config line is discarded silently

### Code

`src/util.rs:474-488`:

```rust
for line in text.lines() {
    let line = line.trim();
    if line.is_empty() {
        continue;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line)
        && v.get("is_delete")
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
        && let Some(c) = v.get("category").and_then(|c| c.as_str())
    {
        set.insert(c.to_string());
    }
}
```

`src/util.rs:501-511` does the same for `fileName`. Neither function logs
anything, and neither has an error arm.

### Failure scenario

The launcher's lists are JSON-lines written in place. A UTF-8 BOM on the first
line (not stripped: U+FEFF is not `char::is_whitespace`, the same fact
`src/repair/audio.rs:90-94` records), or a crash-truncated last line, makes that
line fail `from_str`. With `blacklist_dir = blacklist.txt` and a BOM before
`{"fileName":"GenshinImpact_Data/StreamingAssets/..."}`, the only blacklist
entry is dropped: the file stays in the work list, is downloaded and repaired,
and is kept by the cleanup — the run exits `0` with no warning. If every line is
malformed, the blacklist silently becomes "no blacklist" and the ignore list
silently becomes "delete nothing": a category the user asked to `is_delete` is
fetched and repaired instead of purged. The failure direction is extra traffic
and disk, not data loss, but an explicit configuration is ignored without a
trace.

### Why the standard fails

`ADVERSARIAL_REVIEW.md:857` (decision 9) and R-6 (`:829`) fixed the *read* half
of this boundary: only `NotFound` is absent, every other read error is exit `1`.
This is the *parse* half of the same file, and it still drops failures with the
exact `if let Ok` shape the standard names. R-8 (`:836`, `src/repair/audio.rs:95-109`)
already set the rule for the sibling launcher-written list: a non-blank file
that yields no selection is refused, and a mixed file warns once per
unrecognized line. The blacklist/ignore readers do neither.

### Suggested fix

- Warn per unparseable line, the way `audio::read_scan_file` warns per
  unrecognized line (`src/repair/audio.rs:110-116`): `warn!(path, line, error =
  %e, "unparseable line in the … list; ignored")`.
- Refuse (`Err`) a file that has non-blank text but produced no entry *because*
  at least one line failed to parse. Keep tolerating a valid-but-entryless line
  (`{}`, already pinned at `src/util.rs:1155`) and a genuinely blank file.
- Unit tests: a BOM-prefixed first line plus a truncated second line (one valid
  entry survives, the drop is logged), and an all-malformed file (`Err`).

## T-2 — Low — `file_len` turns a stat failure into "absent"

### Code

`src/util.rs:268-270`:

```rust
pub fn file_len(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.len())
}
```

`src/repair/file/temp.rs:35-39`:

```rust
match util::file_len(final_path) {
    None => tracing::debug!("check: missing local file"),
```

`src/repair/phases/check.rs:61-64` (the scan branch; `:90-93` is the same in the
non-scan branch):

```rust
let actual_len = util::file_len(path);
if actual_len != Some(f.size as u64) {
    bad += 1;
    tracing::warn!(file = %f.rel, expect_size = f.size, actual_size = ?actual_len, "CHECK fail: size");
```

### Failure scenario

A deny ACL, a sharing violation, or any `metadata` error other than `NotFound`
yields `None`. Under `--check-only` the file is reported as `CHECK fail: size
actual_size=null`, i.e. as size damage, when it may be the right size and simply
unreadable; the temp path logs `check: missing local file` for a file that is
present. An operator debugging a refused repair reads the wrong cause, and the
two states the review keeps distinct elsewhere (absent vs unreadable) are one
here.

### Why the standard fails

Decision 9 (`ADVERSARIAL_REVIEW.md:857`) says a stat failure is never a zero and
never "not a file"; S-3 (`:845`) applied that to the deprecated delete with a
four-state classifier. `file_len` is the remaining stat helper that answers
"absent" for every error. The action that follows is safe in both call sites
(repair the file / count it damaged), so this is a diagnostics defect rather
than a wrong write — but it is the same collapse S-3 fixed, one helper away.

### Suggested fix

Return `io::Result<Option<u64>>` (`NotFound => Ok(None)`, any other error `Err`),
or add a `classify_len` helper shaped like `post::stat_deprecated`. `check.rs`
then logs "unreadable" separately from "size" and still counts the file as
not-clean; `temp.rs` logs the error and still repairs the file. Pin absent /
present / failed without a permission fixture by classifying an injected
`io::Error`, as `src/repair/phases/post.rs:530-541` does.

## Verification

On `fix-gaps/girpr` at `1d63841`, Windows, debug profile:

| Command | Result |
|---|---|
| `cargo build` | `Finished dev profile [unoptimized + debuginfo] target(s) ... in 46.00s` |
| `cargo test -p girpr` | **335 passed, 0 failed, 7 ignored** — 196 lib, 0 `src/main.rs`, 7 `artifact_server`, 17 `cli_contract`, 25 `repair_machinery`, 90 `repair_offline`, 0 doc-tests; ignored: 6 `artifact_bridge` + 1 `peak_heap` |
| `cargo clippy -p girpr --all-targets` | `Finished dev profile [unoptimized + debuginfo] target(s) ... in 24.77s`; no warnings |
| `cargo fmt --check` | exit `0`, no diff |

`git status --short --branch` before the review: `## fix-gaps/girpr`, clean.

## Not verified

- T-1 was not reproduced through the binary: no offline fixture writes a
  BOM-prefixed or truncated `blacklist_dir`/`res_category_dir` file, so the
  claim is code-reading plus `serde_json`'s rejection of a leading U+FEFF.
- T-2's stat failure was not reproduced on this host (no ACL fixture, and
  `file_len` has no injection seam); the claim is code-reading.
- The `#[cfg(unix)]` blocks of the containment and mode tests did not run
  (Windows host).
- The 7 ignored tests (6 `artifact_bridge`, 1 `peak_heap`) did not run.
- No release-profile run, no Linux/macOS host, no live CDN traffic.
