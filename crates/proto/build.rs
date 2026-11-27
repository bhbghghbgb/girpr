//! Generate the shared Sophon wire types from the vendored `.proto`.
//!
//! prost-build owns the tags, field names and scalar/message shapes; a vendored
//! protoc binary means neither Windows nor Linux needs a system protobuf. The
//! `serde::Serialize` type attribute makes the generated structs the same
//! full-decode dump surface the artifacts freeze — its key names are part of
//! the on-disk byte layout, not a program's report format.

// A build script has no caller to recover from a failed generation: returning
// the error fails the build with the same finality as a panic, without an
// `expect` in a path no user can reach at runtime.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;

    prost_build::Config::new()
        .protoc_executable(protoc)
        .type_attribute(".", "#[derive(serde::Serialize)]")
        .compile_protos(&["proto/Sophon.proto"], &["proto"])?;
    Ok(())
}
