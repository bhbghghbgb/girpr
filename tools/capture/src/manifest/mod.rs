//! Manifests: fetch → zstd → verify → decode → dump → diff.
//!
//! **M3** brings [`proto`] (the wire types, opaque field 6 preserved verbatim),
//! [`dump`] (the full decode plus its `paths.tsv` sidecar) and this facade, which
//! is the fetch-and-verify pipeline. **M4** adds [`diff`]: the path diff, the
//! reuse-pool measurement, and the reconciliation against `getBuild.stats`.
//!
//! Verification is not optional here. A blob that decodes to something is not
//! evidence that it decodes to the *right* something: the decompressed MD5 is
//! checked against `checksum`, and `checksum` against the manifest id's 32-hex
//! suffix. The id's 16-hex prefix is recorded as `opaque` and never checked —
//! it is not a hash of anything observable.

pub mod diff;
pub mod dump;
pub mod proto;