//! Writing the bundle.
//!
//! **M8.** Everything temporary goes to `tmp/` and is moved into place, so an
//! interrupted run never leaves a half-written document that reads as a
//! complete one. `verify/checksums.json` is what proves nothing is missing.