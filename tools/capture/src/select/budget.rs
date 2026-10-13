//! Smallest-first admission under `--max-download-bytes`.
//!
//! **M5.** A deliberate exclusion is not a failure: an over-budget class is
//! recorded and the run still exits `0`. `--require-class` is what turns an
//! absent class into exit `4`, so "I asked for this and could not have it" is a
//! thing the caller said rather than something the tool decided.