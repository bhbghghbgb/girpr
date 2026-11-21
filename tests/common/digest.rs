//! Thin shim over the library's shared digest spelling.
//!
//! Fixture-free targets include this file with `#[path]`; the implementation
//! lives in `girpr_test_fixtures` so it is compiled once for every test binary.

#[allow(unused_imports)]
pub use girpr_test_fixtures::md5hex;
