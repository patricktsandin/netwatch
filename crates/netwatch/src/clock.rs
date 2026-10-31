//! The only place that reads clocks (ADR-0004, ADR-0012).

use std::time::{Instant as StdInstant, SystemTime, UNIX_EPOCH};

use netwatch_shared::{Instant, WallTime};

/// Converts between the domain's [`Instant`] and the runtime's clocks. Instants count
/// nanoseconds of `CLOCK_MONOTONIC` (which is what `std::time::Instant` reads on Linux)
/// since the daemon started.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    origin: StdInstant,
}

impl Clock {
    pub fn start() -> Self {
        Self {
            origin: StdInstant::now(),
        }
    }

    pub fn now(&self) -> Instant {
        Instant::from_nanos(u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX))
    }

    pub fn wall_now(&self) -> WallTime {
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX));
        WallTime::from_unix_micros(micros)
    }

    pub fn deadline(&self, instant: Instant) -> tokio::time::Instant {
        tokio::time::Instant::from_std(self.origin)
            + std::time::Duration::from_nanos(instant.as_nanos())
    }
}
