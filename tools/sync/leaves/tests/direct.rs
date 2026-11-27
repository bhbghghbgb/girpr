//! A consumer that reaches the leaves without going through girsync's lib.
//!
//! This is the §A2 edge made concrete: the integration test depends on this
//! crate directly, so an edit to girsync's planner, commands or other high-level
//! modules leaves this target cached. It exercises the public surface each
//! leaf exposes, with no girsync module in the path.

use std::num::NonZeroUsize;

use girsync_leaves::filter::Filter;
use girsync_leaves::hardlink;
use girsync_leaves::heartbeat::Heartbeat;
use girsync_leaves::link;
use girsync_leaves::rw::{RwLimiter, RwLimits, RwRuntime, RwSide};

#[test]
fn the_filter_rule_is_reachable_without_girsync() {
    let patterns = girsync_leaves::filter::compile_patterns(&["link".to_string()]).unwrap();
    let filter = Filter::new(vec![], patterns, true);
    assert!(filter.is_excluded("link"));
    assert!(
        filter.is_excluded("link/x.txt"),
        "an excluded dir is a subtree"
    );
    assert!(
        !filter.is_excluded("linkage/x.txt"),
        "a prefix is not a subtree"
    );
}

#[test]
fn the_link_and_hardlink_facts_are_reachable_without_girsync() {
    let root = girpr_tempdir::TempDir::new("girsync_leaves", "direct");
    let file = root.join("a.txt");
    std::fs::write(&file, b"x").unwrap();

    assert!(!link::try_is_link(&file).unwrap());
    assert_eq!(link::topmost_link_ancestor(&root, "a.txt").unwrap(), None);
    assert_eq!(hardlink::count(&file).unwrap(), 1);
    assert!(hardlink::info(&file).unwrap().regular_file);
}

#[test]
fn the_heartbeat_trigger_is_reachable_without_girsync() {
    let mut heartbeat = Heartbeat::new(|count, _| count % 2 == 0);
    assert!(heartbeat.tick(1).is_none());
    assert!(heartbeat.tick(2).is_some());
}

#[test]
fn the_rw_gate_is_reachable_without_girsync() {
    assert_eq!(RwLimits::default().pool_size(), 1);
    let limiter = RwLimiter::new(RwLimits::Shared(NonZeroUsize::new(1).unwrap()));
    let held = limiter.acquire(RwSide::Src);
    assert!(
        limiter.try_acquire(RwSide::Dst).is_none(),
        "one shared counter"
    );
    drop(held);
    assert!(limiter.try_acquire(RwSide::Dst).is_some());

    let runtime = RwRuntime::serial().unwrap();
    assert_eq!(runtime.limits(), RwLimits::default());
}
