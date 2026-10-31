//! Rollups: what a tier keeps about a role's samples in one interval of one epoch.

use std::collections::BTreeMap;

use netwatch_shared::uom::si::time::microsecond;
use netwatch_shared::{IdentityEpoch, Role, WallTime};
use netwatch_verified::Summary;

use crate::{Measurement, Sample};

/// A resolution of stored history (glossary: *tier*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// One-minute rollups, kept 90 days.
    Minute,
    /// One-hour rollups, kept indefinitely.
    Hour,
}

impl Tier {
    /// The tier's interval, in µs.
    pub const fn interval_us(self) -> i64 {
        match self {
            Tier::Minute => 60_000_000,
            Tier::Hour => 3_600_000_000,
        }
    }

    /// The start of the interval containing `at`.
    pub fn start_of(self, at: WallTime) -> WallTime {
        WallTime::from_unix_micros(
            at.as_unix_micros().div_euclid(self.interval_us()) * self.interval_us(),
        )
    }
}

/// RTTs bucketed on a log scale, 10% wide, so that percentiles can be merged. Bucket
/// `i` holds RTTs in `[1.1^i - 1, 1.1^(i+1) - 1)` µs. Merging adds counts per bucket,
/// which is commutative and associative because addition is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Histogram(BTreeMap<u8, u32>);

const GROWTH: f64 = 1.1;

impl Histogram {
    fn bucket(rtt_us: u64) -> u8 {
        let index = ((rtt_us as f64) + 1.0).ln() / GROWTH.ln();
        // `as` saturates: anything beyond bucket 255 (about 36 hours) lands there.
        index.floor() as u8
    }

    fn record(&mut self, rtt_us: u64) {
        *self.0.entry(Self::bucket(rtt_us)).or_default() += 1;
    }

    /// Adds another histogram's counts to this one.
    pub fn merge(&mut self, other: &Histogram) {
        for (&bucket, &count) in &other.0 {
            let total = self.0.entry(bucket).or_default();
            *total = total.saturating_add(count);
        }
    }

    /// The upper bound (µs) of the bucket containing quantile `q` (0..=1) of the
    /// recorded RTTs, or `None` if nothing was recorded. It overstates the true
    /// quantile by at most 10%.
    pub fn quantile_us(&self, q: f64) -> Option<u64> {
        let total: u64 = self.0.values().map(|&c| u64::from(c)).sum();
        if total == 0 {
            return None;
        }
        let rank = (q.clamp(0.0, 1.0) * total as f64).ceil().max(1.0) as u64;
        let mut seen = 0u64;
        for (&bucket, &count) in &self.0 {
            seen += u64::from(count);
            if seen >= rank {
                return Some((GROWTH.powi(i32::from(bucket) + 1) - 1.0).ceil() as u64);
            }
        }
        None
    }

    /// The non-empty buckets, as persisted.
    pub fn buckets(&self) -> impl Iterator<Item = (u8, u32)> + '_ {
        self.0.iter().map(|(&b, &c)| (b, c))
    }

    /// Rebuilds a histogram from persisted buckets.
    pub fn from_buckets(buckets: impl IntoIterator<Item = (u8, u32)>) -> Self {
        Self(buckets.into_iter().filter(|&(_, c)| c > 0).collect())
    }
}

/// What a tier keeps about one role's samples in one interval of one epoch. It never
/// spans two epochs (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rollup {
    /// The role.
    pub role: Role,
    /// The epoch every summarized sample belongs to.
    pub epoch: IdentityEpoch,
    /// The resolution.
    pub tier: Tier,
    /// The interval's start.
    pub start: WallTime,
    /// Counts, loss, and RTT sum, minimum, and maximum, merged with the verified
    /// algebra.
    pub summary: Summary,
    /// The answered RTTs, for percentiles.
    pub histogram: Histogram,
}

type Key = (Role, IdentityEpoch, WallTime);

/// Rolls samples up into `tier`, one rollup per role, epoch, and interval.
pub fn roll_up(samples: &[Sample], tier: Tier) -> Vec<Rollup> {
    let mut rollups: BTreeMap<Key, Rollup> = BTreeMap::new();
    for sample in samples {
        let rtt_us = match sample.measurement {
            Measurement::RoundTrip(rtt) => Some(rtt.get::<microsecond>().round().max(0.0) as u64),
            Measurement::Lost(_) => None,
        };
        let start = tier.start_of(sample.at);
        let rollup = rollups
            .entry((sample.role.clone(), sample.epoch, start))
            .or_insert_with(|| Rollup {
                role: sample.role.clone(),
                epoch: sample.epoch,
                tier,
                start,
                summary: Summary::empty(),
                histogram: Histogram::default(),
            });
        add(rollup, &Summary::of_sample(rtt_us), rtt_us);
    }
    rollups.into_values().collect()
}

/// Merges finer rollups into `tier`. Because the merge algebra is commutative and
/// associative (proven), this gives the same result as rolling the raw samples up into
/// `tier` directly.
pub fn coarsen(rollups: &[Rollup], tier: Tier) -> Vec<Rollup> {
    let mut coarse: BTreeMap<Key, Rollup> = BTreeMap::new();
    for fine in rollups {
        let start = tier.start_of(fine.start);
        let rollup = coarse
            .entry((fine.role.clone(), fine.epoch, start))
            .or_insert_with(|| Rollup {
                tier,
                start,
                summary: Summary::empty(),
                histogram: Histogram::default(),
                ..fine.clone()
            });
        rollup.summary = rollup
            .summary
            .merge(&fine.summary)
            .expect("u64 sample counts cannot be exhausted");
        rollup.histogram.merge(&fine.histogram);
    }
    coarse.into_values().collect()
}

fn add(rollup: &mut Rollup, summary: &Summary, rtt_us: Option<u64>) {
    rollup.summary = rollup
        .summary
        .merge(summary)
        .expect("u64 sample counts cannot be exhausted");
    if let Some(rtt) = rtt_us {
        rollup.histogram.record(rtt);
    }
}
