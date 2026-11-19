//! Building a `tracing` filter from a spec.
//!
//! One place, so that all three projects treat a broken `RUST_LOG` the same
//! way: it is a warning, and the run continues at the level the flag asked
//! for.

use std::str::FromStr;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::Directive;

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

/// Build a **console** filter from `RUST_LOG` and the directives that must
/// always be present.
///
/// `rust_log` is the environment's contribution, read once into the plan. It
/// is parsed on its own — never joined into a comma-separated spec with the
/// fixed directives — and each fixed directive is then appended with
/// [`EnvFilter::add_directive`]. Appending them last is what keeps "last wins":
/// `EnvFilter` gives a tie on the same target to the last directive, so the
/// project's own level (the one the flag chose) decides it even when `RUST_LOG`
/// names that target. Callers put the project's own directive last in `fixed`.
///
/// A broken `RUST_LOG` is warned about and dropped, leaving the fixed
/// directives in charge: a typo never leaves the console blind or widens it
/// past what was asked for. Unlike `parse_lossy`, a bad directive is never
/// silently ignored — the whole spec is rejected and reported.
pub fn console_filter(rust_log: Option<&str>, fixed: &[&str]) -> EnvFilter {
    let mut filter = match rust_log {
        None => EnvFilter::default(),
        Some(spec) => match EnvFilter::try_new(spec) {
            Ok(f) => f,
            Err(e) => {
                eprintln!(
                    "WARN ignoring the log filter {spec:?}: {e}; using {:?}",
                    fixed.join(",")
                );
                EnvFilter::default()
            }
        },
    };
    for directive in fixed {
        match Directive::from_str(directive) {
            Ok(d) => filter = filter.add_directive(d),
            Err(e) => eprintln!("WARN ignoring the fixed log directive {directive:?}: {e}"),
        }
    }
    filter
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

    /// The targets of the events that `filter` lets through.
    fn targets_of_filter(filter: EnvFilter) -> Vec<&'static str> {
        let targets = Targets::default();
        let subscriber = tracing_subscriber::registry().with(targets.clone().with_filter(filter));
        tracing::subscriber::with_default(subscriber, || {
            tracing::event!(target: "girpr::x", tracing::Level::DEBUG, "own");
            tracing::event!(target: "reqwest::x", tracing::Level::DEBUG, "other");
        });
        targets.seen()
    }

    /// The targets of the events that `spec` lets through.
    fn targets_of(spec: &str) -> Vec<&'static str> {
        targets_of_filter(EnvFilter::new(spec))
    }

    #[test]
    fn a_valid_spec_is_used_as_it_is_and_a_broken_one_falls_back() {
        for (spec, fallback, want) in [
            // A spec that parses is used as it is.
            ("girpr=info", "girpr=error", "girpr=info"),
            // A broken spec falls back to the project's own level. What makes
            // a spec unparseable is a *level* that does not exist. A bare word
            // is not that: `debug` is a global level directive, and any other
            // bare word is a target, which is good syntax too.
            ("girpr=bogus", "girpr=info", "girpr=info"),
        ] {
            let f = filter(spec, fallback);
            assert!(f.to_string().contains(want), "{spec}: {f}");
        }
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

    /// `console_filter` appends the project's own directive after `RUST_LOG`,
    /// so the flag wins a tie, and a broken spec falls back to just those
    /// fixed directives.
    #[test]
    fn console_filter_appends_the_own_directive_last() {
        // `RUST_LOG` names the project, but the appended directive wins the tie.
        assert_eq!(
            console_filter(Some("girpr=error"), &["girpr=trace"]).to_string(),
            "girpr=trace"
        );
        // A broken spec and an unset one both leave just the fixed directives.
        assert_eq!(
            console_filter(Some("girpr=bogus"), &["girpr=info"]).to_string(),
            "girpr=info"
        );
        assert_eq!(
            console_filter(None, &["girpr=info"]).to_string(),
            "girpr=info"
        );
        // A spec naming another crate is kept alongside the project's own.
        assert_eq!(
            console_filter(Some("reqwest=debug"), &["girpr=info"]).to_string(),
            "reqwest=debug,girpr=info"
        );
        // A bare global `debug` still cannot widen the project: a more
        // specific target directive wins, so only the other crate shows up.
        assert_eq!(
            targets_of_filter(console_filter(Some("debug"), &["girpr=info"])),
            vec!["reqwest::x"]
        );
        // The appended directive wins a tie on the same target ("last wins").
        assert_eq!(
            targets_of_filter(console_filter(Some("girpr=error"), &["girpr=trace"])),
            vec!["girpr::x"]
        );
    }
}
