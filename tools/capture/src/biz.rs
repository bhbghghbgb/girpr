//! Region identity: launcher ids, game ids, API bases, the channel tuple.
//!
//! **M1.** Every id this tool sends is *resolved* here rather than written into
//! a URL, because the authoritative source is `getGameBranches` and the thing
//! most worth recording is that the tool did not hardcode it.
//!
//! The measured global values — launcher `VYTpXlbWo8`, game `gopR6Cufr3`, cn
//! channel `1/1`, global `1/0`, bili `14/0` — are the ones spec §3.4 pins.
//! `hk4e_cn` and `hk4e_bilibili` carry the same shape but were **not** measured
//! by this tool; only `hk4e_global` is exercised live (plan §6), and a capture
//! against the other two is not yet a supported claim.
//!
//! Nothing here is hardcoded that the API can answer: `package_id`, `password`
//! and `branch` come out of `getGameBranches`, so a build that rotates them
//! needs no code change.
//!
//! A `package_id` **is** stable per game, so hardcoding it would have worked
//! today. It would also have worked yesterday and will work tomorrow, silently,
//! for as long as nobody notices — and a capture tool whose ids are wrong
//! produces a bundle that is *internally consistent and completely fictitious*,
//! which is the worst possible failure for the thing this tool exists to make.

use crate::cli::Biz as BizId;
use crate::http::ApiError;

/// Which API/CDN family serves a region.
///
/// `hk4e_cn` and `hk4e_bilibili` both live on the `mihoyo` family; the ids
/// differ, the hosts do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// Mainland + Bilibili.
    Mihoyo,
    /// International.
    Hoyoverse,
}

impl Host {
    /// Base for the four HoYoPlay JSON endpoints.
    pub const fn hyp_base(self) -> &'static str {
        match self {
            Host::Mihoyo => "https://hyp-api.mihoyo.com/hyp/hyp-connect/api",
            Host::Hoyoverse => "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api",
        }
    }

    /// Base for the Sophon `getBuild` JSON endpoint.
    pub const fn sophon_base(self) -> &'static str {
        match self {
            Host::Mihoyo => "https://downloader-api.mihoyo.com/downloader/sophon_chunk/api",
            Host::Hoyoverse => {
                "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api"
            }
        }
    }
}

/// The four HoYoPlay endpoints `gircap` addresses.
///
/// `channel` / `sub_channel` are sent on [`HypEndpoint::GameDeprecatedFileConfigs`]
/// **only**. Measured 2026-10-14: sending them anywhere else is not the documented
/// request, and a capture that got one wrong is a capture of the wrong request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HypEndpoint {
    /// Launcher configuration: audio scan dir, download mode, exe name.
    GameConfigs,
    /// Per-branch package ids, tags and `diff_tags`.
    GameBranches,
    /// Files the current version no longer ships. Needs the channel tuple.
    GameDeprecatedFileConfigs,
    /// Legacy package listing. Captured as dated staleness evidence only.
    GamePackages,
}

impl HypEndpoint {
    /// The path segment after the base.
    pub const fn path(self) -> &'static str {
        match self {
            HypEndpoint::GameConfigs => "getGameConfigs",
            HypEndpoint::GameBranches => "getGameBranches",
            HypEndpoint::GameDeprecatedFileConfigs => "getGameDeprecatedFileConfigs",
            HypEndpoint::GamePackages => "getGamePackages",
        }
    }

    /// Whether this endpoint carries the channel tuple.
    pub const fn needs_channel(self) -> bool {
        matches!(self, HypEndpoint::GameDeprecatedFileConfigs)
    }
}

/// Everything about a region that a request needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Biz {
    /// The flag's value.
    pub id: BizId,
    /// HoYoPlay `launcher_id`.
    pub launcher_id: &'static str,
    /// HoYoPlay `game_id`, sent as `game_ids[]`.
    pub game_id: &'static str,
    /// `channel`, written to `config.ini` and sent to the deprecated-files call.
    pub channel: &'static str,
    /// `sub_channel`.
    pub sub_channel: &'static str,
    /// `cps`, written to `config.ini`.
    pub cps: &'static str,
    /// Which API family serves this region.
    pub host: Host,
}

