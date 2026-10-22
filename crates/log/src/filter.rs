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
    use std::sync::{Arc, Mutex};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::{Context, SubscriberExt as _};
    use tracing_subscriber::registry::LookupSpan;

    /// Collects the targets of the events that reach it.
    #[derive(Clone, Default)]
    struct Targets(Arc<Mutex<Vec<&'static str>>>);

    impl Targets {
        fn seen(&self) -> Vec<&'static str> {
            self.0.lock().expect("the lock is not poisoned").clone()
        }
    }

    impl<S> Layer<S> for Targets
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            self.0
                .lock()
                .expect("the lock is not poisoned")
                .push(event.metadata().target());
        }
    }

    /// The targets of the events that `spec` lets through.
    fn targets_of(spec: &str) -> Vec<&'static str> {
        let targets = Targets::default();
        let subscriber =
            tracing_subscriber::registry().with(targets.clone().with_filter(EnvFilter::new(spec)));
        tracing::subscriber::with_default(subscriber, || {
            tracing::event!(target: "girpr::x", tracing::Level::DEBUG, "own");
            tracing::event!(target: "reqwest::x", tracing::Level::DEBUG, "other");
        });
        targets.seen()
    }

    #[test]
    fn a_valid_spec_is_used_as_it_is() {
        let f = filter("girpr=info", "girpr=error");
        assert!(f.to_string().contains("girpr=info"), "{f}");
    }

    #[test]
    fn a_broken_spec_falls_back_to_the_projects_own_level() {
        // What makes a spec unparseable is a *level* that does not exist. A
        // bare word is not that: `debug` is a global level directive, and any
        // other bare word is a target, which is good syntax too.
        let f = filter("girpr=bogus", "girpr=info");
        assert!(f.to_string().contains("girpr=info"), "{f}");
    }

    /// The console's spec is built as `"{rust_log},{project}={level}"`, and
    /// this pins the two `EnvFilter` rules that make that order mean what the
    /// policy says it means.
    ///
    /// They are upstream's rules, not ours, so they are worth a test: if
    /// `EnvFilter` ever changed them, the console would quietly stop obeying
    /// `--log-level`, and nothing else in the tree would notice.
    #[test]
    fn a_target_directive_beats_a_global_one() {
        // `girpr=info` is more specific than a bare `debug`, so the project's
        // own level stands and the global directive applies to the rest.
        assert_eq!(
            targets_of("girpr=info,debug"),
            vec!["reqwest::x"],
            "the project's own level is not widened by a global directive"
        );
        // The order in the spec does not change that.
        assert_eq!(targets_of("debug,girpr=info"), vec!["reqwest::x"]);
    }

    #[test]
    fn a_tie_on_the_same_target_goes_to_the_last_directive() {
        // Two directives for the same target: the later one wins. That is
        // why the policy puts the flag's own directive last.
        assert_eq!(targets_of("girpr=info,girpr=trace"), vec!["girpr::x"]);
        assert_eq!(targets_of("girpr=trace,girpr=info"), Vec::<&str>::new());
    }
}
