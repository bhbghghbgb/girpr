//! Building a `tracing` filter from a spec.
//!
//! One place, so that all three projects treat a broken `RUST_LOG` the same
//! way: it is a warning, and the run continues at the level the flag asked
//! for.

use tracing_subscriber::EnvFilter;

/// Build a filter from `spec`, falling back to `fallback` when it does not
/// parse.
///
/// A `RUST_LOG` the user got wrong is their typo, not a reason to run blind:
/// the fallback is the project's own level, which is what the flag asked for
/// before the environment had its say. The typo is reported, because a filter
/// that silently became something else is a filter the user does not know
/// about.
pub fn filter(spec: &str, fallback: &str) -> EnvFilter {
    match EnvFilter::try_new(spec) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("WARN ignoring the log filter {spec:?}: {e}; using {fallback:?}");
            EnvFilter::new(fallback)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_spec_is_used_as_it_is() {
        let f = filter("girpr=info", "girpr=error");
        assert!(f.to_string().contains("girpr=info"), "{f}");
    }

    #[test]
    fn a_broken_spec_falls_back_to_the_projects_own_level() {
        // A *level* that does not exist is what makes a spec unparseable — a
        // bare word is read as a target, and is perfectly good syntax.
        let f = filter("girpr=bogus", "girpr=info");
        assert!(f.to_string().contains("girpr=info"), "{f}");
    }
}
