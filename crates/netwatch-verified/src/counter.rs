//! Kernel counters: increases, and resets told apart from them (ADR-0008).

use vstd::prelude::*;

verus! {

/// How much a monotonic counter grew from `previous` to `current`, or `None` if it
/// went backwards. A counter only goes backwards when it was reset (a reboot, a
/// module reload) or wrapped, and in neither case is the difference a count of
/// anything. Callers report a counter reset, never negative or huge traffic.
///
/// **Proven:** an increase is returned exactly when the counter didn't go
/// backwards, and it is exactly the difference, so it can't underflow.
pub fn counter_increase(previous: u64, current: u64) -> (increase: Option<u64>)
    ensures
        match increase {
            Some(d) => current >= previous && d == current - previous,
            None => current < previous,
        },
{
    if current >= previous {
        Some(current - previous)
    } else {
        None
    }
}

} // verus!
