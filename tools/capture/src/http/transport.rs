use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

/// When to retry, and how long to wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Attempts in total, including the first. `1` means never retry.
    pub attempts: u32,
    /// Delay before the second attempt; doubles each time after that.
    pub base_delay: Duration,
    /// Ceiling on the doubling, so `--retries 10` cannot sleep for an hour.
    pub max_delay: Duration,
}

impl RetryPolicy {
    /// The default for a `--retries` value.
    pub fn new(attempts: u32) -> Self {
        RetryPolicy {
            attempts: attempts.max(1),
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(10),
        }
    }

    /// The delay before attempt number `attempt` (1-based; `1` never waits).
    ///
    /// Pure and off by construction: `base * 2^(attempt - 2)` is the delay
    /// *before* attempt 2, and the exponent is clamped before it can overflow a
    /// shift, which `--retries 10` would otherwise do at attempt 34.
    pub fn delay_before(&self, attempt: u32) -> Duration {
        if attempt <= 1 || self.base_delay.is_zero() {
            return Duration::ZERO;
        }
        let shift = (attempt - 2).min(16);
        let scaled = self
            .base_delay
            .checked_mul(1u32 << shift)
            .unwrap_or(self.max_delay);
        scaled.min(self.max_delay)
    }

    /// Whether a status is worth another attempt.
    ///
    /// The split is the whole retry policy in miniature: a server that is
    /// overloaded or briefly broken may answer again, and a server that has
    /// decided this URL does not exist will not.
    pub fn should_retry_status(status: u16) -> bool {
        matches!(status, 408 | 429) || (500..=599).contains(&status)
    }
}

/// The future [`Sleep::sleep`] hands back.
///
/// **Boxed on purpose.** [`Fetcher`] holds an `Arc<dyn Sleep>`, and a trait
/// method returning `impl Future` or declared `async fn` is not dyn-compatible.
/// One `Box` per *retry backoff* is nothing beside the 55.5 s of waiting it may
/// be standing in for. `'static` because neither implementor borrows `self`.
pub type SleepFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;

/// Something that can be waited on, so tests can count delays instead of
/// spending them.
///
/// **Async on purpose.** This was `fn sleep(&self, d: Duration)` and the real
/// implementation was [`std::thread::sleep`], called from inside
/// [`Fetcher::get`] — an `async fn` whose tasks are spawned onto the runtime.
/// So a retry backoff *blocked a tokio worker thread* rather than yielding it:
/// with `--retries 10` the schedule is 0.5+1+2+4+8+10+10+10+10 = 55.5 s of
/// thread-blocked time per failing request, and on a 4-core box at `--jobs 64`
/// all four workers can be asleep at once with nothing able to make progress.
///
/// [`tokio::time::sleep`] yields the worker instead, so the requests that *are*
/// in flight keep going while this one waits.
pub trait Sleep: Send + Sync + fmt::Debug {
    /// Wait for `d`.
    fn sleep(&self, d: Duration) -> SleepFuture;
}

/// Real wall-clock waiting, without taking the worker with it.
#[derive(Debug, Default)]
pub struct RealSleep;

impl Sleep for RealSleep {
    fn sleep(&self, d: Duration) -> SleepFuture {
        Box::pin(async move {
            if !d.is_zero() {
                tokio::time::sleep(d).await;
            }
        })
    }
}

/// Records what it was asked to wait for and returns immediately.
#[derive(Debug, Default)]
pub struct RecordingSleep(Mutex<Vec<Duration>>);

impl RecordingSleep {
    /// Every delay requested so far.
    pub fn delays(&self) -> Vec<Duration> {
        self.lock().clone()
    }

    /// Take the lock, recovering from poisoning — the same reasoning as
    /// [`RequestLog::lock`]. A recorded delay is evidence about a retry
    /// schedule, and losing it to a panic is a worse outcome than reading a
    /// half-pushed vector.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Duration>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Sleep for RecordingSleep {
    fn sleep(&self, d: Duration) -> SleepFuture {
        // Recorded before the future is built rather than inside it: this
        // sleeper never waits, so the only observable difference would be when
        // the push happens, and doing it here keeps the future free of a
        // borrow and therefore `'static`.
        self.lock().push(d);
        Box::pin(async {})
    }
}

/// The default ceiling on one response body: 64 MiB.
///
/// **Seven times the largest thing this tool fetches.** The reference bundle's
/// biggest single response is the 7.1.0 `game` manifest archive at 9 007 187 B,
/// and the biggest chunk blob is 2 162 366 B. The headroom is deliberate: a
/// ceiling that a legitimate future tag could brush against would be a worse
/// failure than the one it prevents, because it would turn a working capture into
/// a non-working one.
///
/// The point of the ceiling is not this number. It is that there is a number at
/// all: a broken or hostile origin that never stops sending would otherwise be
/// bounded by nothing but the machine's memory. See
/// [`Transport::max_response_bytes`].
pub const DEFAULT_MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;

/// How much concurrency and patience a fetcher has.
#[derive(Debug, Clone, Copy)]
pub struct Transport {
    /// Attempts per request, including the first.
    pub retries: u32,
    /// Per-request timeout.
    pub timeout: Duration,
    /// The most bytes one response body may contribute, in total.
    ///
    /// **A timeout is not a size limit.** A server streaming a gigabyte steadily
    /// for ten minutes never trips one, so the ceiling is the only bound on the
    /// path between a socket and this tool's buffers.
    ///
    /// Checked in two places, because one is not enough: against the declared
    /// `Content-Length` before a byte is read (so a server claiming 10 GiB is
    /// refused immediately), and against the running total as bytes arrive (so a
    /// server that lies about or omits the header is still stopped).
    ///
    /// Exceeding it is a [`ApiError::Shape`], so it is **not** retried: an origin
    /// that sent too much will send too much again.
    pub max_response_bytes: u64,
}

impl Transport {
    /// For a local mock, where a retry or a long timeout would turn a fast
    /// failure into a slow one.
    pub const fn local() -> Self {
        Transport {
            retries: 1,
            timeout: Duration::from_secs(10),
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
        }
    }
}
