//! Monotonic time for agent supervision and retry deadlines. Persisted timestamps remain UTC;
//! subprocess I/O timeouts use real time so a test clock cannot hide a hung harness.

use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

/// A monotonic timeline shared by deadline writers, readers and sleepers. Implementations must
/// never move backwards, and must wake sleeps whose deadline is reached (including past deadlines).
pub trait Clock: std::fmt::Debug + Send + Sync {
    fn now(&self) -> Instant;
    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

#[derive(Debug)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(tokio::time::sleep_until(deadline.into()))
    }
}
