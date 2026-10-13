//! The `game/` fixture: `config.ini`, the audio scan file, and the variant files.
//!
//! **M8.** Exactly three kinds of entry and nothing else, so the directory can be
//! handed to the tool under test untouched. `config.ini` mirrors that tool's own
//! writer so its surgical rewrite is a no-op except for `game_version`, and the
//! scan file's path comes from `getGameConfigs.audio_pkg_scan_dir` — never
//! hardcoded.