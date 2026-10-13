//! Writing the bundle.
//!
//! **M1** lands [`write_requests_log`]: the one file whose whole job is
//! provenance. **M8** adds the rest of the tree — `game/`, `expected/`,
//! `MANIFEST.json`, `checksums.json`, `README.md` — in the submodules below.
//!
//! Everything temporary goes to `tmp/` and is moved into place, so an interrupted
//! run never leaves a half-written document that reads as a complete one.
//! `verify/checksums.json` is what proves nothing is missing.

use std::path::Path;

use crate::http::RequestLog;

/// Write `api/requests.ndjson` inside the bundle at `root`.
///
/// Returns how many requests were recorded, which is zero for an empty log — a
/// file that exists but is empty is the honest shape for a run that made no
/// requests, and it is distinguishable from a run that never got far enough to
/// write one.
pub fn write_requests_log(root: &Path, log: &RequestLog) -> std::io::Result<usize> {
    log.write_ndjson(&root.join("api").join("requests.ndjson"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_lands_at_api_requests_ndjson_and_creates_its_directory() {
        let dir = std::env::temp_dir().join(format!("gircap-emit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("hk4e_global").join("7.0.0__7.1.0").join("20261014T101500Z");

        let log = RequestLog::new();
        assert_eq!(write_requests_log(&root, &log).unwrap(), 0);
        assert!(root.join("api").join("requests.ndjson").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}