//! Writes a [`Report`] for people (ADR-0033, ADR-0039, ADR-0040). [`compose`] decides what the
//! report says, once, as a list of [`Block`]s; [`text`] lays them out in fixed-width
//! columns for an email body, and [`html`] as a printable, self-contained page that
//! carries the same text for copying.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use netwatch_adapter_sqlite::StoredTrace;
use netwatch_probing::incident::Severity;
use netwatch_probing::trace::{TraceKind, TracePath};
use netwatch_shared::uom::si::time::millisecond;
use netwatch_shared::{Role, WallTime};

use super::analysis::{LinkLoad, LossPattern, PeriodStats, TimelineRow, outage_beyond_gateway};
use super::{Episode, PathStats, Place, format_duration, utc};

/// One episode, with everything measured about it.
#[derive(Debug, Clone)]
pub struct EpisodeReport {
    /// The episode.
    pub episode: Episode,
    /// Every role on the affected paths, measured during it.
    pub path: Vec<PathStats>,
    /// Each path role's median RTT in the hour before it started, by role and
    /// identity: a baseline is only for the same router (ADR-0003).
    pub before: BTreeMap<(Role, Option<String>), f64>,
    /// Why probes failed during it, by role. Empty once raw samples have expired.
    pub reasons: BTreeMap<Role, BTreeMap<String, u64>>,
    /// Its timeline, in steps of `step_us`.
    pub timeline: Vec<TimelineRow>,
    /// The timeline's step.
    pub step_us: i64,
    /// Whether raw samples still existed for it.
    pub raw: bool,
    /// Each path role's loss bursts and jitter. Empty once raw samples have expired.
    pub patterns: BTreeMap<Role, LossPattern>,
    /// The NIC's traffic during it, if readings still exist.
    pub load: Option<LinkLoad>,
    /// The episode traces of its targets that it asked for: those that started during
    /// it, or soon after, as they wait for the path to settle (ADR-0059).
    pub traces: Vec<StoredTrace>,
    /// For each traced target and path, the latest routine trace on that path before
    /// it began.
    pub baselines: BTreeMap<(Role, Option<TracePath>), StoredTrace>,
}

/// One event in the span.
#[derive(Debug, Clone)]
pub struct EventLine {
    /// When it happened.
    pub at: WallTime,
    /// What kind of event, for people.
    pub kind: String,
    /// The role it names, if any.
    pub role: Option<Role>,
    /// What happened.
    pub detail: String,
}

/// Everything a report says, decided by the daemon (ADR-0033, ADR-0039).
#[derive(Debug, Clone)]
pub struct Report {
    /// The span's start.
    pub from: WallTime,
    /// The span's end.
    pub until: WallTime,
    /// When the report was made.
    pub generated: WallTime,
    /// The host's offset from UTC, in seconds, so times can be read locally too.
    pub utc_offset_s: i64,
    /// The host and the report: hostname, kernel, version, clock.
    pub about: Vec<(String, String)>,
    /// The customer's ticket details, as typed in for this report (ADR-0040).
    pub ticket: Vec<(String, String)>,
    /// The customer's description of the problem.
    pub description: Option<String>,
    /// Router names, by address, for every identity that has one.
    pub names: BTreeMap<String, String>,
    /// The NIC's negotiated speed, in Mb/s.
    pub link_speed_mbps: Option<u32>,
    /// Each traced target's latest routine trace before the span ended.
    pub normal_paths: Vec<StoredTrace>,
    /// The connection: link, addresses, gateways, ISP routers, public identity.
    pub connection: Vec<(String, String)>,
    /// How measurements are taken, one sentence each.
    pub method: Vec<String>,
    /// Every episode in the span.
    pub episodes: Vec<EpisodeReport>,
    /// Every role's measurements over the whole span.
    pub summary: Vec<PathStats>,
    /// How long this host had no way out in the span, in µs (ADR-0053).
    pub without_a_way_out_us: i64,
    /// All targets together, by period.
    pub periods: Vec<PeriodStats>,
    /// `Hour` or `Day`.
    pub period: &'static str,
    /// Every event in the span, oldest first.
    pub events: Vec<EventLine>,
    /// Where the raw samples are, as CSV.
    pub csv_href: String,
}

/// How roles are named, for people: target names come from the configuration.
pub type Label<'a> = &'a dyn Fn(&Role) -> String;

