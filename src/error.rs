/// Typed failure carrying the process exit code, so the error contract in the
/// README / docs/02 (§1) is enforced at runtime instead of a blanket `2`.
/// `1` usage/config, `2` metadata/network, `3` write/verify.
#[derive(Debug)]
pub struct RunFailure {
    pub exit_code: i32,
    pub source: anyhow::Error,
}

impl RunFailure {
    pub fn usage(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 1,
            source: e.into(),
        }
    }
    pub fn metadata(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 2,
            source: e.into(),
        }
    }
    pub fn write(e: impl Into<anyhow::Error>) -> Self {
        Self {
            exit_code: 3,
            source: e.into(),
        }
    }
}

impl std::fmt::Display for RunFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.source)
    }
}
impl std::error::Error for RunFailure {}
