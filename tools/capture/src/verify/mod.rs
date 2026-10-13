//! The offline commands: `verify`, `materialize`, `list`.
//!
//! **M9.** No network, ever — these run against a bundle's stored bytes, and a
//! verifier that phoned home to check a bundle would be testing the network
//! rather than the bundle. This is the milestone that makes the artifact
//! trustworthy rather than merely present.

pub mod materialize;