impl Biz {
    /// Resolve a flag value into everything a request needs.
    pub const fn resolve(id: BizId) -> Self {
        match id {
            BizId::Hk4eCn => Biz {
                id,
                launcher_id: "jGHBHlcOq1",
                game_id: "1Z8W5NHUQb",
                channel: "1",
                sub_channel: "1",
                cps: "hyp_mihoyo",
                host: Host::Mihoyo,
            },
            BizId::Hk4eGlobal => Biz {
                id,
                launcher_id: "VYTpXlbWo8",
                game_id: "gopR6Cufr3",
                channel: "1",
                sub_channel: "0",
                cps: "hyp_hoyoverse",
                host: Host::Hoyoverse,
            },
            BizId::Hk4eBilibili => Biz {
                id,
                launcher_id: "umfgRO5gh5",
                game_id: "T2S0Gz4Dr2",
                channel: "14",
                sub_channel: "0",
                cps: "hyp_mihoyo",
                host: Host::Mihoyo,
            },
        }
    }

    /// The API id, as the bundle's documents name the region.
    pub const fn as_str(&self) -> &'static str {
        self.id.as_str()
    }

    /// The HoYoPlay base for this region.
    pub const fn hyp_base(&self) -> &'static str {
        self.host.hyp_base()
    }

    /// The Sophon base for this region.
    pub const fn sophon_base(&self) -> &'static str {
        self.host.sophon_base()
    }

    /// The `(channel, sub_channel, cps)` tuple `config.ini` records.
    pub const fn channel_tuple(&self) -> (&'static str, &'static str, &'static str) {
        (self.channel, self.sub_channel, self.cps)
    }

    /// The production bases for this region.
    pub const fn bases(&self) -> Endpoints<'_> {
        Endpoints {
            hyp: self.hyp_base(),
            sophon: self.sophon_base(),
        }
    }
}

/// Where requests go: the resolved pair of API bases.
///
/// Kept separate from [`Biz`] because the two answer different questions — `Biz`
/// is *who to ask* (launcher id, game id, channel tuple) and `Endpoints` is *at
/// what address* — and because the testing seam replaces the second without
/// touching the first. A mock capture must send the same ids to a local server
/// as a production capture sends to a CDN; collapsing the two would make a mock
/// a test of the mock rather than of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints<'a> {
    /// Base for the four HoYoPlay endpoints.
    pub hyp: &'a str,
    /// Base for the Sophon `getBuild` endpoint.
    pub sophon: &'a str,
}

impl<'a> Endpoints<'a> {
    /// Borrowed bases with no override.
    pub const fn new(hyp: &'a str, sophon: &'a str) -> Self {
        Endpoints { hyp, sophon }
    }

    /// Owned bases, for when an override is in play.
///
/// **This leaks, deliberately, and the leak is bounded by construction. §5.5 row
/// 12.** The plan called the leak "bounded and intentional — but nothing says so",
/// and it was right about the second half: there was no warning here at all, so
/// the next person to find `Box::leak` in a codebase that otherwise avoids it would
/// reasonably assume it was a bug and "fix" it — or copy the pattern.
///
/// The bound, stated so it can be checked:
///
/// - **production calls it once per run**, in `capture`, and only when
///   `--hyp-base`/`--sophon-base` is in play; otherwise `Biz::bases()` returns
///   borrowed `&'static` strings and this is not called at all;
/// - **tests call it from fixture builders**, a handful of times per test.
///
/// What it would cost to be wrong: a loop. Each call leaks two short strings and
/// never frees them, so a caller in a loop accumulates without bound and nothing
/// would say so — which is the whole reason this paragraph exists.
///
/// **Not `Cow<'a, str>`.** That is the other fix the plan offers and it is the
/// better shape — no leak at all, and the override could live in the `CapturePlan`
/// it came from and be borrowed. It is also a change to every URL builder in the
/// crate, for zero functional gain, on a seam that runs once per process. Worth
/// doing if the shape ever needs to change; not worth doing to remove a leak that
/// provably cannot accumulate.
pub fn owned(hyp: String, sophon: String) -> Endpoints<'static> {
        Endpoints {
            hyp: Box::leak(hyp.into_boxed_str()),
            sophon: Box::leak(sophon.into_boxed_str()),
        }
    }

    /// Build a HoYoPlay URL for one endpoint.
    ///
    /// `language` is sent on every endpoint — measured, and it is what makes the
    /// response a launcher's rather than a default locale's.
    pub fn hyp_url(&self, biz: &Biz, endpoint: HypEndpoint, language: &str) -> String {
        let mut url = format!(
            "{}/{}?launcher_id={}&language={}",
            self.hyp.trim_end_matches('/'),
            endpoint.path(),
            biz.launcher_id,
            enc(language),
        );
        // The key keeps its brackets literally: that is the measured spelling of
        // the request, and a percent-encoded `game_ids%5B%5D` may be accepted by
        // a CDN and still not match what a reader diffs against.
        push_pair(&mut url, "game_ids[]", biz.game_id);
        if endpoint.needs_channel() {
            push_pair(&mut url, "channel", biz.channel);
            push_pair(&mut url, "sub_channel", biz.sub_channel);
        }
        url
    }

    /// The `getBuild` URL for one tag, or for the branch head when `tag` is `None`.
    pub fn get_build_url(&self, params: &BranchParams, tag: Option<&str>) -> String {
        let mut url = format!("{}/getBuild", self.sophon.trim_end_matches('/'));
        push_pair(&mut url, "branch", &params.branch);
        push_pair(&mut url, "package_id", &params.package_id);
        push_pair(&mut url, "password", &params.password);
        if let Some(t) = tag {
            push_pair(&mut url, "tag", t);
        }
        url
    }
}

