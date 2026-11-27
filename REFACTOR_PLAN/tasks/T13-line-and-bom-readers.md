# T13 — Line/BOM readers and canonical renderings

> Temporary plan: these files and the report are deleted at completion. Do not
> reference them from code or permanent docs. Validate every claim against the
> current tree before editing; stop if the task is stale or blocked.

## Goal

Share line-oriented reading (BOM strip, 1-based accounting, blank skipping) and
parameterize the canonical audio/ini renderings whose contracts differ by
newline and selection policy.

## Report references (read only these)

- `docs/09-shared-code-refactor-findings.md:202-215` — X11, parallel readers.
- `docs/09-shared-code-refactor-findings.md:422-426` — APP-06, four BOM readers.
- `docs/09-shared-code-refactor-findings.md:575-582` — LF-08, display-name map.
- `docs/09-shared-code-refactor-findings.md:773-779` — CAP-07, newline contracts.
- `docs/09-shared-code-refactor-findings.md:781-786` — CAP-08, version readers.

## Boundary / approach

Add a line/BOM iterator leaf. Add `render_scan_file(codes, trailing_newline: bool)`
in `girpr-audio-language`, `game_version_from_text(text, Selection,
SectionRequirement)` in `girpr-util-ini`, and a parameterized reverse display-name
lookup in `girpr-audio-language` taking caller aliases.

## Guardrails

- BOM-only-file behavior is opposite in audio and util-ini; it must be a
  parameter, not a convergence.
- Trailing-newline contracts are opposite and explicitly claimed identical in
  `tools/capture/Cargo.toml`; make the flag explicit.
- Capture reads first match in `[General]`; repair reads last match with no
  section filter. A13 self-check wording and repair's last-match tests are pinned.
- Validation policy and file I/O stay at call sites; capture may drop its
  `ini_core` dependency if it becomes unused.
- The audio-language leaf documents that it owns no alias map; the alias
  parameter must be explicit.

## Commit breakdown

1. Add line/BOM iterator leaf with tests (BOM-only, blank lines, 1-based lines).
2. Add parameterized `render_scan_file`; adopt audio and capture.
3. Add `game_version_from_text`; adopt util-ini and capture.
4. Add reverse lookup + joined display-name helper; adopt `app/audio`.

## Validation

- Newline byte checks on generated files; ini/gamedir golden tests.
- `cargo nextest` for audio, audio-language, util-ini and capture.

## Done when

The reader and renderings have one implementation each, parameterized where the
programs deliberately disagree.
