//! The rollup summary and its merge algebra (ADR-0006, ADR-0016).

use vstd::prelude::*;

verus! {

/// What a rollup keeps about its samples: how many there were, how many were lost,
/// and the sum, minimum, and maximum of the answered RTTs, in µs.
///
/// An empty summary has `rtt_min_us == u64::MAX` and `rtt_max_us == 0`, the identity
/// elements of min and max, so merging with it changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub count: u64,
    pub lost: u64,
    pub rtt_sum_us: u64,
    pub rtt_min_us: u64,
    pub rtt_max_us: u64,
}

/// A summary's fields as mathematical integers, so the laws can be stated without
/// overflow.
#[allow(dead_code)] // Only specs and proofs use it, and a normal build erases them.
pub struct SummaryView {
    pub count: int,
    pub lost: int,
    pub rtt_sum_us: int,
    pub rtt_min_us: int,
    pub rtt_max_us: int,
}

pub open spec fn min(a: int, b: int) -> int {
    if a <= b { a } else { b }
}

pub open spec fn max(a: int, b: int) -> int {
    if a >= b { a } else { b }
}

/// The meaning of merging: counts and sums add, minimum and maximum combine.
pub open spec fn merged(a: SummaryView, b: SummaryView) -> SummaryView {
    SummaryView {
        count: a.count + b.count,
        lost: a.lost + b.lost,
        rtt_sum_us: a.rtt_sum_us + b.rtt_sum_us,
        rtt_min_us: min(a.rtt_min_us, b.rtt_min_us),
        rtt_max_us: max(a.rtt_max_us, b.rtt_max_us),
    }
}

impl Summary {
    pub open spec fn view(self) -> SummaryView {
        SummaryView {
            count: self.count as int,
            lost: self.lost as int,
            rtt_sum_us: self.rtt_sum_us as int,
            rtt_min_us: self.rtt_min_us as int,
            rtt_max_us: self.rtt_max_us as int,
        }
    }

    /// The summary of no samples.
    pub fn empty() -> (s: Self)
        ensures
            s.view() == (SummaryView { count: 0, lost: 0, rtt_sum_us: 0, rtt_min_us: u64::MAX as int, rtt_max_us: 0 }),
    {
        Summary { count: 0, lost: 0, rtt_sum_us: 0, rtt_min_us: u64::MAX, rtt_max_us: 0 }
    }

    /// The summary of one sample: answered after `rtt_us`, or lost.
    pub fn of_sample(rtt_us: Option<u64>) -> (s: Self)
        ensures
            s.count == 1,
            s.lost == (if rtt_us.is_some() { 0u64 } else { 1u64 }),
    {
        match rtt_us {
            Some(rtt) => Summary { count: 1, lost: 0, rtt_sum_us: rtt, rtt_min_us: rtt, rtt_max_us: rtt },
            None => Summary { count: 1, lost: 1, rtt_sum_us: 0, rtt_min_us: u64::MAX, rtt_max_us: 0 },
        }
    }

    /// Merges two summaries, or `None` if a count or sum would overflow `u64`, which
    /// would take 584,000 years of samples at one per microsecond.
    ///
    /// **Proven:** the result means exactly [`merged`] of the two inputs.
    pub fn merge(&self, other: &Self) -> (m: Option<Self>)
        ensures
            match m {
                Some(m) => m.view() == merged(self.view(), other.view()),
                None => self.count as int + other.count as int > u64::MAX as int
                    || self.lost as int + other.lost as int > u64::MAX as int
                    || self.rtt_sum_us as int + other.rtt_sum_us as int > u64::MAX as int,
            },
    {
        let count = self.count.checked_add(other.count)?;
        let lost = self.lost.checked_add(other.lost)?;
        let rtt_sum_us = self.rtt_sum_us.checked_add(other.rtt_sum_us)?;
        Some(Summary {
            count,
            lost,
            rtt_sum_us,
            rtt_min_us: if self.rtt_min_us <= other.rtt_min_us { self.rtt_min_us } else { other.rtt_min_us },
            rtt_max_us: if self.rtt_max_us >= other.rtt_max_us { self.rtt_max_us } else { other.rtt_max_us },
        })
    }
}

/// **Proven:** merge order doesn't matter.
pub proof fn merge_is_commutative(a: SummaryView, b: SummaryView)
    ensures
        merged(a, b) == merged(b, a),
{
}

/// **Proven:** merge grouping doesn't matter, so 1h = merge of its 1m rollups =
/// merge of its raw samples, however the work is split.
pub proof fn merge_is_associative(a: SummaryView, b: SummaryView, c: SummaryView)
    ensures
        merged(merged(a, b), c) == merged(a, merged(b, c)),
{
}

/// **Proven:** the empty summary is the identity of merge, for any summary whose
/// minimum and maximum are within `u64`.
pub proof fn empty_is_the_identity(a: SummaryView)
    requires
        0 <= a.rtt_min_us <= u64::MAX as int,
        0 <= a.rtt_max_us,
    ensures
        merged(a, SummaryView { count: 0, lost: 0, rtt_sum_us: 0, rtt_min_us: u64::MAX as int, rtt_max_us: 0 }) == a,
{
}

} // verus!