/// One piece of a composed report.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// A section heading.
    Heading(String),
    /// Labeled facts.
    Facts(Vec<(String, String)>),
    /// A paragraph.
    Para(String),
    /// A table: its header, then rows, each flagged when it shows loss. Columns from
    /// `numeric` on are right-aligned.
    Table {
        /// Column headings.
        head: Vec<String>,
        /// Cells, and whether the row shows loss.
        rows: Vec<(Vec<String>, bool)>,
        /// The first right-aligned column.
        numeric: usize,
    },
    /// An episode, framed as an outage or a degradation.
    Episode {
        /// An outage, rather than a degradation.
        outage: bool,
        /// Its heading.
        title: String,
        /// What it says.
        blocks: Vec<Block>,
    },
    /// A link to a related resource.
    Link {
        /// What it is.
        text: String,
        /// Where it is.
        href: String,
    },
}

fn severity_text(severity: Severity) -> &'static str {
    match severity {
        Severity::Outage => "Outage",
        Severity::Degradation => "Degradation",
    }
}

fn place_text(place: Place) -> &'static str {
    match place {
        Place::BeyondGateway => "Beyond the gateway",
        Place::HostOrLan => "On this host or LAN",
    }
}

fn ms(value: Option<f64>) -> String {
    value.map_or_else(|| "–".to_owned(), |v| format!("{v:.1} ms"))
}

fn percent(lost: u64, sent: u64) -> String {
    if sent == 0 {
        "–".to_owned()
    } else {
        format!("{:.1}%", lost as f64 * 100.0 / sent as f64)
    }
}

fn short(at: WallTime) -> String {
    utc(at).trim_end_matches(" UTC").to_owned()
}

/// A UTC time, with the host's local time beside it when the offset isn't zero.
fn when_local(at: WallTime, offset_s: i64) -> String {
    if offset_s == 0 {
        return utc(at);
    }
    let local = short(WallTime::from_unix_micros(
        at.as_unix_micros() + offset_s * 1_000_000,
    ));
    let sign = if offset_s < 0 { '-' } else { '+' };
    let (h, m) = (offset_s.abs() / 3600, offset_s.abs() % 3600 / 60);
    format!("{} ({local} UTC{sign}{h:02}:{m:02})", utc(at))
}

fn strings<const N: usize>(cells: [&str; N]) -> Vec<String> {
    cells.map(String::from).to_vec()
}

fn stats_table(
    stats: &[PathStats],
    before: Option<&BTreeMap<(Role, Option<String>), f64>>,
    names: &BTreeMap<String, String>,
    label: Label<'_>,
) -> Block {
    let name = |s: &PathStats| s.identity.as_ref().and_then(|i| names.get(i)).cloned();
    let named = stats.iter().any(|s| name(s).is_some());
    let mut head = strings(["Role", "Identity"]);
    if named {
        head.push("Router name".to_owned());
    }
    let numeric = head.len();
    head.extend(strings(["Sent", "Loss", "Median", "p95"]));
    if before.is_some() {
        head.push("Median before".to_owned());
    }
    Block::Table {
        head,
        rows: stats
            .iter()
            .map(|s| {
                let mut row = vec![
                    label(&s.role),
                    s.identity.clone().unwrap_or_else(|| "–".to_owned()),
                ];
                if named {
                    row.push(name(s).unwrap_or_else(|| "–".to_owned()));
                }
                row.extend([
                    s.sent.to_string(),
                    format!("{:.1}%", s.loss_percent()),
                    ms(s.median_ms),
                    ms(s.p95_ms),
                ]);
                if let Some(before) = before {
                    row.push(ms(before
                        .get(&(s.role.clone(), s.identity.clone()))
                        .copied()));
                }
                (row, s.lost > 0)
            })
            .collect(),
        numeric,
    }
}

fn kind_text(kind: TraceKind) -> &'static str {
    match kind {
        TraceKind::Routine => "Routine",
        TraceKind::Episode => "Episode",
    }
}

