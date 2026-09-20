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
