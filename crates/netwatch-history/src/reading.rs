//! Readings: gauge values over time, and their rollups (ADR-0032).
//!
//! A reading rollup keeps count, sum, min, and max. Integer-valued sums are exact in
//! `f64` up to 2^53, so the merge is associative and commutative for every gauge
//! netwatch records, and coarsening equals rolling up directly (property-tested).

use std::collections::BTreeMap;

use netwatch_shared::{Gauge, IdentityEpoch, Role, WallTime};

use crate::Tier;

/// One gauge value of one role (glossary: *reading*).
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// Whose value it is.
    pub role: Role,
    /// The role's identity epoch when it was read.
    pub epoch: IdentityEpoch,
    /// When it was read.
    pub at: WallTime,
    /// What was read.
    pub gauge: Gauge,
    /// The value, in the gauge's unit.
    pub value: f64,
}

/// What a tier keeps about a gauge's readings in one interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReadingSummary {
    /// How many readings.
    pub count: u64,
    /// Their sum.
    pub sum: f64,
    /// The smallest; `+inf` when empty, so that it is the identity of `min`.
    pub min: f64,
    /// The largest; `-inf` when empty.
    pub max: f64,
}

impl ReadingSummary {
    /// The summary of nothing: the identity of [`merge`](Self::merge).
    pub const fn empty() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }

    /// The summary of one reading.
    pub const fn of(value: f64) -> Self {
        Self {
            count: 1,
            sum: value,
            min: value,
            max: value,
        }
    }

    /// Combines the summaries of two disjoint sets of readings.
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            count: self.count.saturating_add(other.count),
            sum: self.sum + other.sum,
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// The mean, if there were any readings.
    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum / self.count as f64)
    }
}

/// One gauge's readings of one role in one tier interval of one epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadingRollup {
    /// Whose readings.
    pub role: Role,
    /// The epoch they were read in. Rollups never span epochs (ADR-0003).
    pub epoch: IdentityEpoch,
    /// The tier.
    pub tier: Tier,
    /// The start of the interval.
    pub start: WallTime,
    /// Which gauge.
    pub gauge: Gauge,
    /// What the interval kept.
    pub summary: ReadingSummary,
}

type Key = (Role, IdentityEpoch, Gauge, WallTime);

/// Rolls readings up into `tier`, one rollup per role, epoch, gauge, and interval.
pub fn roll_up_readings(readings: &[Reading], tier: Tier) -> Vec<ReadingRollup> {
    let mut rollups: BTreeMap<Key, ReadingRollup> = BTreeMap::new();
    for r in readings {
        let start = tier.start_of(r.at);
        let rollup = rollups
            .entry((r.role.clone(), r.epoch, r.gauge, start))
            .or_insert_with(|| ReadingRollup {
                role: r.role.clone(),
                epoch: r.epoch,
                tier,
                start,
                gauge: r.gauge,
                summary: ReadingSummary::empty(),
            });
        rollup.summary = rollup.summary.merge(&ReadingSummary::of(r.value));
    }
    rollups.into_values().collect()
}

/// Merges finer reading rollups into `tier`.
pub fn coarsen_readings(rollups: &[ReadingRollup], tier: Tier) -> Vec<ReadingRollup> {
    let mut coarse: BTreeMap<Key, ReadingRollup> = BTreeMap::new();
    for fine in rollups {
        let start = tier.start_of(fine.start);
        let rollup = coarse
            .entry((fine.role.clone(), fine.epoch, fine.gauge, start))
            .or_insert_with(|| ReadingRollup {
                tier,
                start,
                summary: ReadingSummary::empty(),
                ..fine.clone()
            });
        rollup.summary = rollup.summary.merge(&fine.summary);
    }
    coarse.into_values().collect()
}
