# T38 - B2: remove the `regex` dependency

Report: `../../OPTIMIZATIONS_REPORT.md` lines 971-989 (B2).

Dependencies: none.

## Goal

`util::ini::game_version_re` is one anchored pattern compiled once via
`OnceLock`, used only by `read_game_version`. The INI reader already splits
lines and parses `key=value`; the regex adds a heavy dependency and a
panic-on-literal `expect` for semantics that are easy to write by hand.

Replace the regex with a hand-written line scan and remove the dependency.

## Where to start

- `src/util/ini.rs`: `read_game_version` (around line 22), `game_version_re`
  (around line 42), the version tests (around lines 443-470).
- `Cargo.toml`: the `regex` dependency.

## Steps

1. Implement the documented semantics directly:
   - scan lines;
   - a line whose **first** character starts `game_version` (no leading
     whitespace) followed by optional whitespace, `=`, optional whitespace, and
     at least one value character;
   - trim trailing whitespace from the value;
   - the **last** match wins.
   The current pattern is `(?m)^game_version\s*=\s*(.+?)\s*$`; note that
   `.` does not match a newline and `$` is end-of-line. Match that behavior,
   including a value containing `=` (the first `=` separates key and value).
2. Do not accidentally accept `game_version_extra=...`; require the key to be
   exactly `game_version` before the optional whitespace.
3. Do not accept leading whitespace before `game_version`; the test
   `version_shape` pins this.
4. Return `Ok(None)` for absent/malformed, as today. Keep the read error
   handling for a missing or unreadable `config.ini`.
5. Remove the `regex` dependency from `Cargo.toml`. Do not remove it from
   `tools/` if their crates use it; this is the main package.
6. Keep the existing tests; add one for trailing whitespace and one for a
   `game_version` with an `=` in the value if not already covered.

## Tests

- `cargo test -p girpr`, especially `version_last_match_wins` and
  `version_shape`.
- A table of lines: leading space, no value, tabs around `=`, trailing spaces,
  `game_version_extra`, duplicate keys, and a value with `=`.
- `cargo tree -p girpr` no longer lists `regex` (unless a transitive
  dependency still does; note it if so).

## Acceptance

- `read_game_version` uses no regex and returns the same answers.
- The `regex` dependency is gone from the main package.
- The existing version tests pass unchanged.
- `cargo test -p girpr` passes.

## Commit plan

1. `refactor(ini): read game_version without a regex`
2. `build(deps): drop the regex dependency`

## Notes

- The regex's panic-on-literal `expect` is also removed. Do not replace it with
  another `expect` in the parse path.
- Keep the INI parser and this reader sharing the same "split at the first `=`"
  rule where possible; the report notes that is the gain beyond the dependency.
