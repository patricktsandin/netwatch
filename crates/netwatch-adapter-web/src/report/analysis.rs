//! The report's analyses (ADR-0039, ADR-0040): totals over episodes, statistics by
//! period, why probes failed, loss bursts and jitter, link load, an episode's
//! timeline, and the raw samples as CSV. All pure.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use netwatch_history::{Histogram, Loss, Measurement, Reading, Rollup, Sample};
use netwatch_probing::incident::Severity;
use netwatch_shared::uom::si::time::millisecond;
use netwatch_shared::{Gauge, IdentityEpoch, Role, WallTime};
use netwatch_verified::Summary;

use super::{Episode, Place, utc};

/// Total and longest outage time beyond the gateway within `from..until`, with
/// overlapping episodes counted once. Open episodes end at `until`.
pub fn outage_beyond_gateway(episodes: &[Episode], from: WallTime, until: WallTime) -> (i64, i64) {
    let mut spans: Vec<(i64, i64)> = episodes
        .iter()
        .filter(|e| e.place == Place::BeyondGateway && e.severity == Severity::Outage)
        .map(|e| {
            let end = e.ended.unwrap_or(until).min(until);
            (e.started.max(from).as_unix_micros(), end.as_unix_micros())
        })
        .filter(|(start, end)| start < end)
        .collect();
    spans.sort_unstable();
    let (mut total, mut longest) = (0, 0);
    let mut current: Option<(i64, i64)> = None;
    for (start, end) in spans {
        current = match current {
            Some((s, e)) if start <= e => Some((s, e.max(end))),
            Some((s, e)) => {
                total += e - s;
                longest = longest.max(e - s);
                Some((start, end))
            }
            None => Some((start, end)),
        };
    }
    if let Some((s, e)) = current {
        total += e - s;
        longest = longest.max(e - s);
    }
    (total, longest)
}

/// Probes, loss, and RTT percentiles over one period.
#[derive(Debug, Clone, PartialEq)]
pub struct PeriodStats {
    /// The period's start.
    pub start: WallTime,
    /// Probes resolved.
    pub sent: u64,
    /// Of those, lost.
    pub lost: u64,
    /// Median RTT of the answered ones.
    pub median_ms: Option<f64>,
    /// 95th-percentile RTT of the answered ones.
    pub p95_ms: Option<f64>,
}

impl PeriodStats {
    /// Loss as a percentage of probes sent.
    pub fn loss_percent(&self) -> f64 {
        if self.sent == 0 {
            0.0
        } else {
            self.lost as f64 * 100.0 / self.sent as f64
        }
    }
}

/// Merges rollups into periods of `period_us`, oldest first, across every role given.
pub fn by_period(rollups: &[&Rollup], period_us: i64) -> Vec<PeriodStats> {
    let mut periods: BTreeMap<i64, (Summary, Histogram)> = BTreeMap::new();
    for r in rollups {
        let start = r.start.as_unix_micros().div_euclid(period_us) * period_us;
        let (summary, histogram) = periods
            .entry(start)
            .or_insert_with(|| (Summary::empty(), Histogram::default()));
        *summary = summary
            .merge(&r.summary)
            .expect("u64 sample counts cannot be exhausted");
        histogram.merge(&r.histogram);
    }
    periods
        .into_iter()
        .map(|(start, (summary, histogram))| PeriodStats {
            start: WallTime::from_unix_micros(start),
            sent: summary.count,
            lost: summary.lost,
            median_ms: histogram.quantile_us(0.5).map(|us| us as f64 / 1000.0),
            p95_ms: histogram.quantile_us(0.95).map(|us| us as f64 / 1000.0),
        })
        .collect()
}

/// Why a probe got no usable answer, in words.
pub fn loss_text(loss: Loss) -> String {
    match loss {
        Loss::TimedOut => "timed out".to_owned(),
        Loss::Unreachable(code) => format!("ICMP destination unreachable, code {code}"),
        Loss::TimeExceeded => "ICMP time exceeded (expired in transit)".to_owned(),
        Loss::Refused(code) => format!("DNS error, rcode {code}"),
        Loss::UnexpectedResponse(status) => format!("unexpected HTTP status {status}"),
        Loss::SendFailed => "could not be sent".to_owned(),
    }
}

