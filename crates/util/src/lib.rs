//! Utility primitives: the scan/MD5 primitive, server-controlled path
//! containment, atomic publishes, and the launcher's config.ini files.
//!
//! - [`md5`] — one read, every answer: `scan_file`, `scan_reader`,
//!   `read_range`, `md5_file`, `md5_file_slice`, plus `Scan`, `Md5` and
//!   `EMPTY_MD5`.
//! - [`path`] — `contained_rel`, `clean_rel`, `contained_join`, `PathRoot`,
//!   `file_len`, `FileLen`, `rel_from_base`, `rel_pair_from_base`, `join_rel`:
//!   lexical + filesystem checks and `/`-form rel spellings.
//! - [`atomic`] — `write_atomic`: temp-sibling-then-rename publishing for every
//!   artifact girpr writes.
//! - [`counter`] — `add_saturating`: the saturating `try_update` shared by the
//!   atomic byte counters, so a charge cannot wrap.
//! - [`ini`] — `read_game_version`, `read_ignore_categories`,
//!   `read_blacklist`, `write_config_ini`: the launcher's config.ini files.
//!
//! The re-exports below keep the shared names at the crate root. The MD5 file
//! wrappers stayed in [`md5`] rather than moving to the root: `md5_file` and
//! `md5_file_slice` are reached as [`md5::md5_file`] and
//! `md5::md5_file_slice` (the latter is a test-only helper).
//!
//! Nothing here names a product data model: the crate holds only mechanics.

pub mod atomic;
pub mod counter;
pub mod ini;
pub mod md5;
pub mod path;

// Unit-test-only junction fixture and classifier shared by `ini` and `path`.
// Only this crate's unit tests reach it, so it stays compiled out of every
// shipped target and out of the product's dependency build.
#[cfg(all(windows, test))]
pub(crate) mod junction_support;
// Unit-test-only fixture and classifier for a refused directory symlink. It
// is a module of its own because the junction support above is declared for
// Windows only, and this rule runs on every host. `pub` so the product's tests
// can ask it for a symlink and for the shared decision through the
// `test-support` feature.
#[cfg(any(test, feature = "test-support"))]
pub mod symlink_support;

pub use self::ini::{read_blacklist, read_game_version, read_ignore_categories, write_config_ini};
pub use self::md5::{EMPTY_MD5, Md5, Scan, ScanRange, read_range, scan_file, scan_reader};
pub use self::path::{FileLen, PathRoot, clean_rel, contained_join, file_len};

/// Test-only helpers shared with the product's test seam.
///
/// `FixtureRefusal` and `skip_fixture_refusal` live here so the unit-test-only
/// junction and symlink fixtures can report a host refusal the same way the
/// product's own tests do. They are re-exported through the product's
/// `test_support` seam, so the two sides name one type.
#[doc(hidden)]
pub mod test_support {
    /// Where a refusal to create a host-dependent test fixture came from.
    ///
    /// A host that cannot create one and a test that injected the failure both
    /// take the skip branch, but only the first is a skipped case. The printed
    /// line says which, and only the real skip keeps the `SKIP precondition:`
    /// wording the runner is configured to surface.
    #[derive(Clone, Copy)]
    pub enum FixtureRefusal {
        /// The host refused the fixture.
        Host,
        /// The test supplied the refusal; no host behaviour was observed.
        Injected,
    }

    impl FixtureRefusal {
        fn lead(self, subject: &str) -> String {
            match self {
                Self::Host => format!("SKIP precondition: this host cannot create {subject}"),
                Self::Injected => format!(
                    "simulated refusal: no host was consulted; a host that cannot create \
                     {subject} would skip"
                ),
            }
        }

        /// What the required-case failure says about where the refusal came
        /// from. An injected refusal never consulted the host, so blaming the
        /// host there would name a cause the test did not observe.
        fn failure_lead(self, subject: &str) -> String {
            match self {
                Self::Host => format!("this host cannot create {subject}"),
                Self::Injected => {
                    format!("the refusal to create {subject} was injected; no host was consulted")
                }
            }
        }
    }

    /// Report that a host-dependent fixture could not be created, or fail when
    /// `required_env` says the case must not be skipped.
    ///
    /// `subject` names the fixture (`a directory junction`, `a non-UTF-8
    /// name`) and `detail` is the reason the attempt failed. A run meant to
    /// prove the fixture must not pass having checked nothing, so the
    /// presence-based switch named by `required_env` turns the skip into a
    /// failure. Only a host refusal blames the host: a required injected
    /// refusal fails saying the refusal was injected and no host was
    /// consulted.
    pub fn skip_fixture_refusal(
        refusal: FixtureRefusal,
        subject: &str,
        required_env: &str,
        detail: impl std::fmt::Display,
        required: bool,
    ) {
        assert!(
            !required,
            "{required_env} is set, but {}: {detail}",
            refusal.failure_lead(subject)
        );
        eprintln!(
            "{}: {detail}; set {required_env} (to any value) to fail instead",
            refusal.lead(subject)
        );
    }
}
