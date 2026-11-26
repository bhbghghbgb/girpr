//! Collapse dispatcher query, in one acyclic leaf.
//!
//! This crate owns the part of the persistent update that is unlike a HYP JSON
//! call: unwrapping the release-repo metadata into an RSA key and a versioned
//! seed, building `query_cur_region`, decrypting its base64/PKCS#1 v1.5 body,
//! decoding the protobuf and deriving the four CDN prefixes plus the three
//! manifest descriptors. It returns a [`DispatchInfo`] value; it does not fetch
//! the manifests, place files, write markers or touch the repair pipeline.
//!
//! ## Transport is injected
//!
//! Both network steps are traits: [`BytesFetcher`] supplies the metadata and
//! dispatcher URLs, [`BaseRevisionFetcher`] supplies the second GET that yields
//! `"<revision> <hash>"`. The caller decides which HTTP client to use, so this
//! leaf has no `reqwest`/`girpr-fetch` edge and every test is offline. The
//! metadata fetch is also digest-pinned: the raw `config_master.json` key blob
//! must hash to [`MASTER_KEY_BLOB_SHA256`], so an upstream format change fails
//! loudly instead of silently changing the key.
//!
//! ## Fail-closed version policy
//!
//! A version must appear in the embedded [`pinned_seed`] table before its seed
//! is accepted. An uncovered version is an error, not a fallback to an older
//! seed. The response's version is compared by *minor* number: the server
//! answers `7.2_live` to a `7.2.0` query, which is agreement, not a mismatch.

mod crypto;
mod dispatch;
mod metadata;
mod proto;
mod url;

#[cfg(test)]
mod tests;

pub use crypto::{decode_base64, decrypt_pkcs1_blocks};
pub use dispatch::{
    BaseRevisionFetcher, BytesFetcher, DispatchInfo, DispatchQuery, ManifestDescriptor,
    ManifestKind, check_minor_version, check_response_channel, minor_version, query_dispatch,
    query_dispatch_with_master_pin,
};
pub use metadata::{
    DispatcherKeys, MAGIC_SIGNATURE, MASTER_KEY_BLOB_LEN, MASTER_KEY_BLOB_SHA256, METADATA_BASE,
    config_file_from_stamp, load_dispatch_keys, load_dispatch_keys_with_pin, master_key_from_json,
    pinned_seed, rsa_key_from_xml, unwrap_magic_blob, unwrap_metadata_field,
};
pub use proto::{
    DescriptorRow, GatewayProperties, OpaqueFields, RepoResVersionProperties, WireValue,
    parse_base_revision, parse_descriptor, parse_dispatch_protobuf, walk_fields,
};

pub use url::{
    DEFAULT_URL_TEMPLATES, account_type, build_dispatcher_url, channel_id, channel_prefix,
    dispatch_host, query_version, region_id, sub_channel_id, template_for, validate_dispatcher_url,
};

use std::fmt;

/// Every failure here is a metadata failure at the product boundary: the
/// later caller maps this enum to `RunFailure::metadata` (exit 2) with a
/// `dispatcher query failed: …` line. Nothing here decides an exit code.
#[derive(Debug)]
pub enum DispatchError {
    /// Metadata fetch, unwrap, pin or version-coverage failure.
    Metadata(String),
    /// Transport failure on either injected GET.
    Transport(String),
    /// Base64 or RSA failure.
    Crypto(String),
    /// Protobuf or descriptor decode failure.
    Proto(String),
    /// The response describes a different minor version than we queried.
    Version { queried: String, response: String },
    /// The codename / channel tuple does not match the selected biz.
    Channel(String),
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Metadata(s) => write!(f, "metadata: {s}"),
            Self::Transport(s) => write!(f, "transport: {s}"),
            Self::Crypto(s) => write!(f, "crypto: {s}"),
            Self::Proto(s) => write!(f, "protobuf: {s}"),
            Self::Version { queried, response } => {
                write!(
                    f,
                    "version mismatch: queried {queried}, response {response}"
                )
            }
            Self::Channel(s) => write!(f, "channel: {s}"),
        }
    }
}

impl std::error::Error for DispatchError {}

pub type Result<T> = std::result::Result<T, DispatchError>;
