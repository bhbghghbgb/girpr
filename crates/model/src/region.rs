//! Region identity and the values both programs send verbatim.
//!
//! There is one enum for the three regions both tools can address, and one
//! table for the launcher ids, game ids, channel tuples and API bases behind
//! them. `girpr` and `gircap` used to carry separate copies of every string
//! here; a rename in one could silently leave the other sending the old id.
//!
//! The `channel`/`sub_channel` pair is sent only on the deprecated-files call.
//! The endpoint name that defines that rule is [`CHANNEL_ENDPOINT`]; each
//! program still decides *how* to build its request and keeps that decision at
//! its own call site. Nothing here builds a URL, resolves a plan or overrides
//! an address.

/// Which API family serves a region.
///
/// Mainland and Bilibili share the `mihoyo` family; the ids differ, the bases
/// do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Host {
    /// Mainland + Bilibili.
    Mihoyo,
    /// International.
    Hoyoverse,
}

impl Host {
    /// The host key the repair binary's `production_bases` has always taken.
    pub const fn as_str(self) -> &'static str {
        match self {
            Host::Mihoyo => "mihoyo",
            Host::Hoyoverse => "hoyoverse",
        }
    }

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

/// The three regions both programs know how to address.
///
/// The `value` spellings are the *API* ids (`hk4e_global`), not clap's derived
/// kebab-case. A bundle's documents name a region by its API id, so the flag
/// and the id must have the same spelling. The derive is present only with the
/// `clap` feature; the enum and its values are available without it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Region {
    /// Mainland China (`hk4e_cn`).
    #[cfg_attr(feature = "clap", value(name = "hk4e_cn"))]
    Hk4eCn,
    /// International (`hk4e_global`).
    #[cfg_attr(feature = "clap", value(name = "hk4e_global"))]
    Hk4eGlobal,
    /// Bilibili (`hk4e_bilibili`).
    #[cfg_attr(feature = "clap", value(name = "hk4e_bilibili"))]
    Hk4eBilibili,
}

impl Region {
    /// The id as the APIs and the flag spell it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Region::Hk4eCn => "hk4e_cn",
            Region::Hk4eGlobal => "hk4e_global",
            Region::Hk4eBilibili => "hk4e_bilibili",
        }
    }

    /// `(host key, launcher_id, game_id)`.
    ///
    /// The host key is `"mihoyo"` or `"hoyoverse"`; the repair binary maps it to
    /// a pair of bases, while the capture tool keeps the same distinction as a
    /// [`Host`] value. Both read the ids from here.
    pub const fn endpoints(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Region::Hk4eCn => ("mihoyo", "jGHBHlcOq1", "1Z8W5NHUQb"),
            Region::Hk4eGlobal => ("hoyoverse", "VYTpXlbWo8", "gopR6Cufr3"),
            Region::Hk4eBilibili => ("mihoyo", "umfgRO5gh5", "T2S0Gz4Dr2"),
        }
    }

    /// Which API family serves this region.
    pub const fn host(self) -> Host {
        match self {
            Region::Hk4eCn | Region::Hk4eBilibili => Host::Mihoyo,
            Region::Hk4eGlobal => Host::Hoyoverse,
        }
    }

    /// HoYoPlay `launcher_id`.
    pub const fn launcher_id(self) -> &'static str {
        self.endpoints().1
    }

    /// HoYoPlay `game_id`, sent as `game_ids[]`.
    pub const fn game_id(self) -> &'static str {
        self.endpoints().2
    }

    /// `channel`, written to `config.ini` and sent to the deprecated-files call.
    pub const fn channel(self) -> &'static str {
        self.channel_tuple().0
    }

    /// `sub_channel`.
    pub const fn sub_channel(self) -> &'static str {
        self.channel_tuple().1
    }

    /// `cps`, written to `config.ini`.
    pub const fn cps(self) -> &'static str {
        self.channel_tuple().2
    }

    /// `(channel, sub_channel, cps)` written to `config.ini`.
    pub const fn channel_tuple(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Region::Hk4eCn => ("1", "1", "hyp_mihoyo"),
            Region::Hk4eGlobal => ("1", "0", "hyp_hoyoverse"),
            Region::Hk4eBilibili => ("14", "0", "hyp_mihoyo"),
        }
    }
}

