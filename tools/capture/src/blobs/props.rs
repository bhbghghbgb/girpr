//! Blob properties: container sniff, digests, and the recorded hash properties.
//!
//! **M6.** Both digests are kept because both are *asserted* — the compressed MD5
//! is what the CDN's `ETag` claims, the decompressed one is what the manifest
//! claims, and a blob that passes only one of them is evidence of something.