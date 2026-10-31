//! Rollups: the tiers agree with each other, and never mix epochs.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::num::NonZeroU64;

use netwatch_history::{Loss, Measurement, Sample, Tier, coarsen, roll_up};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::microsecond;
use netwatch_shared::{IdentityEpoch, Role, TargetId, WallTime};
use proptest::prelude::*;

fn sample(at_s: i64, epoch: u64, rtt_us: Option<u32>) -> Sample {
    Sample {
        role: Role::Target(TargetId::new(1)),
        epoch: IdentityEpoch::new(NonZeroU64::new(epoch).unwrap()),
        at: WallTime::from_unix_micros(at_s * 1_000_000),
        measurement: rtt_us.map_or(Measurement::Lost(Loss::TimedOut), |r| {
            Measurement::RoundTrip(Time::new::<microsecond>(f64::from(r)))
        }),
    }
}

proptest! {
    /// The property ADR-0006 promises: hours rolled up from minutes equal hours rolled
    /// up from raw samples.
    #[test]
    fn coarsening_minutes_equals_rolling_up_hours_directly(
        samples in prop::collection::vec(
            (0i64..3 * 3600, 1u64..3, prop::option::of(0u32..200_000)),
            0..200,
        )
    ) {
        let samples: Vec<Sample> = samples.into_iter().map(|(at, e, r)| sample(at, e, r)).collect();
        prop_assert_eq!(
            coarsen(&roll_up(&samples, Tier::Minute), Tier::Hour),
            roll_up(&samples, Tier::Hour)
        );
    }
}

#[test]
fn a_rollup_never_spans_two_epochs() {
    let rollups = roll_up(
        &[sample(0, 1, Some(10)), sample(1, 2, Some(20))],
        Tier::Minute,
    );
    assert_eq!(rollups.len(), 2);
}

#[test]
fn summary_counts_answers_and_losses() {
    let [rollup] = roll_up(
        &[
            sample(0, 1, Some(1_000)),
            sample(5, 1, None),
            sample(9, 1, Some(3_000)),
        ],
        Tier::Minute,
    )
    .try_into()
    .unwrap();
    assert_eq!((rollup.summary.count, rollup.summary.lost), (3, 1));
    assert_eq!(
        (rollup.summary.rtt_min_us, rollup.summary.rtt_max_us),
        (1_000, 3_000)
    );
}

#[test]
fn quantiles_overstate_by_at_most_ten_percent() {
    let samples: Vec<Sample> = (1..=100)
        .map(|i| sample(i, 1, Some(u32::try_from(i).unwrap() * 1_000)))
        .collect();
    let [rollup] = roll_up(&samples, Tier::Hour).try_into().unwrap();
    let p50 = rollup.histogram.quantile_us(0.5).unwrap();
    assert!((50_000..=55_000).contains(&p50), "p50 = {p50}");
    let p99 = rollup.histogram.quantile_us(0.99).unwrap();
    assert!((99_000..=108_900).contains(&p99), "p99 = {p99}");
}
