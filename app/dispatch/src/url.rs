//! Dispatcher URL values: host, region id, channel prefix and query version.
//!
//! The template itself comes from the unwrapped metadata; this module holds
//! only the per-biz constants and the tiny rewrite used to force the selected
//! host into a template that pins a different one.
//! The query string parameters are checked against those constants after
//! formatting, so a template change is a loud error rather than a silent
//! request to the wrong channel.

use crate::{DispatchError, Result};
use girpr_model::Region;

/// The global template from `metadata_summary.json`. The CN and Bilibili
/// templates are recorded in full as well; all three are used by the offline
/// tests as the fail-closed defaults when a template is missing.
pub const DEFAULT_URL_TEMPLATES: [(Region, &str); 3] = [
    (
        Region::Hk4eGlobal,
        "https://{0}.yuanshen.com/query_cur_region?version={1}&lang=1&platform=3&binary=1\
         &time=260&channel_id=1&sub_channel_id=1&account_type=1&dispatchSeed={2}&key_id=5",
    ),
    (
        Region::Hk4eCn,
        "https://cngfdispatch.yuanshen.com/query_cur_region?version={1}&lang=2&platform=3&binary=1\
         &time=561&channel_id=1&sub_channel_id=1&account_type=1&dispatchSeed={2}&key_id=4",
    ),
    (
        Region::Hk4eBilibili,
        "https://cnqddispatch.yuanshen.com/query_cur_region?version={1}&lang=2&platform=3&binary=1\
         &time=561&channel_id=14&sub_channel_id=0&account_type=0&dispatchSeed={2}&key_id=4",
    ),
];

/// Collapse `PresetConfig.cs:815-823` + `DispatchHelper.cs:173-194`: global's
/// region id is the registry default (USA = 0), CN is 4 and Bilibili is 5.
pub const fn region_id(biz: Region) -> u8 {
    match biz {
        Region::Hk4eGlobal => 0,
        Region::Hk4eCn => 4,
        Region::Hk4eBilibili => 5,
    }
}

/// `DispatchHelper.cs:43-46`: `CNRELWin` from region 4 upward.
pub const fn channel_prefix(biz: Region) -> &'static str {
    match biz {
        Region::Hk4eGlobal => "OSRELWin",
        Region::Hk4eCn | Region::Hk4eBilibili => "CNRELWin",
    }
}

/// The host pinned by each per-biz `GameDispatchURL` in the metadata.
pub const fn dispatch_host(biz: Region) -> &'static str {
    match biz {
        Region::Hk4eGlobal => "osusadispatch",
        Region::Hk4eCn => "cngfdispatch",
        Region::Hk4eBilibili => "cnqddispatch",
    }
}

/// `(ChannelID, SubChannelID)` from the preset; global has no fixed tuple, so
/// `(1, 0)` records its registry-default channel.
pub const fn preset_channel_tuple(biz: Region) -> (u8, u8) {
    match biz {
        Region::Hk4eGlobal => (1, 0),
        Region::Hk4eCn => (1, 1),
        Region::Hk4eBilibili => (14, 0),
    }
}

pub const fn channel_id(biz: Region) -> u8 {
    preset_channel_tuple(biz).0
}

pub const fn sub_channel_id(biz: Region) -> u8 {
    preset_channel_tuple(biz).1
}

pub const fn account_type(biz: Region) -> u8 {
    // Bili is the only channel that uses account type 0.
    match biz {
        Region::Hk4eBilibili => 0,
        Region::Hk4eGlobal | Region::Hk4eCn => 1,
    }
}

/// The `(channel_id, sub_channel_id, account_type)` that actually appear in
/// the query string. The global template pins `sub_channel_id=1` even though
/// its preset `SubChannelID` is 0 — they are different fields and must not be
/// conflated when validating the recorded URL.
pub const fn query_channel_tuple(biz: Region) -> (u8, u8, u8) {
    match biz {
        Region::Hk4eGlobal | Region::Hk4eCn => (1, 1, 1),
        Region::Hk4eBilibili => (14, 0, 0),
    }
}

