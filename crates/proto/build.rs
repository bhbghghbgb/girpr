//! Generate the shared Sophon wire types from the vendored `.proto`.
//!
//! prost-build owns the tags, field names and scalar/message shapes; a vendored
//! protoc binary means neither Windows nor Linux needs a system protobuf. The
//! `serde::Serialize` type attribute makes the generated structs the same
//! full-decode dump surface the artifacts freeze — its key names are part of
//! the on-disk byte layout, not a program's report format.

fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc for this host");

    prost_build::Config::new()
        .protoc_executable(protoc)
        .type_attribute(".", "#[derive(serde::Serialize)]")
        .compile_protos(&["proto/Sophon.proto"], &["proto"])
        .expect("compile vendored Sophon.proto");
}