impl std::fmt::Display for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The only `default_download_mode` either program can work from.
///
/// The other two modes (`DOWNLOAD_MODE_FILE`, `DOWNLOAD_MODE_LDIFF`) deliver
/// whole archives and never chunk-repair. Both tools check this value at their
/// own boundary; the string itself is a fact of the API, so it lives here.
pub const DOWNLOAD_MODE_CHUNK: &str = "DOWNLOAD_MODE_CHUNK";

/// The single HoYoPlay endpoint that carries the `channel`/`sub_channel` pair.
///
/// Sending the pair to any other endpoint is not the documented request. Each
/// program keeps its own request builder; both key the rule off this name.
pub const CHANNEL_ENDPOINT: &str = "getGameDeprecatedFileConfigs";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_region_has_one_id_and_one_set_of_ids() {
        for (region, id, launcher, game, channel, sub, cps, host) in [
            (
                Region::Hk4eCn,
                "hk4e_cn",
                "jGHBHlcOq1",
                "1Z8W5NHUQb",
                "1",
                "1",
                "hyp_mihoyo",
                Host::Mihoyo,
            ),
            (
                Region::Hk4eGlobal,
                "hk4e_global",
                "VYTpXlbWo8",
                "gopR6Cufr3",
                "1",
                "0",
                "hyp_hoyoverse",
                Host::Hoyoverse,
            ),
            (
                Region::Hk4eBilibili,
                "hk4e_bilibili",
                "umfgRO5gh5",
                "T2S0Gz4Dr2",
                "14",
                "0",
                "hyp_mihoyo",
                Host::Mihoyo,
            ),
        ] {
            assert_eq!(region.as_str(), id);
            assert_eq!(region.launcher_id(), launcher);
            assert_eq!(region.game_id(), game);
            assert_eq!(region.channel_tuple(), (channel, sub, cps));
            assert_eq!(region.channel(), channel);
            assert_eq!(region.sub_channel(), sub);
            assert_eq!(region.cps(), cps);
            assert_eq!(region.host(), host);
            assert_eq!(region.to_string(), id);
        }
    }

    #[test]
    fn host_keys_and_bases_are_the_production_strings() {
        assert_eq!(Host::Mihoyo.as_str(), "mihoyo");
        assert_eq!(Host::Hoyoverse.as_str(), "hoyoverse");
        assert_eq!(
            Host::Mihoyo.hyp_base(),
            "https://hyp-api.mihoyo.com/hyp/hyp-connect/api"
        );
        assert_eq!(
            Host::Mihoyo.sophon_base(),
            "https://downloader-api.mihoyo.com/downloader/sophon_chunk/api"
        );
        assert_eq!(
            Host::Hoyoverse.hyp_base(),
            "https://sg-hyp-api.hoyoverse.com/hyp/hyp-connect/api"
        );
        assert_eq!(
            Host::Hoyoverse.sophon_base(),
            "https://sg-downloader-api.hoyoverse.com/downloader/sophon_chunk/api"
        );
        assert_eq!(Region::Hk4eCn.endpoints().0, Host::Mihoyo.as_str());
        assert_eq!(Region::Hk4eGlobal.endpoints().0, Host::Hoyoverse.as_str());
    }

    #[test]
    fn the_channel_rule_names_one_endpoint() {
        assert_eq!(CHANNEL_ENDPOINT, "getGameDeprecatedFileConfigs");
        assert_eq!(DOWNLOAD_MODE_CHUNK, "DOWNLOAD_MODE_CHUNK");
    }

    #[cfg(feature = "clap")]
    #[test]
    fn the_clap_values_are_the_api_ids() {
        use clap::ValueEnum;
        for (region, id) in [
            (Region::Hk4eCn, "hk4e_cn"),
            (Region::Hk4eGlobal, "hk4e_global"),
            (Region::Hk4eBilibili, "hk4e_bilibili"),
        ] {
            assert_eq!(region.to_possible_value().unwrap().get_name(), id);
        }
    }
}