/// The dispatcher query version is the channel prefix plus `meta.latest` —
/// e.g. `OSRELWin7.2.0`, never the pre-repair local version.
pub fn query_version(biz: Region, latest: &str) -> String {
    format!("{}{}", channel_prefix(biz), latest)
}

/// Substitute `{0}` host, `{1}` version and `{2}` seed in the metadata
/// template. A template that pins a different authority is rewritten to the
/// selected host.
pub fn build_dispatcher_url(
    template: &str,
    host: &str,
    version: &str,
    seed: &str,
) -> Result<String> {
    // CN/Bili templates pin the host outright (no `{0}`); global uses the
    // placeholder. Both are accepted, and both are rewritten to the selected
    // host below so a mismatched template cannot send the query elsewhere.
    let host_placeholders = template.matches("{0}").count();
    if host_placeholders > 1
        || template.matches("{1}").count() != 1
        || template.matches("{2}").count() != 1
    {
        return Err(DispatchError::Metadata(format!(
            "dispatcher URL template must carry {{1}} and {{2}} once and at most one {{0}}: {template:?}"
        )));
    }
    let expanded = template
        .replacen("{0}", host, 1)
        .replacen("{1}", version, 1)
        .replacen("{2}", seed, 1);
    let mut url = url::Url::parse(&expanded).map_err(|e| {
        DispatchError::Metadata(format!("dispatcher URL is not absolute: {expanded:?}: {e}"))
    })?;
    // Force the authority unconditionally: a template that pins a different
    // host, a port or user info must not redirect the query. This is the same
    // fail-closed rewrite the string version made, now owned by `Url`.
    url.set_host(Some(&format!("{host}.yuanshen.com")))
        .map_err(|_| DispatchError::Metadata(format!("invalid dispatcher host: {host:?}")))?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_port(None)
        .map_err(|_| DispatchError::Metadata(format!("invalid dispatcher port: {expanded:?}")))?;
    Ok(url.to_string())
}

/// Check the fixed query parameters against the per-biz constants. Called by
/// the query path so a template from a changed metadata blob cannot silently
/// switch `key_id` or `lang`.
pub fn validate_dispatcher_url(url: &str, biz: Region) -> Result<()> {
    let lang = if biz == Region::Hk4eGlobal { 1 } else { 2 };
    let time = if biz == Region::Hk4eGlobal { 260 } else { 561 };
    let key_id = if biz == Region::Hk4eGlobal { 5 } else { 4 };
    let (query_channel, query_sub_channel, query_account_type) = query_channel_tuple(biz);
    let expected = [
        ("lang", lang.to_string()),
        ("platform", "3".to_owned()),
        ("binary", "1".to_owned()),
        ("time", time.to_string()),
        ("channel_id", query_channel.to_string()),
        ("sub_channel_id", query_sub_channel.to_string()),
        ("account_type", query_account_type.to_string()),
        ("key_id", key_id.to_string()),
    ];
    let parsed = url::Url::parse(url).map_err(|e| {
        DispatchError::Metadata(format!("dispatcher URL is not absolute: {url:?}: {e}"))
    })?;
    let sent: Vec<_> = parsed.query_pairs().collect();
    for (key, value) in expected {
        if !sent
            .iter()
            .any(|(k, v)| k.as_ref() == key && v.as_ref() == value.as_str())
        {
            return Err(DispatchError::Metadata(format!(
                "dispatcher URL for {biz} is missing {key}={value:?}: {url:?}"
            )));
        }
    }
    Ok(())
}

pub fn template_for(biz: Region) -> &'static str {
    DEFAULT_URL_TEMPLATES
        .iter()
        .find(|(region, _)| *region == biz)
        .map(|(_, template)| *template)
        .unwrap_or(DEFAULT_URL_TEMPLATES[0].1)
}
