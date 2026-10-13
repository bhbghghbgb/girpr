//! Building a file out of chunks.
//!
//! **M7.** Three assemblies, and the third is the point: `old` from
//! `materialize_old`, `new` from `materialize_new`, and the *patch flow* —
//! REUSE a local slice after verifying it by MD5, FETCH otherwise — which must
//! then byte-equal the direct `new` assembly. The first two prove the artifact
//! can produce what it claims; only the third proves the patchmap describes a
//! patch anyone could run.