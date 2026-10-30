//! The offline commands: `verify`, `materialize`, `list`.
//!
//! **M9.** No network, ever - these run against a bundle's stored bytes, and a
//! verifier that phoned home to check a bundle would be testing the network
//! rather than the bundle. This is the milestone that makes the artifact
//! trustworthy rather than merely present.
//!
//! ## What `verify` trusts, and in what order
//!
//! The blobs and the layout `chunks/index.json` describes. It does **not** trust
//! `selfcheck.json`, and the order it works in is the argument for that:
//!
//! 1. **A16** - re-hash every file, compare with `verify/checksums.json`. First
//!    because everything after this point assumes it is reading the bytes the
//!    capture wrote; a stale digest list would make every later verdict a
//!    verdict about a different tree.
//! 2. **A6** - re-measure every stored blob, compare with `chunks/index.json`.
//! 3. Rebuild both sides of every capture from the blobs alone (A11), check each
//!    rebuilt digest against the one the patchmap declared (A7/A8), and compare
//!    the rebuilt bytes with `game/` and `expected/` on disk (A18).
//! 4. Re-parse `game/config.ini` against the old tag (A13); re-count the matrix
//!    (A17).
//! 5. Only then: does the recomputation agree with what the capture recorded?
//!
//! Step 5 is last on purpose. A selfcheck that disagrees with a recomputation is
//! a finding *about the recording*, and a reader who met it before step 3 would
//! reasonably go looking for the wrong thing.
//!
//! ## A18, which is not in the spec
//!
//! The spec's seventeen assertions are all about a capture doing its job. None of
//! them asks whether `game/` is what the retained blobs say it is - which is the
//! bundle's central claim, and the one `materialize` exists to test. It gets an
//! id of its own so it cannot be mistaken for one of the seventeen, and so a
//! reader diffing two `selfcheck.json` files sees a row that only a verifier
//! could have written.
//!
//! ## What `verify` cannot re-derive
//!
//! A1-A5, A9, A10, A12 and A14-A15 are statements about the API bodies, the
//! manifests and the diff, and re-running them here would be re-running the
//! capture's own algorithm against the capture's own output. They are counted
//! and named in `covered_by_recording`, never silently dropped.
//!
//! A21-A23 are the bundle's **coverage** — whether it holds both sides of the
//! reuse, moved and size boundaries — and they are re-derived but never
//! *failed*: a bundle that does not cover the patch space is still internally
//! consistent, so it is reported as a finding and the bundle still verifies.

mod bundle;
mod checks;
mod list;
mod report;

pub mod materialize;

// Preserve the module's existing public paths.
pub use bundle::Bundle;
pub use list::{Index, Listed, Unreadable, list};
pub use report::{A18, DERIVED_UNRECORDED, REDERIVED, Report, derived_ids, reverify};

// Private names kept here so the test module (`use super::*`) can see them:
#[cfg(test)]
use crate::blobs::ChunkIndex;
#[cfg(test)]
use crate::hash;
#[cfg(test)]
use bundle::probe_manifest;
#[cfg(test)]
use checks::{A20, A20_ID, check_blobs, check_patchmap_arithmetic};
#[cfg(test)]
use list::{child_dirs, list_with};
#[cfg(test)]
use std::path::Path;

#[cfg(test)]
mod tests;
