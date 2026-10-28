# Sixth adversarial pass — R-7…R-9

Reviewed tip: `1f00dbd` (`docs(review): fold the fifth-pass verdicts into the
review`), branch `fix-gaps/girpr`. Scope: `src/**`, `tests/**`, `docs/**`,
`README.md`, root `Cargo.toml`. The fifth-pass fixes R-1…R-6 still hold at
their primary sites; the pass found three new gaps, two of them behavioural.
No source, test, doc, README or Cargo file was changed by this pass.

## 1. The fifth-pass fixes still hold

| # | Verdict | Site checked |
|---|---|---|
| R-1 | **Holds.** The document is computed once (`disk`) and written in both branches, before the execution phase. | `src/repair/mod.rs:315-344` |
| R-2 | **Holds.** Three-valued read (`Ok(None)` only for an unconfigured dir / `NotFound`, empty `Some` = game-only, `Err` otherwise), atomic scan write, exit `1` mapping. | `src/repair/audio.rs:75-143`; `src/repair/phases/metadata.rs:171-206` |
| R-3 | **Holds.** The chunk error stays the cause; the rollback failure is context. | `src/repair/file/temp.rs:198-207` |
| R-4 | **Holds.** Every `remove_file` error except `NotFound` propagates as `discard <path>`. | `src/repair/file/inplace.rs:444-450` |
| R-5 | **Holds.** `sync_all` failure aborts the publish and removes the temp. | `src/util.rs:717-739` |
| R-6 | **Holds.** `read_config_list` maps `NotFound` to `None` and every other read error to `Err`; all three readers use it. | `src/util.rs:496-522`; `src/repair/phases/metadata.rs:124-145` |

## 2. New findings

### R-7 — Medium — `--audio` mutates the install before the ceilings and `--plan-out`

**Location:** `src/repair/phases/metadata.rs:179-206` (write),
`src/repair/mod.rs:95` (call), `:146-149` (exit `2`), `:192-209` (exit `1`),
`:287-302` (exit `5`), `:335-344` (exit `3`); claims in
`src/repair/mod.rs:20-24`, `:299-300`, `README.md:300`,
`docs/04-scope-limitation-spec.md:420-422`.

**What the code does.** `metadata::fetch` resolves the effective audio set and,
when `--audio` was explicit, calls `audio::write_scan_file` immediately
(`metadata.rs:203`). That is inside the plan half, before `build_plan`
(`mod.rs:146-149`, exit `2` on a hostile rel), before the empty-`--include`
refusal (`mod.rs:192-209`, exit `1`), before the scope-of-work ceilings
(`mod.rs:287-302`, exit `5`) and before the `--plan-out` write
(`mod.rs:335-344`, exit `3`). `write_scan_file` also calls
`create_dir_all(parent)` (`audio.rs:119-122`), so on a missing install it
creates `game_dir` as well as the scan file.

**Why it matters.** Every one of those refusals is documented as leaving the
install untouched: “Nothing on disk has been touched at the end of this half”
(`mod.rs:21-22`), “Nothing has been written at this point — no directory
created” (`mod.rs:299-300`), and “before the first mutation, so a write failure
is exit `3` with nothing changed” (`README.md:300`, `docs/04:421-422`). With an
explicit `--audio` and a configured scan dir, a run that exits `5` for
`--max-files 0`, exits `2` for a bad manifest rel, exits `1` for a typo'd
`--include`, or exits `3` because the plan-out path is unwritable has already
rewritten the user's audio selection and, on a missing install, created the
game directory. The fifth pass's own §11.2 item 2 (“before any install
mutation”) is contradicted by the fifth pass's scan write.

This is also pinned as an invariant by
`max_files_refuses_before_touching_anything` (`tests/repair_offline.rs:1877-1885`:
“a refusal must not even create game_dir”). That test does not pass `--audio`;
with it, the scan write creates the directory before the ceiling is reached.

**Required fix.** Keep the path-containment and scope-visibility checks in
`metadata` (they are validation), but defer `write_scan_file` to the execution
phase, after the ceiling check and the `--plan-out` write and after the
`--print-plan` return. The repair uses the in-memory `meta.audio`, not the file,
so moving it changes nothing else. A failure then occurs in the execution
phase, which is already allowed to start mutating. If the write is intentionally
early, the contract in all four sites must say so and a test must pin the
exception; the review standard here is “no partial application on a refused
run”, so deferring is the smaller change.

**Regression test.** With the mock's `scan_dir` configured and a missing
`game_dir`: explicit `--audio` plus a tripped ceiling (`--max-files 0`) and,
separately, an unwritable `--plan-out` directory → exit `5`/`3`, no game dir,
no scan file. `--include` selecting nothing and an escaping manifest rel are
the same assertion with exit `1`/`2`.

### R-8 — Medium — an all-unrecognized scan file reads as game-only, and `--purge-after` deletes the selected audio

