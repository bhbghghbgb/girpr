# T08 — Remove redundant post-phase mkdir syscalls and materialization

## Goal / pointers
- `docs/OPTIMIZATION_REPORT.md` §3 **IO-07** (lines 277–285) — `create_dir_all` is
  called for every audio-cache file in `post.rs:684`, and
  `any_component_has_windows_trailing_dot_or_space` uses `to_string_lossy()` per file.
  Keep a `HashSet<PathBuf>` of parents already created and add a byte-based component
  scan for the common ASCII case.
- §3 **IO-08** (lines 286–293) — the audio-cache move collects all `WalkDir` paths
  into a `Vec<PathBuf>` then iterates twice; stream the walk into the per-file
  decision instead.
- Keep items: best-effort post-phase semantics, §12, `guidance/DO-NOT-TOUCH.md`.

## Scope
- `src/repair/phases/post.rs` (audio-cache move ~629–657, `create_dir_all` ~684,
  `check_publish_target` ~477–511).
- `app/util-path/src/path.rs` (`any_component_has_windows_trailing_dot_or_space`
  ~264–269).
- Locate current code before editing; the list above is a hint.

## Implementation plan
1. Confirm the per-file `create_dir_all` and the collect-then-iterate walk in the
   current tree. If the full path list is genuinely required, skip only the streaming
   part (IO-08) and keep the mkdir dedup (IO-07).
2. Track created parents in a `HashSet<PathBuf>` and call `create_dir_all` once per
   directory.
3. Add a byte-based fast path for the ASCII common case in the trailing-dot/space
   check, preserving behavior on non-ASCII and Windows edge cases.
4. Stream `WalkDir` results into the per-file decision rather than collecting all
   paths first.
5. Deliver in one or a few commits.

## Constraints / validation
- The post/audio-cache phase stays best-effort; do not turn failures into fatal
   errors.
- The Windows trailing-dot/space check must remain semantically identical on all
  inputs; keep tests for edge cases.
- Do not change data-plane output or any §12 non-finding.

## Done when
`create_dir_all` is called once per parent, the walk is streamed, and the ASCII fast
path is behavior-preserving with tests passing. If the list is genuinely required or
the byte scan cannot preserve semantics, **stop** and record `skipped` with a one-line
reason.
