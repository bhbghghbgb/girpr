//! One-shot `sled` -> `redb` cache converter, extracted from `girsync`.
//!
//! The converter used to live in `girsync` behind a `convert` feature, which
//! put `sled` in girsync's dependency graph. It is now its own tool crate so
//! that the `girsync` package and binary stay completely free of `sled`.
//!
//! The conversion itself is unchanged. It reads a legacy `<root>/girpr-cache`
//! sled directory (JSON values, hex digests under the `\0meta` reservation)
//! and writes a new redb file through girsync's **public** cache API; hex
//! digests are decoded to raw bytes and unknown algorithms pass through
//! untouched. The only thing that moved is the crate boundary: `crate::cache`,
//! `crate::hash` and `crate::util` paths became `girsync::…` paths.
//!
//! ## The binary's arguments and its one conversion record
//!
//! [`Args`] is the binary's clap definition, and [`conversion_record`] plus
//! [`conversion_output`] are the seam around the one line it reports. They live
//! here rather than in `main.rs` so a test can parse a spelling and render the
//! record in-process, instead of spawning the binary to read stdout. The binary
//! still emits through girsync's shared writer, so `--output json` means the
//! same thing in both binaries and there is one definition of a conversion
//! record rather than a `println!` per tool.
//!
//! There is no `--version`: the command declares no version, so the flag is an
//! unknown one and the run fails like any other usage error. The error path in
//! `main` already treats a version request as an answer rather than a failure,
//! so the flag would exit `0` the moment one was declared.
//!
//! The argument struct carries no doc comment, on purpose. `clap` reads one as
//! the command's `long_about`, so `--help` would print it verbatim — the `**`
//! markers and the `#[command(version)]` attribute included, because `clap`
//! does not render markdown. A note written for a source reader would then
//! tell a user about an attribute they cannot type. The notes above are for
//! source readers, so they live here.

use girsync::report::{OutputFormat, Record, Report};
use std::path::{Path, PathBuf};

pub mod convert;

// Deliberately no doc comment on `Args`: clap reads one as the command's
// `long_about` and would print it verbatim in `--help` (see the module notes).
#[derive(clap::Parser, Debug)]
#[command(
    name = "sled2redb",
    about = "Convert a legacy sled cache directory into a redb cache file"
)]
pub struct Args {
    /// Legacy sled cache directory.
    pub sled_dir: PathBuf,
    /// Destination redb file.
    pub redb_path: PathBuf,
    /// Replace an existing destination.
    #[arg(long)]
    pub force: bool,
    /// How the conversion record is rendered.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

/// The `converted` record, as a pure function of the counts and the two paths.
///
/// This is the seam the tests reach without spawning: the binary prints this
/// record through girsync's shared writer, and the pure render below reads the
/// same record, so the two cannot drift apart.
pub fn conversion_record(c: &convert::Converted, src: &Path, dst: &Path) -> Record {
    Record::keyed("converted")
        .put("src", src.display().to_string())
        .put("dst", dst.display().to_string())
        .put("files", c.files)
        .put("dirs", c.dirs)
        .put("hashes", c.hashes)
}

/// The line the binary prints for a conversion, without the trailing newline.
///
/// A thin wrapper over [`Report::render`]: given the counts, the two paths and
/// an [`OutputFormat`], the rendered text is a pure function of those inputs.
/// Tests assert this directly where the old suite spawned a process to read
/// stdout; the binary emits the very same [`Record`] through the shared writer.
pub fn conversion_output(
    c: &convert::Converted,
    src: &Path,
    dst: &Path,
    format: OutputFormat,
) -> String {
    Report::new(format).render(&conversion_record(c, src, dst))
}