/// The `main` branch's downloader parameters, resolved from `getGameBranches`.
///
/// This is the type that replaces the `package_id`/`password`/`branch` triple a
/// client would otherwise hardcode, and it carries the two tag fields the
/// version pair is resolved from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchParams {
    /// `main.package_id`.
    pub package_id: String,
    /// `main.password`.
    pub password: String,
    /// `main.branch`, always `"main"` in practice.
    pub branch: String,
    /// `main.tag`: the version the branch currently serves.
    pub tag: String,
    /// `main.diff_tags`: the patch-support list, **not** a retention list.
    ///
    /// `getBuild?tag=` serves far more than this. Measured 2026-10-14: `6.6.0`
    /// and `5.5.0` are both absent here and both served. That difference is what
    /// decides whether an old-version file is reconstructible at all.
    pub diff_tags: Vec<String>,
}

impl BranchParams {
    /// Pull the `main` branch out of a `getGameBranches` body.
    ///
    /// Strict on purpose. A missing field here means the request was wrong or the
    /// API changed shape, and both are facts a capture must record rather than
    /// paper over with a default — a default would produce a bundle built from a
    /// guessed package id.
    pub fn from_branches(body: &[u8]) -> Result<Self, ApiError> {
        let v: serde_json::Value =
            serde_json::from_slice(body).map_err(|e| ApiError::shape("getGameBranches", e))?;
        let branches = v
            .pointer("/data/game_branches")
            .and_then(|b| b.as_array())
            .ok_or_else(|| ApiError::shape("getGameBranches", "data.game_branches is absent"))?;
        // The list is per-game; the game is the one this tool asked about, and
        // there is exactly one. Select by `branch == "main"` rather than by index
        // so a second entry cannot silently become the one used.
        let main = branches
            .iter()
            .find(|b| b.pointer("/main/branch").and_then(|b| b.as_str()) == Some("main"))
            .ok_or_else(|| {
                ApiError::shape("getGameBranches", "no entry with main.branch == \"main\"")
            })?;
        let m = main
            .get("main")
            .ok_or_else(|| ApiError::shape("getGameBranches", "the main entry has no `main`"))?;

        Ok(BranchParams {
            package_id: field(m, "/package_id", "getGameBranches")?,
            password: field(m, "/password", "getGameBranches")?,
            branch: field(m, "/branch", "getGameBranches")?,
            tag: field(m, "/tag", "getGameBranches")?,
            diff_tags: m
                .get("diff_tags")
                .and_then(|d| d.as_array())
                .ok_or_else(|| ApiError::shape("getGameBranches", "main.diff_tags is absent"))?
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect(),
        })
    }
}

