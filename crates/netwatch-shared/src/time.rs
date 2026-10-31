//! Monotonic time, as values (ADR-0004, ADR-0012).

use uom::si::f64::Time;
use uom::si::time::nanosecond;

/// A reading of the monotonic clock (`CLOCK_MONOTONIC`), in nanoseconds since an
/// arbitrary origin. The domain never reads a clock: the shell passes instants in.
/// Durations are only ever differences between instants, so wall-clock jumps can't
/// distort them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(u64);

impl Instant {
    /// Wraps a monotonic clock reading.
    pub const fn from_nanos(nanos: u64) -> Self {
        Self(nanos)
    }

    /// The reading, in nanoseconds since the clock's origin.
    pub const fn as_nanos(self) -> u64 {
        self.0
    }

    /// How long after `earlier` this instant is, or zero if it isn't later.
    pub fn since(self, earlier: Instant) -> Time {
        // u64 nanoseconds → f64 is exact for any duration under 104 days.
        Time::new::<nanosecond>(self.0.saturating_sub(earlier.0) as f64)
    }

    /// The instant `duration` after this one. Negative durations count as zero.
    pub fn after(self, duration: Time) -> Instant {
        let nanos = duration.get::<nanosecond>().max(0.0).round();
        // `as` saturates for floats that are out of range, which is the behavior we want.
        Instant(self.0.saturating_add(nanos as u64))
    }
}

/// A UTC timestamp in microseconds since the Unix epoch. It is used only to label
/// samples and events, never to measure a duration (ADR-0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WallTime(i64);

impl WallTime {
    /// Wraps microseconds since the Unix epoch.
    pub const fn from_unix_micros(micros: i64) -> Self {
        Self(micros)
    }

    /// Microseconds since the Unix epoch, as persisted.
    pub const fn as_unix_micros(self) -> i64 {
        self.0
    }
}