/// Every role's failed probes, counted by reason.
pub fn loss_reasons(samples: &[Sample]) -> BTreeMap<Role, BTreeMap<String, u64>> {
    let mut reasons: BTreeMap<Role, BTreeMap<String, u64>> = BTreeMap::new();
    for s in samples {
        if let Measurement::Lost(loss) = s.measurement {
            *reasons
                .entry(s.role.clone())
                .or_default()
                .entry(loss_text(loss))
                .or_default() += 1;
        }
    }
    reasons
}

/// How one role's probes were lost, and how steady its answers were.
#[derive(Debug, Clone, PartialEq)]
pub struct LossPattern {
    /// Runs of consecutive lost probes (glossary: *loss burst*).
    pub bursts: u64,
    /// The longest run, in probes.
    pub longest: u64,
    /// Runs of 1, 2–4, 5–29, and 30 or more probes.
    pub by_length: [u64; 4],
    /// The mean absolute difference between consecutive answered RTTs (glossary:
    /// *jitter*), if at least two were answered.
    pub jitter_ms: Option<f64>,
}

/// The bucket of [`LossPattern::by_length`] a run of `length` probes falls in.
fn length_bucket(length: u64) -> usize {
    match length {
        0..=1 => 0,
        2..=4 => 1,
        5..=29 => 2,
        _ => 3,
    }
}

/// Every role's loss pattern, from its samples in time order.
pub fn loss_patterns(samples: &[Sample]) -> BTreeMap<Role, LossPattern> {
    let mut by_role: BTreeMap<Role, Vec<&Sample>> = BTreeMap::new();
    for s in samples {
        by_role.entry(s.role.clone()).or_default().push(s);
    }
    by_role
        .into_iter()
        .map(|(role, mut samples)| {
            samples.sort_by_key(|s| s.at);
            let mut pattern = LossPattern {
                bursts: 0,
                longest: 0,
                by_length: [0; 4],
                jitter_ms: None,
            };
            let mut run = 0;
            let end_run = |run: &mut u64, pattern: &mut LossPattern| {
                if *run > 0 {
                    pattern.bursts += 1;
                    pattern.longest = pattern.longest.max(*run);
                    pattern.by_length[length_bucket(*run)] += 1;
                    *run = 0;
                }
            };
            let (mut previous, mut sum, mut steps) = (None::<f64>, 0.0, 0u64);
            for s in samples {
                match s.measurement {
                    Measurement::Lost(_) => run += 1,
                    Measurement::RoundTrip(rtt) => {
                        end_run(&mut run, &mut pattern);
                        let ms = rtt.get::<millisecond>();
                        if let Some(before) = previous {
                            sum += (ms - before).abs();
                            steps += 1;
                        }
                        previous = Some(ms);
                    }
                }
            }
            end_run(&mut run, &mut pattern);
            pattern.jitter_ms = (steps > 0).then(|| sum / steps as f64);
            (role, pattern)
        })
        .collect()
}

/// The NIC's traffic over a stretch of time, against its link speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkLoad {
    /// Mean and peak receive throughput, in bit/s.
    pub rx: (f64, f64),
    /// Mean and peak transmit throughput, in bit/s.
    pub tx: (f64, f64),
}

/// The NIC's mean and peak throughput from its readings. `None` without any.
pub fn link_load(readings: &[Reading]) -> Option<LinkLoad> {
    let stats = |gauge: Gauge| {
        let values: Vec<f64> = readings
            .iter()
            .filter(|r| r.role == Role::Nic && r.gauge == gauge)
            .map(|r| r.value)
            .collect();
        (!values.is_empty()).then(|| {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            (mean, values.iter().copied().fold(0.0, f64::max))
        })
    };
    Some(LinkLoad {
        rx: stats(Gauge::RxThroughput)?,
        tx: stats(Gauge::TxThroughput)?,
    })
}

/// One step of an episode's timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineRow {
    /// The step's start.
    pub start: WallTime,
    /// The probable cause's probes and losses in this step.
    pub cause: (u64, u64),
    /// The affected targets' probes and losses, together.
    pub targets: (u64, u64),
}

