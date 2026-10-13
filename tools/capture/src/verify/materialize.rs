//! Rebuilding both files from `chunks/`, with no network.
//!
//! **M9.** This is the closure check: if the retained blobs cannot reproduce
//! `game/` and `expected/` byte-for-byte, then the bundle's claim that it is
//! self-contained is false, and everything downstream of that claim is void.