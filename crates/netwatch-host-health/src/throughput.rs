//! Throughput: a NIC's byte counters as rates (ADR-0032).

use std::collections::BTreeMap;

use netwatch_shared::Instant;
use netwatch_shared::uom::si::time::second;
use netwatch_verified::counter_increase;

/// Turns successive byte-counter readings of one device into receive and transmit
/// throughput. Like a [`CounterWatch`](crate::CounterWatch), it compares readings of
/// one device only, so the caller starts a new one when the NIC changes.
///
/// A counter that went down was reset (a driver reload, or a 32-bit counter wrapping
/// on old hardware). That interval has no throughput: the verified
/// `counter_increase` says so, and the reset value becomes the new baseline.
#[derive(Debug, Default)]
pub struct Throughput {
    last: Option<(u64, u64, Instant)>,
}

const RX: &str = "Link.RxBytes";
const TX: &str = "Link.TxBytes";

impl Throughput {
    /// Takes the device's counters, read at `at`, and returns the receive and
    /// transmit throughput since the previous reading, in bit/s. `None` for the
    /// first reading, after a reset, if the counters are missing, or if no time
    /// has passed.
    pub fn observe(&mut self, counters: &BTreeMap<String, u64>, at: Instant) -> Option<(f64, f64)> {
        let (&rx, &tx) = (counters.get(RX)?, counters.get(TX)?);
        let previous = self.last.replace((rx, tx, at));
        let (last_rx, last_tx, last_at) = previous?;
        let seconds = at.since(last_at).get::<second>();
        if seconds <= 0.0 {
            return None;
        }
        let rx_bytes = counter_increase(last_rx, rx)?;
        let tx_bytes = counter_increase(last_tx, tx)?;
        Some((
            rx_bytes as f64 * 8.0 / seconds,
            tx_bytes as f64 * 8.0 / seconds,
        ))
    }
}
