use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Biz {
    #[value(name = "hk4e_cn")]
    Hk4eCn,
    #[value(name = "hk4e_global")]
    Hk4eGlobal,
    #[value(name = "hk4e_bilibili")]
    Hk4eBilibili,
}

impl Biz {
    pub fn as_str(&self) -> &'static str {
        match self {
            Biz::Hk4eCn => "hk4e_cn",
            Biz::Hk4eGlobal => "hk4e_global",
            Biz::Hk4eBilibili => "hk4e_bilibili",
        }
    }
    /// (host, launcher_id, game_id)
    pub fn endpoints(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Biz::Hk4eCn => ("mihoyo", "jGHBHlcOq1", "1Z8W5NHUQb"),
            Biz::Hk4eGlobal => ("hoyoverse", "VYTpXlbWo8", "gopR6Cufr3"),
            Biz::Hk4eBilibili => ("mihoyo", "umfgRO5gh5", "T2S0Gz4Dr2"),
        }
    }
    /// (channel, sub_channel, cps) written to config.ini
    pub fn channel_tuple(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Biz::Hk4eCn => ("1", "1", "hyp_mihoyo"),
            Biz::Hk4eGlobal => ("1", "0", "hyp_hoyoverse"),
            Biz::Hk4eBilibili => ("14", "0", "hyp_mihoyo"),
        }
    }
}