**Location:** `src/repair/audio.rs:88-102` (`_ => None` at `:96`), the
consequence chain `src/repair/audio.rs:48-52` → `src/sophon.rs:81-90` →
`src/repair/purge.rs:88`, `:347-359`; test at `src/repair/audio.rs:184-199`;
spec at `docs/02-repair-chunk-spec.md:59-66`.

**What the code does.** Every line that is not one of the four exact display
names is silently dropped. A file whose lines are all unrecognized therefore
returns `Ok(Some(empty))`, which `resolve_effective_audio` keeps as the real
game-only selection. The distinction R-2 introduced — “readable empty is a
selection, not absence” — is correct, but the reader cannot tell a genuinely
empty file from a non-empty file it understood nothing in.

A UTF-8 BOM is enough. `char::is_whitespace` is false for U+FEFF, so
`"\u{feff}Chinese".trim()` is still `"\u{feff}Chinese"` and matches no arm
(verified with a standalone `rustc` probe). A renamed display name
(`English (US)` with a space) or a language added upstream has the same effect.

**Why it matters.** An empty audio set makes `select_manifests` drop every
audio manifest (`sophon.rs:81-90`), so those paths are absent from
`plan.expected_paths()`, which is the purge keep set (`purge.rs:88`). Under
`--purge-after` or `--purge-only`, the audio packages of the language the user
actually selected are classified `Purge` (`purge.rs:347-359`) and deleted —
exactly the data-loss consequence R-2 documented for the old empty-vs-absent
confusion, reached by a different input. The unit test
`english_aliases_and_unconfigured_dir_are_tolerated` deliberately pins
tolerance when a known line is also present; it does not cover the all-unknown
case.

**Required fix.** Count recognized lines. If the file contains non-blank text
but zero recognized language lines, return `Err` naming the file and the first
unrecognized line (exit `1`, as R-2 maps it); keep blank lines and a genuinely
empty file as game-only. When at least one line is recognized, warn on each
unrecognized non-blank line instead of dropping it silently. Stripping one
leading U+FEFF before parsing is a reasonable companion fix, but the fail-fast
rule is what closes the data-loss path.

**Regression tests.** All-unknown non-empty file → exit `1`, no chunk request,
scan file unchanged. Mixed known + unknown → known set plus a warning. BOM +
`Chinese` → either recognized as `zh-cn` or refused; never read as game-only.

### R-9 — Low — the `--plan-out` help still documents the pre-`mode` JSON shape

**Location:** `src/cli.rs:177-180` versus `src/repair/scope/report.rs:232-245`
and `docs/04-scope-limitation-spec.md:417-419`.

**What the code does.** The flag help says the document is
`{repair:[{rel,size,chunks_total,est_download_bytes}],purge:[{rel,size}]}`.
`RepairEntry` also serializes `mode` and `extra_bytes`, and `docs/04` now shows
both. The fifth pass corrected the spec but left the CLI text one shape behind;
`help_text_lists_every_documented_flag` only checks flag names, so nothing
catches it.

**Required fix.** Extend the help string to the serialized shape
(`…,mode,extra_bytes`). No behaviour changes.

## 3. Verification (pre-fix baseline)

Run on Windows, debug profile, at tip `1f00dbd` with only
`ADVERSARIAL_REVIEW_NOTE.md` added:

- `cargo build` — ok (`Finished dev profile … in 43.66s`).
- `cargo test -p girpr` — **320 passed, 0 failed, 7 ignored**: 185 lib + 7
  `artifact_server` + 16 `cli_contract` + 25 `repair_machinery` + 87
  `repair_offline`; 0 `src/main.rs`, 0 doc-tests. Ignored: 6
  `artifact_bridge` + 1 `peak_heap`.
- `cargo clippy -p girpr --all-targets` — clean, no warnings
  (`Finished dev profile … in 17.73s`).
- `cargo fmt --check` — exit `0`, no output.

**Not run:** the `#[cfg(unix)]` blocks of the containment tests (Windows host);
the 7 ignored tests; no release profile, Linux/macOS host or live CDN traffic.
R-5's `sync_all` failure still has no deterministic injection seam. R-7 and
R-8 were established by reading the exact source path; no fixture in the tree
combines explicit `--audio` with a refusing run, and none supplies an
all-unrecognized scan file, so neither was executed end to end. The BOM trigger
for R-8 was executed in a standalone probe.

**Checked and accepted (not findings):** `promote_tmp` still renames without
`sync_all` (a crashed run leaves no version bump and the next run re-verifies;
less urgent than the R-5 `config.ini` publish), `PlanDocument::write_to` still
uses a truncating `fs::write` (a failed artifact is detectable and the run
exits `3`), `DiskPlan` still conflates missing and unreadable with `stat().ok()`
(documented over-approximation, §9.4/§10.2), and the purge's decision to keep
only `config.ini`, the lock and `audio_lang_*` / `Audio_*_pkg_version` is the
documented v1 Genshin parity (`purge.rs:282-298`).

