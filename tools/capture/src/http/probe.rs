//! `getBuild?tag=` probing: which version tags the API will actually serve.
//!
//! **M1.** `diff_tags` is a patch-support list, not a retention list, and the
//! difference is what decides whether an old-version file is reconstructible at
//! all. The result is `api/tag_probe.json`, and `served` is the load-bearing
//! field in it.