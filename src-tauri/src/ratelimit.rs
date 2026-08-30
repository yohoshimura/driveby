//! A byte-rate ceiling for the copy loop.
//!
//! Copying at the disk's full speed makes the machine unpleasant to use
//! while a backup runs, which is a good way to teach someone to stop running
//! backups. The ceiling is a token bucket: tokens accrue at the configured
//! rate, a writer takes as many as the chunk it is about to write, and waits
//! when there are not enough.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::Mutex;
// tokio's Instant rather than std's, so the tests can drive the clock
// instead of sleeping through it. In production the two are the same thing.
use tokio::time::Instant;

struct Bucket {
    /// Bytes' worth of budget available right now.
    tokens: f64,
    /// When `tokens` was last brought up to date.
    last: Instant,
    /// The largest single request seen so far, which the cap must never fall
    /// below: a bucket smaller than the chunk being asked for could never
    /// fill enough to grant it, and the copy loop would wait for ever.
    ///
    /// Remembered rather than recomputed per call. The cap used to be
    /// `max(rate, n)` of whichever request happened to be in hand, so a full
    /// 1 MiB chunk raised it, tokens accrued up there, and the short final
    /// chunk of the same file lowered it again — clamping away budget that
    /// had already been earned. The error was conservative, but it meant a
    /// ceiling that quietly delivered less than it advertised.
    largest_request: f64,
}

pub struct RateLimiter {
    /// Bytes per second. Zero means no ceiling and short-circuits every
    /// call — the setting's default, and the path that must stay free.
    rate: AtomicU64,
    bucket: Mutex<Bucket>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            rate: AtomicU64::new(0),
            bucket: Mutex::new(Bucket {
                tokens: 0.0,
                last: Instant::now(),
                largest_request: 0.0,
            }),
        }
    }

    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Wait until `n` bytes may be written, then spend them.
    ///
    /// The lock is held only for the arithmetic; the waiting happens outside
    /// it, so a writer that has to wait doesn't stop the others from
    /// accounting for what they wrote.
    pub async fn acquire(&self, n: u64) {
        let rate = self.rate();
        if rate == 0 {
            return;
        }
        let rate = rate as f64;
        loop {
            let wait = {
                let mut bucket = self.bucket.lock().await;
                // One second of budget, but never less than the biggest chunk
                // anyone has asked for. Derived from the remembered maximum,
                // not from this call, so a small request cannot discard what a
                // large one banked — while lowering the ceiling still lowers
                // the cap, because `rate` is read fresh each time.
                bucket.largest_request = bucket.largest_request.max(n as f64);
                let capacity = rate.max(bucket.largest_request);
                let now = Instant::now();
                let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
                bucket.last = now;
                bucket.tokens = (bucket.tokens + elapsed * rate).min(capacity);
                if bucket.tokens >= n as f64 {
                    bucket.tokens -= n as f64;
                    return;
                }
                Duration::from_secs_f64((n as f64 - bucket.tokens) / rate)
            };
            tokio::time::sleep(wait).await;
        }
    }
}

/// The one bucket every copy in the process draws from.
///
/// Global on purpose. "Driveby never goes above 50 MB/s" has to hold when
/// the scheduler fires three tasks at once, and a limiter per run would let
/// them add up to three times the ceiling — which is precisely the moment
/// the machine feels slow and the setting was supposed to prevent.
pub fn shared() -> &'static RateLimiter {
    static SHARED: OnceLock<RateLimiter> = OnceLock::new();
    SHARED.get_or_init(RateLimiter::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A large chunk followed by a small one. The cap used to be derived
    /// from whichever request was in hand, so the 4 000-byte call raised it,
    /// four seconds of budget accrued up there, and the next 10-byte call
    /// lowered it back to one second — throwing away three seconds{27} worth
    /// the copy loop had already waited for. The copy loop does exactly this
    /// at the end of every file.
    #[tokio::test(start_paused = true)]
    async fn a_small_request_does_not_discard_what_a_large_one_banked() {
        let limiter = RateLimiter::new();
        limiter.set_rate(1_000);
        // Ask for a big chunk once so the bucket knows how large it must be.
        limiter.acquire(4_000).await;
        // Idle long enough to refill to that capacity.
        tokio::time::sleep(Duration::from_secs(10)).await;

        // A short chunk, then the banked budget: all of it should still be
        // there, so neither call waits.
        let start = Instant::now();
        limiter.acquire(10).await;
        limiter.acquire(3_990).await;
        assert_eq!(
            start.elapsed(),
            Duration::ZERO,
            "the budget earned before the short chunk was thrown away"
        );
    }

    /// The default, and the path every run takes until someone sets a
    /// ceiling: no lock, no clock, no wait.
    #[tokio::test(start_paused = true)]
    async fn no_ceiling_means_no_wait() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        limiter.acquire(u64::MAX).await;
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_second_of_budget_takes_a_second_from_empty() {
        let limiter = RateLimiter::new();
        limiter.set_rate(1_000);
        let start = Instant::now();
        limiter.acquire(1_000).await;
        assert!(start.elapsed() >= Duration::from_millis(999));
    }

    /// The copy loop reads in 1 MiB chunks. A ceiling below that is a
    /// perfectly reasonable thing to ask for, and it must not deadlock:
    /// the bucket grows to fit the request rather than capping at one
    /// second's worth.
    #[tokio::test(start_paused = true)]
    async fn a_chunk_larger_than_a_second_of_budget_still_passes() {
        let limiter = RateLimiter::new();
        limiter.set_rate(1_000);
        let start = Instant::now();
        limiter.acquire(4_000).await;
        assert!(start.elapsed() >= Duration::from_millis(3_999));
    }

    /// The property that makes the ceiling mean anything: two runs at once
    /// share it instead of getting one each.
    #[tokio::test(start_paused = true)]
    async fn concurrent_callers_share_the_ceiling() {
        let limiter = Arc::new(RateLimiter::new());
        limiter.set_rate(1_000);
        let start = Instant::now();

        let a = tokio::spawn({
            let limiter = limiter.clone();
            async move { limiter.acquire(1_000).await }
        });
        let b = tokio::spawn({
            let limiter = limiter.clone();
            async move { limiter.acquire(1_000).await }
        });
        a.await.unwrap();
        b.await.unwrap();

        assert!(
            start.elapsed() >= Duration::from_millis(1_999),
            "two seconds' worth of bytes took {:?}",
            start.elapsed()
        );
    }

    /// Idle time accrues budget, so a run that pauses does not then have to
    /// wait again for bytes it has already earned.
    #[tokio::test(start_paused = true)]
    async fn budget_accrues_while_nothing_is_copying() {
        let limiter = RateLimiter::new();
        limiter.set_rate(1_000);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let start = Instant::now();
        limiter.acquire(1_000).await;
        assert_eq!(start.elapsed(), Duration::ZERO);
    }

    /// Budget does not accrue for ever: an app left open all night must not
    /// bank eight hours of bytes and then ignore the ceiling completely.
    #[tokio::test(start_paused = true)]
    async fn budget_stops_accruing_at_one_second() {
        let limiter = RateLimiter::new();
        limiter.set_rate(1_000);
        tokio::time::sleep(Duration::from_secs(60)).await;
        let start = Instant::now();
        limiter.acquire(1_000).await;
        limiter.acquire(1_000).await;
        assert!(
            start.elapsed() >= Duration::from_millis(999),
            "the second batch should have had to wait: {:?}",
            start.elapsed()
        );
    }
}
