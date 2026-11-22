//! Repair write-mode policy: the requested mode, the decided mode, and the one
//! rule that connects them.
//!
//! Every write path needs the same answer to one question: does this file go
//! through the in-place writer or the temp writer? The answer is decided here
//! once. The writer probes a path with [`resolve_mode`]; the disk estimate
//! injects the very same target fact with [`resolve_mode_from_target`] so its
//! test seam cannot drift from the writer; the download estimate probes with
//! [`resolve_mode`] so its temp/discard branches agree with the writer.
//!
//! This is a leaf. It owns no plan, scan, phase or report types, emits no bytes
//! and re-exports nothing from the binary. Its only dependency is the optional
//! clap `ValueEnum` derive for the `--repair-mode` flag. In particular the
//! `file ↔ scan` cycle does not pass through here: both modules consume this
//! crate directly.

use std::path::Path;

/// How a file is repaired.
///
/// The mode decides **where the bytes go**, not whether the file is
/// repaired: every mode ends with the same whole-file MD5 gate, and every
/// mode can fail the same way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum RepairMode {
    /// Repair in place when the target can be opened for writing, and write
    /// the file the `temp` way when it cannot. The default.
    #[default]
    Auto,
    /// Repair in place: write only the bytes that are wrong, at the offsets
    /// they belong at. No `_tmp` file, and the peak extra disk is the file's
    /// *growth* rather than its size.
    Inplace,
    /// Assemble the repair in `{path}_tmp` and rename it over the target. The
    /// target file is never modified and the swap is atomic, and the peak
    /// extra disk is the whole file.
    Temp,
    /// Delete the target and write the file from scratch. Zero peak extra
    /// disk, and a full download for every file it touches.
    Discard,
}

impl RepairMode {
    /// The value as it appears in `--repair-mode` and in `SUMMARY`.
    pub fn as_str(&self) -> &'static str {
        match self {
            RepairMode::Auto => "auto",
            RepairMode::Inplace => "inplace",
            RepairMode::Temp => "temp",
            RepairMode::Discard => "discard",
        }
    }
}

impl std::fmt::Display for RepairMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How one file was written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Repaired in place: the target was written where it lies.
    #[default]
    Inplace,
    /// Written through `{path}_tmp` and renamed over the target.
    Temp,
}

/// The single mode decision, over the two facts that can change it.
///
/// `in_place` is whether the target can be written where it lies: a regular
/// file, or a path that does not exist yet (the in-place writer creates it).
/// A directory, a symlink or another non-file target is `false`.
///
/// This is the pure form of [`resolve_mode`], used by the disk estimate's
/// injection seam. Keeping the rule here means the estimate and the writer
/// cannot disagree, whatever source supplies the target fact.
pub fn resolve_mode_from_target(requested: RepairMode, in_place: bool) -> Mode {
    if requested == RepairMode::Temp || !in_place {
        Mode::Temp
    } else {
        Mode::Inplace
    }
}

/// How one file will be repaired, probed from the target on disk.
///
/// The table:
///
/// | the target               | mode                          |
/// |--------------------------|-------------------------------|
/// | does not exist           | in place — it is created      |
/// | an existing regular file | in place                      |
/// | a directory, a symlink   | temp — it is not a file       |
///
/// `--repair-mode` decides whether the in-place path is tried at all:
/// `temp` never is, and `discard` deletes the target first and then repairs
/// it in place.
///
/// A target that turns out to be unopenable for writing is **not** decided
/// here: the attempt finds out and answers `Inplace::Unavailable`, and the
/// caller falls back to the temp path with a warning. Deciding it from the
/// metadata alone would be a guess; deciding it from the open is a fact.
///
/// `temp` short-circuits the stat: no target fact can change its answer.
pub fn resolve_mode(requested: RepairMode, path: &Path) -> Mode {
    if requested == RepairMode::Temp {
        return Mode::Temp;
    }
    let in_place = match std::fs::symlink_metadata(path) {
        Ok(m) => m.is_file(),
        // Not there, or the stat failed: the in-place path creates the file.
        Err(_) => true,
    };
    resolve_mode_from_target(requested, in_place)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The published spelling of every flag value, and its display form.
    #[test]
    fn repair_mode_keeps_its_flag_spelling() {
        assert_eq!(RepairMode::default(), RepairMode::Auto);
        assert_eq!(RepairMode::Auto.as_str(), "auto");
        assert_eq!(RepairMode::Inplace.as_str(), "inplace");
        assert_eq!(RepairMode::Temp.as_str(), "temp");
        assert_eq!(RepairMode::Discard.as_str(), "discard");
        assert_eq!(RepairMode::Temp.to_string(), "temp");
    }

    /// The injected form of the rule: only `temp` or a non-file target picks
    /// the temp path. `discard` is in place here, as it is in the writer.
    #[test]
    fn the_pure_rule_matches_the_resolution_table() {
        for requested in [RepairMode::Auto, RepairMode::Inplace, RepairMode::Discard] {
            assert_eq!(
                resolve_mode_from_target(requested, true),
                Mode::Inplace,
                "{requested:?}"
            );
            assert_eq!(
                resolve_mode_from_target(requested, false),
                Mode::Temp,
                "{requested:?}"
            );
        }
        assert_eq!(resolve_mode_from_target(RepairMode::Temp, true), Mode::Temp);
        assert_eq!(
            resolve_mode_from_target(RepairMode::Temp, false),
            Mode::Temp
        );
    }
}
