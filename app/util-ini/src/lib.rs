//! Ini primitives: the launcher's config.ini files.
//!
//! - [`ini`] — `read_game_version`, `read_ignore_categories`,
//!   `read_blacklist`, `write_config_ini`: the launcher's config.ini files.
//!
//! The re-exports keep the shared names at the crate root. Nothing here names
//! a product data model: the crate holds only mechanics.

pub mod ini;

pub use self::ini::{read_blacklist, read_game_version, read_ignore_categories, write_config_ini};