fn addresses(responders: &[std::net::IpAddr]) -> String {
    if responders.is_empty() {
        "*".to_owned()
    } else {
        responders
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A path trace as a table, beside `baseline` (a routine trace of the same path)
/// if given, with every TTL whose routers differ from it marked.
fn trace_blocks(
    trace: &StoredTrace,
    baseline: Option<&StoredTrace>,
    offset_s: i64,
    label: Label<'_>,
) -> Vec<Block> {
    let target = label(&trace.role);
    let toward = if target.contains(&trace.destination.to_string()) {
        target
    } else {
        format!("{target} ({})", trace.destination)
    };
    let mut intro = format!(
        "{} path trace toward {toward}, beyond hop 3, at {}",
        kind_text(trace.kind),
        when_local(trace.at, offset_s)
    );
    if let Some(b) = baseline {
        let _ = write!(intro, ", beside the routine trace at {}", utc(b.at));
    }
    intro.push(':');
    let mut head = strings(["TTL", "Router", "Name", "Answered", "Best", "Median"]);
    if baseline.is_some() {
        head.push("Routine trace".to_owned());
    }
    let rtt =
        |t: Option<netwatch_shared::uom::si::f64::Time>| ms(t.map(|t| t.get::<millisecond>()));
    let rows = trace
        .trace
        .hops
        .iter()
        .map(|hop| {
            let names: Vec<&str> = hop
                .responders
                .iter()
                .filter_map(|a| trace.names.get(a).map(String::as_str))
                .collect();
            let mut row = vec![
                hop.ttl.get().to_string(),
                addresses(&hop.responders),
                if names.is_empty() {
                    "–".to_owned()
                } else {
                    names.join(", ")
                },
                format!("{}/{}", hop.answered, hop.sent),
                rtt(hop.best),
                rtt(hop.median),
            ];
            let mut changed = false;
            if let Some(b) = baseline {
                let before = b.trace.hops.iter().find(|h| h.ttl == hop.ttl);
                let was = before.map_or_else(Vec::new, |h| h.responders.clone());
                // A silent TTL, then or now, is no evidence of a different router.
                changed = !was.is_empty() && !hop.responders.is_empty() && was != hop.responders;
                row.push(match (before, changed) {
                    (None, _) => "not reached".to_owned(),
                    (Some(_), true) => format!("changed, was {}", addresses(&was)),
                    (Some(_), false) => "same".to_owned(),
                });
            }
            (row, changed)
        })
        .collect();
    let mut blocks = vec![Block::Para(intro)];
    let end = match (
        trace.trace.reached,
        trace.trace.hops.last(),
        trace.trace.silent_from,
    ) {
        (true, Some(last), _) if trace.trace.hops.len() == 1 && last.ttl.get().get() == 4 => Some(
            "The destination is within 4 hops, so hops 1 to 3 above are its whole path.".to_owned(),
        ),
        (true, Some(last), _) => Some(format!(
            "The destination answered at TTL {}.",
            last.ttl.get()
        )),
        (_, _, Some(from)) => Some(format!(
            "No router answered from TTL {} up to 30.",
            from.get()
        )),
        _ => None,
    };
    if !trace.trace.hops.is_empty() {
        blocks.push(Block::Table {
            head,
            rows,
            numeric: 3,
        });
    }
    blocks.extend(end.map(Block::Para));
    blocks
}

fn rate(bits_per_second: f64) -> String {
    if bits_per_second >= 1e6 {
        format!("{:.1} Mb/s", bits_per_second / 1e6)
    } else {
        format!("{:.0} kb/s", bits_per_second / 1e3)
    }
}

/// The link load as one line: `mean 3.1 Mb/s, peak 9.8 Mb/s (10% of 100 Mb/s)`.
fn load_text((mean, peak): (f64, f64), speed_mbps: Option<u32>) -> String {
    let share = speed_mbps
        .filter(|&s| s > 0)
        .map(|s| format!(" ({:.0}% of {s} Mb/s)", peak * 100.0 / (f64::from(s) * 1e6)))
        .unwrap_or_default();
    format!("mean {}, peak {}{share}", rate(mean), rate(peak))
}

fn episode_blocks(r: &EpisodeReport, report: &Report, label: Label<'_>) -> Vec<Block> {
    let e = &r.episode;
    let end = e.ended.unwrap_or(report.until);
    let mut facts = vec![
        (
            "Started".to_owned(),
            when_local(e.started, report.utc_offset_s),
        ),
        (
            "Ended".to_owned(),
            e.ended.map_or_else(
                || "ongoing".to_owned(),
                |t| when_local(t, report.utc_offset_s),
            ),
        ),
        (
            "Duration".to_owned(),
            format_duration(end.as_unix_micros() - e.started.as_unix_micros()),
        ),
        ("Probable cause".to_owned(), label(&e.cause)),
    ];
    if let Some(identity) = r
        .path
        .iter()
        .find(|s| s.role == e.cause)
        .and_then(|s| s.identity.clone())
    {
        facts.push(("Cause's identity".to_owned(), identity));
    }
    facts.push((
        "Affected".to_owned(),
        e.affected.iter().map(label).collect::<Vec<_>>().join(", "),
    ));
    if let Some(load) = r.load {
        facts.push((
            "Link load, receive".to_owned(),
            load_text(load.rx, report.link_speed_mbps),
        ));
        facts.push((
            "Link load, transmit".to_owned(),
            load_text(load.tx, report.link_speed_mbps),
        ));
    }
    let mut blocks = vec![
        Block::Facts(facts),
        Block::Para(
            "Path during the episode, with each role's median in the hour before it:".to_owned(),
        ),
        stats_table(&r.path, Some(&r.before), &report.names, label),
    ];
    for trace in &r.traces {
        blocks.extend(trace_blocks(
            trace,
            r.baselines.get(&(trace.role.clone(), trace.path.clone())),
            report.utc_offset_s,
            label,
        ));
    }
    if r.traces.is_empty() {
        for baseline in r.baselines.values() {
            blocks.push(Block::Para(
                "No episode trace ran during it. The path before it:".to_owned(),
            ));
            blocks.extend(trace_blocks(baseline, None, report.utc_offset_s, label));
        }
    }
    if !r.raw {
        blocks.push(Block::Para(
            "Raw samples for this episode have expired (they are kept 7 days), so why probes failed, loss \
             bursts, jitter, and the timeline are not available; the path figures come from rollups."
                .to_owned(),
        ));
        return blocks;
    }
    if !r.reasons.is_empty() {
        blocks.push(Block::Para("Why probes failed:".to_owned()));
        blocks.push(Block::Table {
            head: strings(["Role", "Reason", "Probes"]),
            rows: r
                .reasons
                .iter()
                .flat_map(|(role, reasons)| {
                    reasons.iter().map(move |(reason, n)| {
                        (vec![label(role), reason.clone(), n.to_string()], true)
                    })
                })
                .collect(),
            numeric: 2,
        });
    }
    if !r.patterns.is_empty() {
        blocks.push(Block::Para(
            "Loss pattern: runs of consecutive lost probes, by length, and jitter (mean change between \
             consecutive answers):"
                .to_owned(),
        ));
        blocks.push(Block::Table {
            head: strings([
                "Role", "Runs", "Longest", "1", "2–4", "5–29", "30+", "Jitter",
            ]),
            rows: r
                .path
                .iter()
                .filter_map(|s| Some((&s.role, r.patterns.get(&s.role)?)))
                .map(|(role, p)| {
                    let mut row = vec![label(role), p.bursts.to_string(), p.longest.to_string()];
                    row.extend(p.by_length.iter().map(ToString::to_string));
                    row.push(ms(p.jitter_ms));
                    (row, p.bursts > 0)
                })
                .collect(),
            numeric: 1,
        });
    }
    if !r.timeline.is_empty() {
        blocks.push(Block::Para(format!(
            "Timeline, in {} steps, of the probable cause and the affected targets:",
            format_duration(r.step_us)
        )));
        blocks.push(Block::Table {
            head: strings([
                "From (UTC)",
                "Cause probes",
                "Cause loss",
                "Target probes",
                "Target loss",
            ]),
            rows: r
                .timeline
                .iter()
                .map(|t| {
                    (
                        vec![
                            short(t.start),
                            t.cause.0.to_string(),
                            percent(t.cause.1, t.cause.0),
                            t.targets.0.to_string(),
                            percent(t.targets.1, t.targets.0),
                        ],
                        t.cause.1 + t.targets.1 > 0,
                    )
                })
                .collect(),
            numeric: 1,
        });
    }
    blocks
}

/// What the report says, in order (ADR-0039).
pub fn compose(report: &Report, label: Label<'_>) -> Vec<Block> {
    let span = report.until.as_unix_micros() - report.from.as_unix_micros();
    let mut blocks = vec![Block::Facts(
        [
            (
                "Span".to_owned(),
                format!(
                    "{} – {} ({})",
                    utc(report.from),
                    utc(report.until),
                    format_duration(span)
                ),
            ),
            (
                "Generated".to_owned(),
                when_local(report.generated, report.utc_offset_s),
            ),
        ]
        .into_iter()
        .chain(report.about.iter().cloned())
        .collect(),
    )];
    if !report.ticket.is_empty() || report.description.is_some() {
        blocks.push(Block::Heading("Customer and ticket".to_owned()));
        if !report.ticket.is_empty() {
            blocks.push(Block::Facts(report.ticket.clone()));
        }
        blocks.extend(
            report
                .description
                .iter()
                .map(|d| Block::Para(format!("What the customer noticed: {d}"))),
        );
    }

    let mut episodes: Vec<&EpisodeReport> = report.episodes.iter().collect();
    episodes.sort_by_key(|r| (r.episode.place, r.episode.started));
    let beyond = episodes
        .iter()
        .filter(|r| r.episode.place == Place::BeyondGateway)
        .count();
    let outages = episodes
        .iter()
        .filter(|r| {
            r.episode.place == Place::BeyondGateway && r.episode.severity == Severity::Outage
        })
        .count();
    let all: Vec<Episode> = report.episodes.iter().map(|r| r.episode.clone()).collect();
    let (total, longest) = outage_beyond_gateway(&all, report.from, report.until);
    blocks.push(Block::Heading("Summary".to_owned()));
    blocks.push(Block::Facts(vec![
        (
            "Beyond the gateway".to_owned(),
            format!(
                "{beyond} episode(s): {outages} outage(s), {} degradation(s)",
                beyond - outages
            ),
        ),
        (
            "On this host or LAN".to_owned(),
            format!("{} episode(s)", episodes.len() - beyond),
        ),
        (
            "Outage time beyond the gateway".to_owned(),
            format!(
                "{} in total ({:.3}% of the span), longest {}",
                format_duration(total),
                if span > 0 {
                    total as f64 * 100.0 / span as f64
                } else {
                    0.0
                },
                format_duration(longest)
            ),
        ),
        (
            "Time without a way out".to_owned(),
            format!(
                "{} (no route at all from this host, so nothing could be sent)",
                format_duration(report.without_a_way_out_us)
            ),
        ),
    ]));
    // One row per target. Probes and availability are counts, which add up across its
    // epochs. A median doesn't: the RTTs are of the path it was probed on most, over
    // all that path's epochs, and of that path's largest epoch (ADR-0057, ADR-0058).
    let mut targets: BTreeMap<Role, Vec<&PathStats>> = BTreeMap::new();
    for s in report
        .summary
        .iter()
        .filter(|s| matches!(s.role, Role::Target(_)))
    {
        targets.entry(s.role.clone()).or_default().push(s);
    }
    if !targets.is_empty() {
        blocks.push(Block::Table {
            head: strings([
                "Target",
                "Main path",
                "Probes",
                "Availability",
                "Median",
                "p95",
            ]),
            rows: targets
                .iter()
                .filter_map(|(role, epochs)| {
                    let mut by_path: BTreeMap<Option<&String>, u64> = BTreeMap::new();
                    for s in epochs {
                        *by_path.entry(s.identity.as_ref()).or_default() += s.sent;
                    }
                    let (&main_path, _) = by_path.iter().max_by_key(|&(_, &sent)| sent)?;
                    let main = epochs
                        .iter()
                        .filter(|s| s.identity.as_ref() == main_path)
                        .max_by_key(|s| s.sent)?;
                    let sent: u64 = epochs.iter().map(|s| s.sent).sum();
                    let lost: u64 = epochs.iter().map(|s| s.lost).sum();
                    let availability = if sent == 0 {
                        "–".to_owned()
                    } else {
                        format!("{:.3}%", 100.0 - lost as f64 * 100.0 / sent as f64)
                    };
                    let path = main.identity.clone().unwrap_or_else(|| "–".to_owned());
                    let others = match by_path.len() - 1 {
                        0 => String::new(),
                        n => format!(" (and {n} other path(s))"),
                    };
                    Some((
                        vec![
                            label(role),
                            format!("{path}{others}"),
                            sent.to_string(),
                            availability,
                            ms(main.median_ms),
                            ms(main.p95_ms),
                        ],
                        lost > 0,
                    ))
                })
                .collect(),
            numeric: 2,
        });
        blocks.push(Block::Para(
            "Availability is the share of probes answered. Time without a way out isn't in \
             it: no probe could be sent then. Median and p95 are of the target's main path: \
             a median across paths would describe none of them."
                .to_owned(),
        ));
    }

    blocks.push(Block::Heading("Connection".to_owned()));
    blocks.push(Block::Facts(report.connection.clone()));

    blocks.push(Block::Heading("Episodes".to_owned()));
    if episodes.is_empty() {
        blocks.push(Block::Para("No incidents in this span.".to_owned()));
    }
    for (n, r) in episodes.iter().enumerate() {
        blocks.push(Block::Episode {
            outage: r.episode.severity == Severity::Outage,
            title: format!(
                "{}. {} · {}",
                n + 1,
                severity_text(r.episode.severity),
                place_text(r.episode.place)
            ),
            blocks: episode_blocks(r, report, label),
        });
    }

    if !report.periods.is_empty() {
        blocks.push(Block::Heading(format!(
            "All targets by {} (UTC)",
            report.period.to_lowercase()
        )));
        blocks.push(Block::Table {
            head: strings([report.period, "Probes", "Loss"]),
            rows: report
                .periods
                .iter()
                .map(|p| {
                    (
                        vec![
                            short(p.start),
                            p.sent.to_string(),
                            format!("{:.2}%", p.loss_percent()),
                        ],
                        p.lost > 0,
                    )
                })
                .collect(),
            numeric: 1,
        });
    }

    if !report.normal_paths.is_empty() {
        blocks.push(Block::Heading("Normal paths".to_owned()));
        blocks.push(Block::Para(
            "Each target's latest routine path trace, which runs every 6 hours:".to_owned(),
        ));
        // Targets that share an address (an ICMP and a DNS target, say) share a path.
        let mut shown = std::collections::BTreeSet::new();
        for trace in report
            .normal_paths
            .iter()
            .filter(|t| shown.insert(t.destination))
        {
            blocks.extend(trace_blocks(trace, None, report.utc_offset_s, label));
        }
    }

    blocks.push(Block::Heading("Every role over the whole span".to_owned()));
    blocks.push(stats_table(&report.summary, None, &report.names, label));

    blocks.push(Block::Heading("Events".to_owned()));
    if report.events.is_empty() {
        blocks.push(Block::Para("No events in this span.".to_owned()));
    } else {
        blocks.push(Block::Table {
            head: strings(["Time (UTC)", "Event", "Role", "Detail"]),
            rows: report
                .events
                .iter()
                .map(|e| {
                    (
                        vec![
                            short(e.at),
                            e.kind.clone(),
                            e.role.as_ref().map_or_else(String::new, label),
                            e.detail.clone(),
                        ],
                        false,
                    )
                })
                .collect(),
            numeric: 4,
        });
    }

    blocks.push(Block::Heading("How this was measured".to_owned()));
    blocks.extend(report.method.iter().cloned().map(Block::Para));
    blocks.push(Block::Link {
        text: "Raw samples around every episode (CSV)".to_owned(),
        href: report.csv_href.clone(),
    });
    blocks
}

fn text_table(
    out: &mut String,
    indent: &str,
    head: &[String],
    rows: &[(Vec<String>, bool)],
    numeric: usize,
) {
    let widths: Vec<usize> = (0..head.len())
        .map(|i| {
            rows.iter()
                .filter_map(|(r, _)| r.get(i))
                .chain(head.get(i))
                .map(|c| c.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: &[String]| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let w = widths.get(i).copied().unwrap_or(0);
                if i >= numeric {
                    format!("{c:>w$}")
                } else {
                    format!("{c:<w$}")
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    let _ = writeln!(out, "{indent}{}", line(head));
    for (row, _) in rows {
        let _ = writeln!(out, "{indent}{}", line(row));
    }
}

fn text_blocks(out: &mut String, blocks: &[Block], indent: &str) {
    for block in blocks {
        match block {
            Block::Heading(h) => {
                let _ = writeln!(
                    out,
                    "\n{indent}{h}\n{indent}{}",
                    "=".repeat(h.chars().count())
                );
            }
            Block::Facts(facts) => {
                let w = facts
                    .iter()
                    .map(|(k, _)| k.chars().count())
                    .max()
                    .unwrap_or(0)
                    + 1;
                for (k, v) in facts {
                    let _ = writeln!(out, "{indent}{:<w$} {v}", format!("{k}:"));
                }
            }
            Block::Para(p) => {
                let _ = writeln!(out, "{indent}{}", p.replace('\n', &format!("\n{indent}")));
            }
            Block::Table {
                head,
                rows,
                numeric,
            } => text_table(out, indent, head, rows, *numeric),
            Block::Episode { title, blocks, .. } => {
                let _ = writeln!(out, "\n{indent}{title}");
                text_blocks(out, blocks, &format!("{indent}   "));
            }
            Block::Link { text, href } => {
                let _ = writeln!(out, "{indent}{text}: {href}");
            }
        }
    }
}

/// The report as plain text, laid out in fixed-width columns for an email body.
pub fn text(report: &Report, label: Label<'_>) -> String {
    let mut out = String::from("Network incident report (netwatch)\n");
    text_blocks(&mut out, &compose(report, label), "");
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_blocks(out: &mut String, blocks: &[Block]) {
    for block in blocks {
        match block {
            Block::Heading(h) => {
                let _ = write!(out, "<h2>{}</h2>", escape(h));
            }
            Block::Facts(facts) => {
                out.push_str("<dl>");
                for (k, v) in facts {
                    let _ = write!(out, "<dt>{}</dt><dd>{}</dd>", escape(k), escape(v));
                }
                out.push_str("</dl>");
            }
            Block::Para(p) => {
                let _ = write!(out, "<p>{}</p>", escape(p).replace('\n', "<br>"));
            }
            Block::Table {
                head,
                rows,
                numeric,
            } => {
                out.push_str("<div class=\"scroll\"><table><thead><tr>");
                for (i, h) in head.iter().enumerate() {
                    let class = if i >= *numeric { " class=\"n\"" } else { "" };
                    let _ = write!(out, "<th{class}>{}</th>", escape(h));
                }
                out.push_str("</tr></thead><tbody>");
                for (row, lossy) in rows {
                    out.push_str(if *lossy {
                        "<tr class=\"lossy\">"
                    } else {
                        "<tr>"
                    });
                    for (i, c) in row.iter().enumerate() {
                        let class = if i >= *numeric { " class=\"n\"" } else { "" };
                        let _ = write!(out, "<td{class}>{}</td>", escape(c));
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</tbody></table></div>");
            }
            Block::Episode {
                outage,
                title,
                blocks,
            } => {
                let kind = if *outage { "outage" } else { "degradation" };
                let _ = write!(
                    out,
                    "<section class=\"episode {kind}\"><h3>{}</h3>",
                    escape(title)
                );
                html_blocks(out, blocks);
                out.push_str("</section>");
            }
            Block::Link { text, href } => {
                let _ = write!(
                    out,
                    "<p><a href=\"{}\">{}</a></p>",
                    escape(href),
                    escape(text)
                );
            }
        }
    }
}

const STYLE: &str = "
:root { --bg: #fff; --text: #1d1f23; --muted: #6b7079; --border: #e4e4e1; --outage: #d9342b;
  --degradation: #b7791f; --lossy: #fdf0ef; --link: #2f6fdf; color-scheme: light; }
@media (prefers-color-scheme: dark) { :root:not([data-theme=\"light\"]) { --bg: #131417; --text: #e8e9ec;
  --muted: #9198a3; --border: #2c2f35; --outage: #ff6b61; --degradation: #e0a948; --lossy: #3a2321;
  --link: #6aa0ff; color-scheme: dark; } }
:root[data-theme=\"dark\"] { --bg: #131417; --text: #e8e9ec; --muted: #9198a3; --border: #2c2f35;
  --outage: #ff6b61; --degradation: #e0a948; --lossy: #3a2321; --link: #6aa0ff; color-scheme: dark; }
body { margin: 0 auto; max-width: 1040px; padding: 24px 16px; background: var(--bg); color: var(--text);
  font: 14px/1.45 system-ui, sans-serif; }
h1 { font-size: 20px; margin: 0 0 8px; } h2 { font-size: 16px; margin: 28px 0 8px; }
a { color: var(--link); }
dl { display: grid; grid-template-columns: max-content 1fr; gap: 2px 14px; margin: 8px 0; }
dt { color: var(--muted); } dd { margin: 0; overflow-wrap: anywhere; }
.episode { border: 1px solid var(--border); border-left: 4px solid var(--degradation); border-radius: 6px;
  padding: 10px 14px; margin: 12px 0; break-inside: avoid; }
.episode.outage { border-left-color: var(--outage); }
.episode h3 { margin: 0 0 4px; font-size: 15px; }
table { border-collapse: collapse; width: 100%; margin: 6px 0 10px; font-variant-numeric: tabular-nums; }
th, td { text-align: left; padding: 3px 8px; border-bottom: 1px solid var(--border); white-space: nowrap; }
th.n, td.n { text-align: right; }
tr.lossy td { background: var(--lossy); }
.scroll { overflow-x: auto; }
pre { white-space: pre; overflow-x: auto; border: 1px solid var(--border); border-radius: 6px; padding: 10px; }
button, select { font: inherit; padding: 4px 12px; border: 1px solid var(--border); border-radius: 6px;
  background: var(--bg); color: var(--text); cursor: pointer; }
@media print { button, select, details { display: none; } body { max-width: none; } }
";

/// The theme the UI's switch saved (same origin, so same storage), applied before
/// the first paint.
const THEME: &str = "<script>try { const t = localStorage.getItem('netwatch-theme'); \
if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t; } catch (e) {}</script>";

/// The same theme switch as the UI's, sharing its saved choice.
const THEME_SWITCH: &str = "const theme = document.getElementById('theme'); \
theme.value = document.documentElement.dataset.theme || 'auto'; \
theme.onchange = () => { \
if (theme.value === 'auto') delete document.documentElement.dataset.theme; \
else document.documentElement.dataset.theme = theme.value; \
try { localStorage.setItem('netwatch-theme', theme.value); } catch (e) {} };";

/// The report as a self-contained HTML page: printable (or saved as PDF from the
/// browser), with the plain text alongside for copying into an email.
pub fn html(report: &Report, label: Label<'_>) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Incident report</title><style>{STYLE}</style>{THEME}</head><body>\
         <h1>Network incident report</h1>\
         <p><button id=\"copy\">Copy as text</button> \
         <button onclick=\"print()\">Print or save as PDF</button> \
         <select id=\"theme\" aria-label=\"Theme\"><option value=\"auto\">Auto</option>\
         <option value=\"light\">Light</option><option value=\"dark\">Dark</option></select></p>"
    );
    html_blocks(&mut out, &compose(report, label));
    let _ = write!(
        out,
        "<details><summary>Plain text</summary><pre id=\"text\">{}</pre></details>\
         <script>document.getElementById('copy').onclick = () => \
         navigator.clipboard.writeText(document.getElementById('text').textContent);\
         {THEME_SWITCH}</script>\
         </body></html>",
        escape(&text(report, label))
    );
    out
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;
    use std::num::NonZeroU8;

    use netwatch_probing::trace::{PathTrace, TraceHop};
    use netwatch_shared::{HopTtl, TargetId};

    use super::*;

    fn report() -> Report {
        Report {
            from: WallTime::from_unix_micros(0),
            until: WallTime::from_unix_micros(3_600_000_000),
            generated: WallTime::from_unix_micros(3_600_000_000),
            utc_offset_s: 0,
            about: vec![],
            ticket: vec![],
            description: None,
            names: BTreeMap::new(),
            link_speed_mbps: Some(100),
            normal_paths: vec![],
            connection: vec![],
            method: vec![],
            episodes: vec![],
            summary: vec![],
            without_a_way_out_us: 0,
            periods: vec![],
            period: "Hour",
            events: vec![],
            csv_href: String::new(),
        }
    }

    fn label(role: &Role) -> String {
        role.key()
    }

    fn trace(kind: TraceKind, routers: &[(u8, &str)]) -> StoredTrace {
        StoredTrace {
            role: Role::Target(TargetId::new(1)),
            at: WallTime::from_unix_micros(0),
            kind,
            destination: "1.1.1.1".parse().unwrap(),
            trace: PathTrace {
                hops: routers
                    .iter()
                    .map(|&(ttl, router)| TraceHop {
                        ttl: HopTtl::new(NonZeroU8::new(ttl).unwrap()),
                        responders: router.parse::<IpAddr>().into_iter().collect(),
                        sent: 3,
                        answered: 3,
                        best: None,
                        median: None,
                    })
                    .collect(),
                reached: true,
                silent_from: None,
            },
            names: BTreeMap::new(),
            path: None,
        }
    }

    #[test]
    fn a_trace_marks_routers_that_differ_from_the_routine_trace() {
        let routine = trace(
            TraceKind::Routine,
            &[(4, "10.0.0.4"), (5, "10.0.0.5"), (6, "1.1.1.1")],
        );
        let during = trace(
            TraceKind::Episode,
            &[(4, "10.0.0.4"), (5, "10.9.9.9"), (6, "silent")],
        );
        let blocks = trace_blocks(&during, Some(&routine), 0, &label);
        let Block::Table { rows, .. } = &blocks[1] else {
            panic!("a table follows the introduction: {blocks:?}");
        };
        let marks: Vec<(&str, bool)> = rows
            .iter()
            .map(|(r, changed)| (r[6].as_str(), *changed))
            .collect();
        assert_eq!(
            marks,
            vec![
                ("same", false),
                ("changed, was 10.0.0.5", true),
                ("same", false)
            ],
            "a silent TTL is no evidence of a different router"
        );
    }

    #[test]
    fn ticket_details_open_the_report_and_link_load_is_against_the_link_speed() {
        let mut r = report();
        r.ticket = vec![("Account number".to_owned(), "A-1234".to_owned())];
        r.description = Some("Drops every evening".to_owned());
        let text = text(&r, &label);
        let ticket = text.find("Customer and ticket").unwrap();
        assert!(ticket < text.find("Summary").unwrap());
        assert!(text.contains("A-1234") && text.contains("Drops every evening"));
        assert_eq!(
            load_text((3e6, 25e6), Some(100)),
            "mean 3.0 Mb/s, peak 25.0 Mb/s (25% of 100 Mb/s)"
        );
        assert!(
            !super::text(&report(), &label).contains("Customer and ticket"),
            "no details, no section"
        );
    }
}
