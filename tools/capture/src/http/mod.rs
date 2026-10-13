//! The HTTP fetcher and the tag probe.
//!
//! **M1.** [`Fetcher`] owns retries, the timeout, the `User-Agent`, and the
//! request log that becomes `api/requests.ndjson` — the provenance record that
//! makes a captured bundle auditable rather than merely present. [`probe`]
//! answers "which tags does this API actually serve", which is the fact
//! `diff_tags` does not carry.