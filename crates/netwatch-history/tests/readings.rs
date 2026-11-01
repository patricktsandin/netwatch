//! Reading rollups (ADR-0032): the same tiers as samples, with a merge that is
//! associative and commutative, so coarsening equals rolling up directly.

#![allow(clippy::unwrap_used)] // Tests may unwrap (ADR-0015), including in helpers.

use std::num::NonZeroU64;

use netwatch_history::{Reading, ReadingSummary, Tier, coarsen_readings, roll_up_readings};
use netwatch_shared::{Gauge, IdentityEpoch, Role, WallTime};
use proptest::prelude::*;

fn reading(at_s: i64, gauge: Gauge, value: f64) -> Reading {
    Reading {
        role: Role::Nic,
        epoch: IdentityEpoch::new(NonZeroU64::new(1).unwrap()),
        at: WallTime::from_unix_micros(at_s * 1_000_000),
        gauge,
        value,
    }
}

#[test]
fn a_minute_keeps_count_sum_min_and_max_per_gauge() {
    let rollups = roll_up_readings(
        &[
            reading(0, Gauge::RxThroughput, 10.0),
            reading(1, Gauge::RxThroughput, 30.0),
            reading(1, Gauge::TxThroughput, 5.0),
            reading(61, Gauge::RxThroughput, 7.0),
        ],
        Tier::Minute,
    );
    assert_eq!(rollups.len(), 3);
    let first = &rollups[0];
    assert_eq!(first.gauge, Gauge::RxThroughput);
    assert_eq!(first.start, WallTime::from_unix_micros(0));
    assert_eq!(
        first.summary,
        ReadingSummary {
            count: 2,
            sum: 40.0,
            min: 30.0_f64.min(10.0),
            max: 30.0
        }
    );
    assert_eq!(first.summary.mean(), Some(20.0));
}

fn summary() -> impl Strategy<Value = ReadingSummary> {
    proptest::collection::vec(-1000i32..1000, 0..5).prop_map(|values| {
        values
            .into_iter()
            .map(|v| ReadingSummary::of(f64::from(v)))
            .fold(ReadingSummary::empty(), |a, b| a.merge(&b))
    })
}

proptest! {
    #[test]
    fn merge_is_commutative_and_associative_with_empty_as_identity(
        a in summary(), b in summary(), c in summary()
    ) {
        prop_assert_eq!(a.merge(&b), b.merge(&a));
        prop_assert_eq!(a.merge(&b).merge(&c), a.merge(&b.merge(&c)));
        prop_assert_eq!(a.merge(&ReadingSummary::empty()), a);
    }

    #[test]
    fn coarsening_minutes_equals_rolling_up_hours_directly(
        values in proptest::collection::vec((0i64..7200, -100i32..100), 0..40)
    ) {
        let readings: Vec<Reading> = values
            .iter()
            .map(|&(at, v)| reading(at, Gauge::TxThroughput, f64::from(v)))
            .collect();
        prop_assert_eq!(
            coarsen_readings(&roll_up_readings(&readings, Tier::Minute), Tier::Hour),
            roll_up_readings(&readings, Tier::Hour)
        );
    }
}
