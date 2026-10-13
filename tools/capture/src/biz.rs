//! Region identity: launcher ids, game ids, API bases, the channel tuple.
//!
//! **M1.** Every id this tool sends is *resolved* here rather than written into
//! a URL, because the authoritative source is `getGameBranches` and the thing
//! most worth recording is that the tool did not hardcode it.
//!
//! The measured global values — launcher `VYTpXlbWo8`, game `gopR6Cufr3`, cn
//! channel `1/1`, global `1/0`, bili `14/0` — live in [`Biz`] as the *fallback*
//! for the endpoints that must be addressed before `getGameBranches` can answer,
//! and `tests/biz.rs` pins them against the live shapes.