//! Time, without a clock.
//!
//! The engine never reads the clock: every method that cares about time takes a `now` argument.
//! That keeps the crate sans-IO and makes the switch's timing quirks (1 s command spacing,
//! 200 ms reply retries) testable instantly and deterministically.

use core::ops::Add;
use core::time::Duration;

/// A monotonic point in time.
///
/// Implemented by `std::time::Instant`, `tokio::time::Instant` and [`Tick`] through a blanket
/// impl: anything that is copyable, ordered and can have a [`Duration`] added to it qualifies.
pub trait Timestamp: Copy + Ord + Add<Duration, Output = Self> {}

impl<T> Timestamp for T where T: Copy + Ord + Add<Duration, Output = T> {}

/// A deterministic [`Timestamp`] for tests: a plain offset from an arbitrary epoch.
///
/// ```
/// use core::time::Duration;
/// use tesmart_proto::time::Tick;
///
/// let start = Tick::ZERO;
/// let later = start + Duration::from_millis(250);
/// assert!(later > start);
/// assert_eq!(later.since_epoch(), Duration::from_millis(250));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Tick(Duration);

impl Tick {
    /// The epoch.
    pub const ZERO: Self = Self(Duration::ZERO);

    /// A tick `millis` after the epoch.
    pub const fn from_millis(millis: u64) -> Self {
        Self(Duration::from_millis(millis))
    }

    /// A tick `secs` after the epoch.
    pub const fn from_secs(secs: u64) -> Self {
        Self(Duration::from_secs(secs))
    }

    /// Time elapsed since the epoch.
    pub const fn since_epoch(self) -> Duration {
        self.0
    }
}

impl Add<Duration> for Tick {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self {
        Self(self.0 + rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_timestamp<T: Timestamp>(_: T) {}

    #[test]
    fn tick_is_a_timestamp() {
        assert_timestamp(Tick::ZERO);
        assert_timestamp(std::time::Instant::now());
    }

    #[test]
    fn ticks_order_by_offset() {
        assert!(Tick::from_millis(1) < Tick::from_secs(1));
        assert_eq!(Tick::from_millis(1000), Tick::from_secs(1));
        assert_eq!(Tick::ZERO + Duration::from_secs(2), Tick::from_secs(2));
    }
}
