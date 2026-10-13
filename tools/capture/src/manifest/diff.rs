//! The path diff, the reuse pool, and the `getBuild.stats` reconciliation.
//!
//! **M4.** This is the module that makes the artifact a *measurement* rather
//! than a dump: counts over the common path set, the `same_md5` present anywhere
//! in the old manifest versus the `same_md5` in the same file, and the number of
//! bytes that same-file-only reuse forgoes. The last one records the cost of a
//! deliberate simplification in the tool under test; nothing here acts on it.