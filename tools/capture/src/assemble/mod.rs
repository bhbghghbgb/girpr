//! Building a file out of chunks.
//!
//! Three assemblies, and the third is the point: `old` from
//! `materialize_old`, `new` from `materialize_new`, and the *patch flow* -
//! REUSE a local slice after verifying it by MD5, FETCH otherwise - which must
//! then byte-equal the direct `new` assembly. The first two prove the artifact
//! can produce what it claims; only the third proves the patchmap describes a
//! patch anyone could run.

pub mod ranges;

mod assembly;
mod source;
mod types;

// Preserve the module's existing public paths.
pub use assembly::{AssemblyInput, MAX_ASSEMBLY_BYTES, assemble, patch_shape, tile};
pub use source::ChunkSource;
pub use types::{
    Assembled, Decision, Expectations, FileResult, PatchMap, PatchOutcome, PatchShape, PlanCounts,
    PlanEntry, Side, SourceRange, Totals,
};

// Private names kept here so the test module (`use super::*`) can see them:
#[cfg(test)]
use crate::blobs::chunks::FilePair;
#[cfg(test)]
use crate::select::Class;
#[cfg(test)]
use assembly::totals_for;
#[cfg(test)]
use ranges::{Layout, Range};
#[cfg(test)]
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;