/// An episode's timeline in steps of `step_us`, from the samples during it.
pub fn timeline(
    samples: &[Sample],
    cause: &Role,
    targets: &[Role],
    from: WallTime,
    step_us: i64,
) -> Vec<TimelineRow> {
    let mut rows: BTreeMap<i64, TimelineRow> = BTreeMap::new();
    for s in samples {
        let is_cause = s.role == *cause;
        let is_target = targets.contains(&s.role);
        if !is_cause && !is_target {
            continue;
        }
        let offset = (s.at.as_unix_micros() - from.as_unix_micros()).max(0) / step_us;
        let start = from.as_unix_micros() + offset * step_us;
        let row = rows.entry(start).or_insert_with(|| TimelineRow {
            start: WallTime::from_unix_micros(start),
            cause: (0, 0),
            targets: (0, 0),
        });
        let lost = u64::from(matches!(s.measurement, Measurement::Lost(_)));
        if is_cause {
            row.cause = (row.cause.0 + 1, row.cause.1 + lost);
        }
        if is_target {
            row.targets = (row.targets.0 + 1, row.targets.1 + lost);
        }
    }
    rows.into_values().collect()
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// The samples as CSV, one per line, with a header.
pub fn csv(
    samples: &[Sample],
    identities: &BTreeMap<IdentityEpoch, String>,
    label: &dyn Fn(&Role) -> String,
) -> String {
    let mut out = String::from("time_utc,unix_us,role,label,epoch,identity,rtt_ms,outcome\n");
    for s in samples {
        let (rtt, outcome) = match s.measurement {
            Measurement::RoundTrip(rtt) => (
                format!("{:.3}", rtt.get::<millisecond>()),
                "answered".to_owned(),
            ),
            Measurement::Lost(loss) => (String::new(), loss_text(loss)),
        };
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{}",
            utc(s.at).trim_end_matches(" UTC"),
            s.at.as_unix_micros(),
            s.role.key(),
            csv_field(&label(&s.role)),
            s.epoch.get(),
            csv_field(identities.get(&s.epoch).map_or("", String::as_str)),
            rtt,
            csv_field(&outcome),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use netwatch_history::{Tier, roll_up};
    use netwatch_shared::uom::si::f64::Time;

    use super::*;

    fn s(seconds: i64) -> WallTime {
        WallTime::from_unix_micros(seconds * 1_000_000)
    }

    fn role(key: &str) -> Role {
        Role::from_key(key).expect("a valid key")
    }

    fn episode(start: i64, end: Option<i64>, place: Place, severity: Severity) -> Episode {
        Episode {
            started: s(start),
            ended: end.map(s),
            severity,
            cause: role("hop:1:2"),
            affected: vec![],
            place,
        }
    }

    fn sample(key: &str, at: i64, measurement: Measurement) -> Sample {
        Sample {
            role: role(key),
            epoch: IdentityEpoch::new(NonZeroU64::MIN),
            at: s(at),
            measurement,
        }
    }

    fn rtt(ms: f64) -> Measurement {
        Measurement::RoundTrip(Time::new::<millisecond>(ms))
    }

    #[test]
    fn outage_time_counts_overlaps_once_and_only_outages_beyond_the_gateway() {
        let episodes = [
            episode(0, Some(60), Place::BeyondGateway, Severity::Outage),
            episode(30, Some(90), Place::BeyondGateway, Severity::Outage),
            episode(200, Some(210), Place::BeyondGateway, Severity::Outage),
            episode(300, Some(900), Place::HostOrLan, Severity::Outage),
            episode(400, Some(900), Place::BeyondGateway, Severity::Degradation),
            episode(950, None, Place::BeyondGateway, Severity::Outage),
        ];
        assert_eq!(
            outage_beyond_gateway(&episodes, s(0), s(1000)),
            (150_000_000, 90_000_000)
        );
        // Clipped to the span: only what falls inside it counts.
        assert_eq!(
            outage_beyond_gateway(&episodes, s(45), s(205)),
            (50_000_000, 45_000_000)
        );
    }

    #[test]
    fn periods_merge_rollups_of_every_role() {
        let samples = [
            sample("target:1", 10, rtt(10.0)),
            sample("target:2", 20, Measurement::Lost(Loss::TimedOut)),
            sample("target:1", 3700, rtt(30.0)),
        ];
        let rollups = roll_up(&samples, Tier::Minute);
        let periods = by_period(&rollups.iter().collect::<Vec<_>>(), 3_600_000_000);
        assert_eq!(periods.len(), 2);
        assert_eq!((periods[0].sent, periods[0].lost), (2, 1));
        assert!((periods[0].loss_percent() - 50.0).abs() < 1e-9);
        assert_eq!(periods[1].start, s(3600));
    }

    #[test]
    fn loss_reasons_count_each_kind_with_its_code() {
        let reasons = loss_reasons(&[
            sample("target:1", 1, Measurement::Lost(Loss::TimedOut)),
            sample("target:1", 2, Measurement::Lost(Loss::TimedOut)),
            sample("target:1", 3, Measurement::Lost(Loss::Unreachable(1))),
            sample("target:1", 4, rtt(5.0)),
        ]);
        let of_target = &reasons[&role("target:1")];
        assert_eq!(of_target["timed out"], 2);
        assert_eq!(of_target["ICMP destination unreachable, code 1"], 1);
    }

    #[test]
    fn loss_patterns_count_bursts_by_length_and_jitter_between_answers() {
        let lost = || Measurement::Lost(Loss::TimedOut);
        let patterns = loss_patterns(&[
            sample("target:1", 1, rtt(10.0)),
            sample("target:1", 2, lost()),
            sample("target:1", 3, rtt(14.0)),
            sample("target:1", 4, lost()),
            sample("target:1", 5, lost()),
            sample("target:1", 6, lost()),
            sample("target:1", 7, rtt(12.0)),
            // Out of order, and still the end of a run of one.
            sample("target:1", 9, lost()),
        ]);
        let p = &patterns[&role("target:1")];
        assert_eq!((p.bursts, p.longest, p.by_length), (3, 3, [2, 1, 0, 0]));
        // |14 − 10| and |12 − 14|.
        assert_eq!(p.jitter_ms, Some(3.0));
    }

    #[test]
    fn link_load_is_mean_and_peak_of_each_direction() {
        let reading = |gauge, value| Reading {
            role: Role::Nic,
            epoch: IdentityEpoch::new(NonZeroU64::MIN),
            at: s(0),
            gauge,
            value,
        };
        let load = link_load(&[
            reading(Gauge::RxThroughput, 2e6),
            reading(Gauge::RxThroughput, 4e6),
            reading(Gauge::TxThroughput, 1e6),
        ])
        .unwrap();
        assert_eq!((load.rx, load.tx), ((3e6, 4e6), (1e6, 1e6)));
        assert_eq!(link_load(&[]), None);
    }

    #[test]
    fn the_timeline_steps_through_the_cause_and_the_targets() {
        let rows = timeline(
            &[
                sample("hop:1:2", 5, Measurement::Lost(Loss::TimedOut)),
                sample("target:1", 6, Measurement::Lost(Loss::TimedOut)),
                sample("target:2", 65, rtt(4.0)),
                sample("hop:1:3", 66, rtt(4.0)),
            ],
            &role("hop:1:2"),
            &[role("target:1"), role("target:2")],
            s(0),
            60_000_000,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].cause, rows[0].targets), ((1, 1), (1, 1)));
        assert_eq!(
            (rows[1].start, rows[1].cause, rows[1].targets),
            (s(60), (0, 0), (1, 0))
        );
    }

    #[test]
    fn csv_quotes_fields_that_need_it() {
        let text = csv(
            &[sample("target:1", 0, rtt(1.5))],
            &BTreeMap::new(),
            &|_| "a, \"quoted\" name".to_owned(),
        );
        assert_eq!(
            text,
            "time_utc,unix_us,role,label,epoch,identity,rtt_ms,outcome\n\
             1970-01-01 00:00:00,0,target:1,\"a, \"\"quoted\"\" name\",1,,1.500,answered\n"
        );
    }
}
