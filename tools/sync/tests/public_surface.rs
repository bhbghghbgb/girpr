//! What a caller of the library can reach without a dependency of its own.
//!
//! The crate root re-exports the crates whose types appear in its own public
//! signatures, so a caller naming one of those types does not have to add the
//! dependency itself. A case here can also pin a rule the crate publishes
//! itself: the reachability of the rule is the thing being pinned, and only an
//! outside caller can show it. These cases are integration tests on purpose: a
//! unit test inside the crate sees its own paths, and cannot show that an
//! outside caller reaches the same thing.

use girsync::{Filter, compile_patterns};

/// **A compiled pattern is reachable from the crate root.**
///
/// [`Filter::includes`] and [`Filter::excludes`] hand back `Compiled`, so the
/// type is part of this crate's public surface. A caller that wants to hold
/// one — to read what the filter kept, or to match a path itself — would
/// otherwise have to depend on `girpr-filter` directly, and could get a
/// different version of it than the one the filter was built with.
#[test]
fn a_compiled_pattern_is_reachable_from_the_crate_root() {
    let patterns = compile_patterns(&["*.txt".to_string()]).unwrap();
    let pattern: &girsync::Compiled = &patterns[0];
    assert!(
        pattern.matches("a.txt", true),
        "the pattern the crate compiled"
    );
    assert!(
        !pattern.matches("a.bin", true),
        "and does not match what it should not"
    );

    // The filter's own patterns are the same type, and it hands them back.
    let filter = Filter::new(patterns, vec![], true);
    let included: &[girsync::Compiled] = filter.includes();
    assert_eq!(included.len(), 1, "the filter keeps what it was given");
    assert!(
        included[0].matches("a.txt", true),
        "and it is the pattern compiled"
    );
    let excluded: &[girsync::Compiled] = filter.excludes();
    assert!(excluded.is_empty(), "and there are no excludes to read");
}

/// **A glob can also be built through the re-export.**
///
/// The crate compiles its own globs, and that is the way it spells the rule —
/// but a caller that already wants a `globset::Glob` of its own is not obliged
/// to go through the crate's compiler. The re-export puts it on the same
/// version the filter uses, so a caller can ask the same question.
#[test]
fn a_pattern_can_be_built_through_the_re_export() {
    let built: girsync::globset::Glob = girsync::globset::Glob::new("*.txt").unwrap();
    assert!(
        built.compile_matcher().is_match("a.txt"),
        "and it matches what it says"
    );
}

/// **The path key is reachable from outside the crate.**
///
/// The pairing a run reports is `CasePairing::Lowercase`, which pairs two
/// sides by the path key, so the key is part of the answer the crate gives and
/// the rule behind it is published with it: a caller that reads that pairing
/// has to be able to ask the same question the same way, and the ASCII-only
/// fold is the half of the rule that is easy to get wrong.
///
/// The key, the borrowed form of it and the index over them are all reached
/// here, and the fold is the one the crate uses — ASCII case only, so a
/// non-ASCII case variant stays a distinct key.
#[test]
fn the_path_key_and_its_index_are_reachable_from_outside_the_crate() {
    use std::collections::HashMap;

    // The key itself: ASCII case folded, every other byte exact.
    assert_eq!(
        girsync::util::path_key("Game/Readme.TXT"),
        "game/readme.txt"
    );
    assert_ne!(
        girsync::util::path_key("Ärger"),
        girsync::util::path_key("ärger"),
        "the key folds ASCII case only, so a non-ASCII variant is a distinct key"
    );

    // The index over it: every key under its folded spelling, with the key it
    // was built from and the value the map holds under it.
    let map: HashMap<String, u32> = [("Readme.TXT".to_string(), 7)].into_iter().collect();
    let index = girsync::util::folded_keys(&map);
    let (key, value) = *index
        .get(&girsync::util::path_key("readme.txt"))
        .expect("the folded spelling finds the key");
    assert_eq!(key.as_str(), "Readme.TXT");
    assert_eq!(*value, 7);

    // The borrowed form of the same rule, which finds a key under any
    // spelling of the same path.
    let mut borrowed: HashMap<girsync::util::FoldedKey<'_>, &str> = HashMap::new();
    borrowed.insert(girsync::util::FoldedKey("Readme.TXT"), "Readme.TXT");
    assert_eq!(
        borrowed.get(&girsync::util::FoldedKey("readme.txt")),
        Some(&"Readme.TXT"),
        "the case variant is the same key"
    );
}
