//! Shared region values for the `girpr` workspace.
//!
//! The repair binary and the capture tool address the same three regions and
//! send the same launcher ids, game ids, channel tuples and base URLs. Those are
//! values, not behaviour, so they live here once and both programs read them.
//!
//! ## Mechanics, not program logic
//!
//! This crate is a value table with `const` accessors. It holds no resolve
//! policy, no URL builder, no override seam and no validation: each program
//! keeps its own `Biz`/`resolve` wrapper, its own request construction and its
//! own test seam at its call site. The two things that make a program a program
//! — which region it accepts and how it builds a request — stay out of here.
//! The one byte table that spells query values is here too: it is a value both
//! programs send verbatim, not a request builder.
//!
//! The `clap` feature adds only the `ValueEnum` derive so a CLI-facing consumer
//! can use the same enum as its flag. Without that feature clap is not in the
//! dependency graph at all.

pub mod query;
pub mod region;

pub use region::{CHANNEL_ENDPOINT, DOWNLOAD_MODE_CHUNK, Host, Region};