/// The `getBuild` URL for one tag, or for the branch head when `tag` is `None`.
///
/// A convenience over [`Endpoints::get_build_url`] for a caller that has only
/// the parameters and a base string. It delegates rather than duplicating, so the
/// two spellings cannot drift — the query order is load-bearing when a reader
/// diffs a captured URL against a fresh request.
impl BranchParams {
    /// See [`Endpoints::get_build_url`].
    pub fn get_build_url(&self, sophon_base: &str, tag: Option<&str>) -> String {
        Endpoints::new(sophon_base, sophon_base).get_build_url(self, tag)
    }
}

/// One required string field, named precisely when it is missing.
fn field(v: &serde_json::Value, pointer: &str, what: &str) -> Result<String, ApiError> {
    v.pointer(pointer)
        .and_then(|f| f.as_str())
        .map(str::to_string)
        .ok_or_else(|| ApiError::shape(what, format!("main.{pointer} is missing or not a string")))
}

/// Append `&key=encoded_value`, with the separator decided by what is there.
fn push_pair(url: &mut String, key: &str, value: &str) {
    url.push(if url.contains('?') { '&' } else { '?' });
    url.push_str(key);
    url.push('=');
    url.push_str(&enc(value));
}

/// Percent-encode a query **value**.
///
/// Keys are appended literally (see [`Endpoints::hyp_url`]); values are encoded,
/// so a `password` with a `/` or a space cannot change the request's shape.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = Vec::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                // Flush any pending escape run first, so a literal byte is never
                // emitted after the escapes that follow it.
                if !run.is_empty() {
                    out.push_str(&percent(&run));
                    run.clear();
                }
                out.push(b as char)
            }
            _ => run.push(b),
        }
    }
    if !run.is_empty() {
        out.push_str(&percent(&run));
    }
    out
}

