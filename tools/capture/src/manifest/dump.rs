//! Writing a decoded manifest out: the full JSON dump and its `paths.tsv` sidecar.
//!
//! **M3.** The gzip threshold is a decision this module owns because it is also
//! the module that writes the plain sidecar — the two are not independent
//! features, the sidecar exists *because* the dump may be compressed. The
//! decision is recorded in `MANIFEST.json → files[].encoding` so a reader never
//! has to infer it from the filename.