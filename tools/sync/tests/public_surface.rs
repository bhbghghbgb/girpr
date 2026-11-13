//! What a caller of the library can reach without a dependency of its own.
//!
//! The crate root re-exports the crates whose types appear in its own public
//! signatures, so a caller naming one of those types does not have to add the
//! dependency itself. These cases are integration tests on purpose: a unit
//! test inside the crate sees its own paths, and cannot show that an outside
//! caller reaches the same thing.

use girsync::{Filter, compile_patterns};

/// **A compiled pattern is reachable from the crate root.**
///
/// [`Filter::includes`] and [`Filter::excludes`] hand back `glob::Pattern`, so
/// the type is part of this crate's public surface. A caller that wants to
/// hold one — to read what the filter kept, or to match a path itself —
/// would otherwise have to depend on `glob` directly, and could get a
/// different version of it than the one the filter was built with.
#[test]
fn a_compiled_pattern_is_reachable_from_the_crate_root() {
    let patterns = compile_patterns(&["*.txt".to_string()]).unwrap();
    let pattern: &girsync::glob::Pattern = &patterns[0];
    assert!(pattern.matches("a.txt"), "the pattern the crate compiled");
    assert!(
        !pattern.matches("a.bin"),
        "and does not match what it should not"
    );

    // The filter's own patterns are the same type, and it hands them back.
    let filter = Filter::new(patterns, vec![], true);
    let included: &[girsync::glob::Pattern] = filter.includes();
    assert_eq!(included.len(), 1, "the filter keeps what it was given");
    assert!(
        included[0].matches("a.txt"),
        "and it is the pattern compiled"
    );
    let excluded: &[girsync::glob::Pattern] = filter.excludes();
    assert!(excluded.is_empty(), "and there are no excludes to read");
}

/// **A pattern can also be built through the re-export.**
///
/// The crate compiles its own patterns, and that is the way it spells the
/// rule — but a caller that already wants a `glob::Pattern` of its own is not
/// obliged to go through the crate's compiler, and the two must produce the
/// same type for a caller to mix them.
#[test]
fn a_pattern_can_be_built_through_the_re_export() {
    let built: girsync::glob::Pattern = girsync::glob::Pattern::new("*.txt").unwrap();
    assert!(built.matches("a.txt"), "and it matches what it says");
}