## 4. Sixth-pass validation and fixes

All three findings were checked against the source at their cited lines and
then reproduced before the fix. R-7 and R-8 were executed through the real
binary against the in-process mock; R-9 was read from `--help`. All three were
valid and are fixed. No shared crate or tool was touched.

| # | Verdict | What changed | Commit |
|---|---|---|---|
| R-7 | **Valid** (Medium, partial mutation) | `metadata::fetch` no longer calls `write_scan_file`. It keeps the containment validation and the explicit-`--audio`/scope refusal (both are plan-half validation), and `repair::run` writes the scan file in the execution phase — after `create_dir_all`, the single-writer lock, the ceilings, the empty-`--include` refusal, the plan build and the `--plan-out` write. The repair uses the in-memory `Meta::audio`, not the file. | `be6d83f` |
| R-8 | **Valid** (Medium, data loss) | `read_scan_file` parses through `parse_scan_text`: one leading U+FEFF is stripped, and non-blank text that yields zero recognized languages is `Err` naming the file and the first unrecognized line (mapped to exit `1`). A recognized-plus-unrecognized file keeps the known set and warns per unknown line; a blank/empty file is still the game-only selection. | `a44c666` |
| R-9 | **Valid** (Low, doc/CLI) | The `--plan-out` help now lists `mode` and `extra_bytes` and says what they mean. `docs/04:417-419` already showed the full shape; only the CLI text was one shape behind. | `c9fb86f` |

**R-7 reproduction.** `explicit_audio_is_not_written_before_a_refusal`
(`tests/repair_offline.rs`) drives the binary with `--audio en-us` and, in
turn, `--max-files 0` (exit `5`), an unwritable `--plan-out` directory (exit
`3`) and an `--include` matching nothing (exit `1`), each against a missing
game dir. On the unfixed tree it failed at the first case: exit `5` had
already created `<game>/Audio/scan.txt`. With the write deferred it passes:
no game dir, no scan dir, no chunk request.

**R-8 reproduction.** On the unfixed tree an `Audio/scan.txt` containing only
`English (US)` produced `keeping current audio langs: {}` (game-only) and the
run exited `0` after fetching chunks. The new binary test
`an_all_unrecognized_audio_scan_file_is_refused_not_read_as_game_only` requires
exit `1`, an error that names the file and the line, no chunk request, and the
file left byte-identical. Unit tests cover the same error through
`resolve_effective_audio`, a BOM-prefixed `Chinese` (now `zh-cn`), and a mixed
file whose unknown line is identified. The purge consequence was established
by reading the chain (`select_manifests` drops audio manifests →
`plan.expected_paths()` loses them → `classify_purge_path` returns `Purge`); it
was not executed end to end because the mock fixture carries a single `game`
manifest and no per-language package to delete. The new refusal removes the
empty set that reached that chain.

**R-9 reproduction.** `--help` showed
`{repair:[{rel,size,chunks_total,est_download_bytes}],purge:[{rel,size}]}`.
The new binary test `plan_out_help_documents_every_serialized_field`
(`tests/cli_contract.rs`) fails on that text and passes on
`…,mode,extra_bytes…`.

**Surviving decisions.** (1) The plan half is now literally mutation-free; an
explicit `--audio` writes the scan file only in the execution phase, so
`--print-plan`, `--dry-run` and `--check-only` still write nothing. (2) A
non-empty scan file naming no known language is local-config exit `1`, not the
game-only selection; an editor-added BOM is tolerated because it is stripped
before parsing. (3) Mixed known/unknown lines stay tolerant, with a warning per
unknown line; the warning is a code-reading claim because tracing caches a
callsite's interest globally, which makes a log capture order-dependent.

## 5. Verification after the fixes

Full run on Windows, debug profile, final sixth-pass tree:

- `cargo build` — ok.
- `cargo test -p girpr` — **326 passed, 0 failed, 7 ignored** (188 lib + 7
  `artifact_server` + 17 `cli_contract` + 25 `repair_machinery` + 89
  `repair_offline`; ignored: 6 `artifact_bridge` + 1 `peak_heap`; 0
  `src/main.rs`, 0 doc-tests). The fifth-pass tree had 320; the six new tests
  are the difference (two binary R-7/R-8, three audio unit, one CLI help).
- `cargo clippy -p girpr --all-targets` — clean, no warnings.
- `cargo fmt --check` — clean (exit `0`).

**Not run:** the `#[cfg(unix)]` blocks of the containment tests (Windows
host); the 7 ignored tests; no release-profile run; no Linux/macOS host; no
live CDN traffic. R-8's purge-deletion consequence and the mixed-line warning
capture are the two sixth-pass behaviours with no executed test, for the
reasons above.
