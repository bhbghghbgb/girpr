//! Path primitives: server-controlled path containment and atomic publishes.
//!
//! - [`path`] — `classify_metadata`, `MetadataFact`, `file_len`, `FileLen`,
//!   `contained_rel`, `clean_rel`, `contained_join`, `PathRoot`,
//!   `rel_from_base`, `rel_pair_from_base`, `join_rel`: one stat classification
//!   with thin views, lexical + filesystem checks and `/`-form rel spellings.
//! - [`atomic`] — `write_atomic`: temp-sibling-then-rename publishing for every
//!   artifact girpr writes.
//! - [`readonly`] — `clear_readonly` and `ReadOnlyOutcome`: the stat → check →
//!   clear → set_permissions sequence for one file, with the error policy left
//!   to the caller.
//!
//! The re-exports keep the shared names at the crate root. Nothing here names
//! a product data model: the crate holds only mechanics.
//!
//! The junction and symlink fixtures and the refusal seam live here too because
//! the containment tests are their only users besides the config.ini writer.
//! They are compiled out of every shipped target and only reach other crates'
//! tests through the `test-support` feature.

pub mod atomic;
pub mod path;
pub mod readonly;

// Unit-test-only junction fixture and classifier. Only this crate's tests and
// the tests that enable `test-support` reach it, so it stays out of shipped
// targets.
#[cfg(all(windows, any(test, feature = "test-support")))]
pub mod junction_support;
// Unit-test-only symlink fixture and classifier. `pub` so the config writer's
// tests and the product's tests can reach the same decision through the
// `test-support` feature.
#[cfg(any(test, feature = "test-support"))]
pub mod symlink_support;

pub use self::path::{
    FileLen, MetadataFact, PathRoot, classify_metadata, clean_rel, contained_join, file_len,
};

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