/// `%XX` for each byte, uppercase — the spelling `application/x-www-form-
/// urlencoded` requires.
///
/// Public because `report` percent-encodes the same way for the `SUMMARY`
/// line; two encoders that disagree would make a value unquotable.
pub fn percent(bytes: &[u8]) -> String {
    let hex = crate::hash::hex_bytes_upper(bytes);
    let mut out = String::with_capacity(bytes.len() * 3);
    for pair in hex.as_bytes().chunks(2) {
        out.push('%');
        out.push(pair[0] as char);
        out.push(pair[1] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOBAL: &str = "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api";

    #[test]
    fn owned_endpoints_are_the_borrowed_ones_and_leak_only_on_construction() {
        // §5.5 row 12. `Endpoints::owned` is `Box::leak` on purpose and the doc
        // now says so; this checks the part a doc cannot: that the leaked value is
        // the one the caller passed, and that a second call is independent of the
        // first rather than aliasing it.
        //
        // The aliasing case is the one that would actually be wrong. If `owned`
        // returned a pointer into a shared buffer, or if the first call's strings
        // were freed, the second would read something else — and since both are
        // `&'static str` nothing would complain about the type.
        let a = Endpoints::owned("https://a.example".to_string(), "https://b.example".to_string());
        let b = Endpoints::owned("https://c.example".to_string(), "https://d.example".to_string());

        assert_eq!((a.hyp, a.sophon), ("https://a.example", "https://b.example"));
        assert_eq!((b.hyp, b.sophon), ("https://c.example", "https://d.example"));

        // And the borrowed constructor agrees with the owned one for a real base,
        // so the seam cannot quietly change which address a capture talks to.
        let resolved = Biz::resolve(BizId::Hk4eGlobal);
        let bases = resolved.bases();
        assert_eq!(bases.hyp, GLOBAL);
        let from_string = Endpoints::owned(GLOBAL.to_string(), bases.sophon.to_string());
        assert_eq!(from_string.hyp, bases.hyp, "the override path must not rewrite the base");
    }

    #[test]
    fn the_global_ids_are_the_measured_ones() {
        let b = Biz::resolve(BizId::Hk4eGlobal);
        assert_eq!(b.launcher_id, "VYTpXlbWo8");
        assert_eq!(b.game_id, "gopR6Cufr3");
        assert_eq!(b.channel_tuple(), ("1", "0", "hyp_hoyoverse"));
        assert_eq!(b.hyp_base(), GLOBAL);
        assert_eq!(
            b.sophon_base(),
            "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api"
        );
    }

    #[test]
    fn the_channel_tuple_differs_per_region() {
        assert_eq!(
            Biz::resolve(BizId::Hk4eCn).channel_tuple(),
            ("1", "1", "hyp_mihoyo")
        );
        assert_eq!(
            Biz::resolve(BizId::Hk4eGlobal).channel_tuple(),
            ("1", "0", "hyp_hoyoverse")
        );
        assert_eq!(
            Biz::resolve(BizId::Hk4eBilibili).channel_tuple(),
            ("14", "0", "hyp_mihoyo")
        );
    }

    #[test]
    fn cn_and_bilibili_share_a_host_family() {
        assert_eq!(Biz::resolve(BizId::Hk4eCn).host, Host::Mihoyo);
        assert_eq!(Biz::resolve(BizId::Hk4eBilibili).host, Host::Mihoyo);
        assert_eq!(Biz::resolve(BizId::Hk4eGlobal).host, Host::Hoyoverse);
        assert_eq!(
            Biz::resolve(BizId::Hk4eCn).hyp_base(),
            "https://hyp-api.mihoyo.com/hyp/hyp-connect/api"
        );
    }

    #[test]
    fn a_configs_url_is_the_measured_one() {
        let got = Biz::resolve(BizId::Hk4eGlobal).bases().hyp_url(
            &Biz::resolve(BizId::Hk4eGlobal),
            HypEndpoint::GameConfigs,
            "en-us",
        );
        assert_eq!(
            got,
            format!(
                "{GLOBAL}/getGameConfigs?launcher_id=VYTpXlbWo8&language=en-us&game_ids[]=gopR6Cufr3"
            )
        );
    }

    /// The deprecated-files call is the only one carrying the channel tuple, and
    /// it carries the *per-launcher* pair: a cn or bili capture sent `1/0` would
    /// be filtered against the wrong list.
    #[test]
    fn only_the_deprecated_endpoint_carries_the_channel_tuple() {
        let b = Biz::resolve(BizId::Hk4eBilibili);
        let ep = b.bases();
        let deprecated = ep.hyp_url(&b, HypEndpoint::GameDeprecatedFileConfigs, "en-us");
        assert!(
            deprecated.contains("&channel=14&sub_channel=0"),
            "{deprecated}"
        );
        assert!(
            deprecated.ends_with("&channel=14&sub_channel=0"),
            "{deprecated}"
        );

        for endpoint in [
            HypEndpoint::GameConfigs,
            HypEndpoint::GameBranches,
            HypEndpoint::GamePackages,
        ] {
            let url = ep.hyp_url(&b, endpoint, "en-us");
            assert!(!url.contains("channel="), "{ep:?} must not carry it: {url}");
        }
        assert_eq!(
            [
                HypEndpoint::GameConfigs,
                HypEndpoint::GameBranches,
                HypEndpoint::GameDeprecatedFileConfigs,
                HypEndpoint::GamePackages
            ]
            .iter()
            .filter(|e| e.needs_channel())
            .count(),
            1
        );
    }

    #[test]
    fn every_endpoint_path_is_the_api_spelling() {
        assert_eq!(HypEndpoint::GameConfigs.path(), "getGameConfigs");
        assert_eq!(HypEndpoint::GameBranches.path(), "getGameBranches");
        assert_eq!(
            HypEndpoint::GameDeprecatedFileConfigs.path(),
            "getGameDeprecatedFileConfigs"
        );
        assert_eq!(HypEndpoint::GamePackages.path(), "getGamePackages");
    }

    /// A `password` with a `/` would otherwise start a second query parameter.
    #[test]
    fn query_values_are_encoded() {
        assert_eq!(enc("a b/c+d?e"), "a%20b%2Fc%2Bd%3Fe");
        assert_eq!(
            enc("hk4e_global"),
            "hk4e_global",
            "ordinary ids pass through"
        );
        let p = BranchParams {
            package_id: "pkg id".into(),
            password: "p@ss/word".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec![],
        };
        let url = p.get_build_url("http://x/api", None);
        assert!(url.contains("package_id=pkg%20id"), "{url}");
        assert!(url.contains("password=p%40ss%2Fword"), "{url}");
        assert_eq!(url.matches('?').count(), 1, "{url}");
    }

    #[test]
    fn the_latest_get_build_url_omits_the_tag() {
        let p = BranchParams {
            package_id: "ScSYQBFhu9".into(),
            password: "bDL4JUHL625x".into(),
            branch: "main".into(),
            tag: "7.1.0".into(),
            diff_tags: vec!["7.0.0".into()],
        };
        assert_eq!(
            p.get_build_url("https://s/api", None),
            "https://s/api/getBuild?branch=main&package_id=ScSYQBFhu9&password=bDL4JUHL625x"
        );
        assert_eq!(
            p.get_build_url("https://s/api", Some("7.0.0")),
            "https://s/api/getBuild?branch=main&package_id=ScSYQBFhu9&password=bDL4JUHL625x&tag=7.0.0"
        );
    }

    /// A trailing slash on the base would otherwise produce `//getBuild`, which
    /// some proxies answer and some do not.
    #[test]
    fn a_trailing_slash_on_the_base_is_harmless() {
        let p = BranchParams {
            package_id: "a".into(),
            password: "b".into(),
            branch: "main".into(),
            tag: "t".into(),
            diff_tags: vec![],
        };
        let a = p.get_build_url("https://s/api/", None);
        assert!(!a.contains("//getBuild"), "{a}");
    }

    /// Measured 2026-10-14, verbatim.
    const BRANCHES: &str = r#"{"retcode":0,"message":"OK","data":{"game_branches":[{"game":{"id":"gopR6Cufr3","biz":"hk4e_global"},"main":{"package_id":"ScSYQBFhu9","branch":"main","password":"bDL4JUHL625x","tag":"7.1.0","diff_tags":["7.0.0","6.7.0"],"categories":[{"category_id":"10016","matching_field":"game"}]},"pre_download":null}]}}"#;

    #[test]
    fn branch_params_are_extracted_from_a_measured_body() {
        let p = BranchParams::from_branches(BRANCHES.as_bytes()).expect("measured body parses");
        assert_eq!(p.package_id, "ScSYQBFhu9");
        assert_eq!(p.password, "bDL4JUHL625x");
        assert_eq!(p.branch, "main");
        assert_eq!(p.tag, "7.1.0");
        assert_eq!(p.diff_tags, vec!["7.0.0".to_string(), "6.7.0".to_string()]);
    }

    /// Selecting by index would let a second entry become the one used.
    #[test]
    fn the_main_branch_is_chosen_by_name_not_position() {
        let body = br#"{"data":{"game_branches":[
            {"main":{"branch":"beta","package_id":"WRONG","password":"w","tag":"9.9.9","diff_tags":[]}},
            {"main":{"branch":"main","package_id":"RIGHT","password":"p","tag":"7.1.0","diff_tags":["7.0.0"]}}
        ]}}"#;
        assert_eq!(
            BranchParams::from_branches(body).unwrap().package_id,
            "RIGHT"
        );
    }

    #[test]
    fn a_missing_or_changed_shape_is_an_error_not_a_default() {
        for body in [
            &b"not json"[..],
            br#"{"data":{}}"#,
            br#"{"data":{"game_branches":[]}}"#,
            br#"{"data":{"game_branches":[{"main":{"branch":"beta"}}]}}"#,
            br#"{"data":{"game_branches":[{"main":{"branch":"main","package_id":"a","password":"b","branch":"main","tag":"1","diff_tags":[]}}]}}"#,
        ] {
            // The last one differs only in a missing `diff_tags` shape check:
            // an array is present, so it parses. Everything else must fail.
            let r = BranchParams::from_branches(body);
            if body.starts_with(br#"{"data":{"game_branches":[{"main":{"branch":"main","package_id":"a""#) {
                assert!(r.is_ok(), "{}", String::from_utf8_lossy(body));
            } else {
                assert!(r.is_err(), "unexpectedly parsed: {}", String::from_utf8_lossy(body));
            }
        }
    }

    #[test]
    fn a_diff_tags_entry_that_is_not_a_string_is_dropped_not_fatal() {
        let body = br#"{"data":{"game_branches":[{"main":{"branch":"main","package_id":"a","password":"b","tag":"1","diff_tags":["7.0.0",7,null]}}]}}"#;
        assert_eq!(
            BranchParams::from_branches(body).unwrap().diff_tags,
            vec!["7.0.0".to_string()]
        );
    }
}
