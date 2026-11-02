//! The single writer (ADR-0006): batches samples and records events on one thread.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use netwatch_adapter_sqlite::Store;
use netwatch_history::{self as history, Reading, Sample, Tier};
use netwatch_shared::WallTime;
use netwatch_topology::TopologyEvent;

pub enum Write {
    Sample(Sample),
    /// A gauge reading (ADR-0032), batched with samples.
    Reading(Reading),
    /// A topology event, and whether it resumes an identity from before the
    /// start (ADR-0044).
    Event(TopologyEvent, WallTime, bool),
    /// An event that isn't a topology event: a kernel log line, a disabled source.
    Note(netwatch_adapter_sqlite::StoredEvent),
    /// An incident that opened, changed, or closed (ADR-0031).
    Incident(netwatch_probing::incident::Incident),
    /// A finished path trace (ADR-0040).
    Trace(netwatch_adapter_sqlite::StoredTrace),
    /// Write everything received so far, then acknowledge. The API sends this before
    /// answering a backfill, so a backfill never misses a sample the live stream
    /// already sent (ADR-0029). The channel is FIFO, so "everything so far" includes
    /// every sample broadcast before the request.
    Flush(tokio::sync::oneshot::Sender<()>),
    /// Delete all history (ADR-0038), then report how it went. The batch held so far
    /// is discarded first, so nothing from before the flush lands after it.
    FlushHistory(tokio::sync::oneshot::Sender<Result<(), String>>),
}

const BATCH: usize = 256;
const FLUSH_EVERY: Duration = Duration::from_secs(1);
const PRUNE_EVERY: Duration = Duration::from_secs(3600);
const ROLL_UP_EVERY: Duration = Duration::from_secs(60);
/// A minute is rolled up only this long after it ends, so that samples still in a
/// write batch aren't missed.
const SETTLE_US: i64 = 2 * 60 * 1_000_000;

/// Rolls up every complete minute not yet rolled up, then every complete hour from
/// its minutes, an hour of data at a time. Progress is recorded per tier, so a
/// restart resumes where it left off (ADR-0006).
fn roll_up(store: &mut Store, now: WallTime) -> Result<(), netwatch_adapter_sqlite::StoreError> {
    let settled = WallTime::from_unix_micros(now.as_unix_micros() - SETTLE_US);
    let minutes_done = Tier::Minute.start_of(settled);
    let hour = Tier::Hour.interval_us();
    let first_minute = store.rollup_progress(Tier::Minute)?.unwrap_or_else(|| {
        WallTime::from_unix_micros(
            minutes_done.as_unix_micros() - netwatch_adapter_sqlite::RAW_RETENTION_DAYS * 24 * hour,
        )
    });
    let mut from = first_minute;
    while from < minutes_done {
        let until = WallTime::from_unix_micros(
            (from.as_unix_micros() + hour).min(minutes_done.as_unix_micros()),
        );
        let samples = store.samples_between(from, until, None)?;
        store.write_rollups(&history::roll_up(&samples, Tier::Minute))?;
        let readings = store.readings_between(from, until)?;
        store.write_reading_rollups(&history::roll_up_readings(&readings, Tier::Minute))?;
        store.set_rollup_progress(Tier::Minute, until)?;
        from = until;
    }
    let hours_done = Tier::Hour.start_of(minutes_done);
    let mut from = store
        .rollup_progress(Tier::Hour)?
        .unwrap_or(Tier::Hour.start_of(first_minute));
    while from < hours_done {
        let until = WallTime::from_unix_micros(from.as_unix_micros() + hour);
        let minutes = store.rollups_between(Tier::Minute, from, until, None)?;
        store.write_rollups(&history::coarsen(&minutes, Tier::Hour))?;
        let minutes = store.reading_rollups_between(Tier::Minute, from, until)?;
        store.write_reading_rollups(&history::coarsen_readings(&minutes, Tier::Hour))?;
        store.set_rollup_progress(Tier::Hour, until)?;
        from = until;
    }
    Ok(())
}

/// What the store says each role was, and the public address, before
/// this start (ADR-0044).
#[derive(Debug, Default)]
pub struct BeforeStart {
    /// Each role's identity, in words, in the epoch the stop left open.
    pub identities: std::collections::BTreeMap<netwatch_shared::Role, String>,
    /// The public address last noted, in words.
    pub public: Option<String>,
}

/// Starts the writer, and returns what was there before this start. Writes are lost
/// only if the database itself fails, and that is logged.
pub fn spawn(
    path: PathBuf,
    clock: crate::clock::Clock,
) -> anyhow::Result<(mpsc::Sender<Write>, BeforeStart)> {
    let mut store = Store::open(&path)?;
    // Read before the epochs the last run left open are closed: those are the ones
    // that can resume.
    let before = BeforeStart {
        identities: store.last_identities()?,
        public: store.last_public_address()?,
    };
    let closed = store.close_epochs_left_open(clock.wall_now())?;
    if closed > 0 {
        tracing::info!(closed, "closed epochs left open by the previous run");
    }
    let closed = store.close_incidents_left_open()?;
    if closed > 0 {
        tracing::info!(closed, "closed incidents left open by the previous run");
    }
    let (tx, rx) = mpsc::channel::<Write>();
    thread::Builder::new()
        .name("sqlite-writer".to_owned())
        .spawn(move || {
            let mut batch = Vec::with_capacity(BATCH);
            let mut readings: Vec<Reading> = Vec::with_capacity(BATCH);
            let mut flushed = Instant::now();
            let mut pruned = Instant::now() - PRUNE_EVERY;
            let mut rolled = Instant::now() - ROLL_UP_EVERY;
            loop {
                let next = rx.recv_timeout(FLUSH_EVERY);
                match next {
                    Ok(Write::Sample(sample)) => batch.push(sample),
                    Ok(Write::Reading(reading)) => readings.push(reading),
                    Ok(Write::Event(event, at, resumed)) => {
                        if let Err(error) = store.record_event(&event, at, resumed) {
                            tracing::error!(%error, "cannot record topology event");
                        }
                    }
                    Ok(Write::Note(note)) => {
                        if let Err(error) = store.record_note(&note) {
                            tracing::error!(%error, "cannot record an event");
                        }
                    }
                    Ok(Write::Incident(incident)) => {
                        if let Err(error) = store.write_incident(&incident) {
                            tracing::error!(%error, "cannot record an incident");
                        }
                    }
                    Ok(Write::Trace(trace)) => {
                        if let Err(error) = store.write_trace(&trace) {
                            tracing::error!(%error, "cannot record a path trace");
                        }
                    }
                    Ok(Write::FlushHistory(done)) => {
                        batch.clear();
                        readings.clear();
                        let result = store.flush_history().map_err(|e| e.to_string());
                        match &result {
                            Ok(()) => tracing::warn!("all history flushed"),
                            Err(error) => tracing::error!(%error, "cannot flush history"),
                        }
                        let _ = done.send(result);
                    }
                    Ok(Write::Flush(done)) => {
                        if !batch.is_empty()
                            && let Err(error) = store.write_samples(&batch)
                        {
                            tracing::error!(%error, lost = batch.len(), "cannot write samples");
                        }
                        batch.clear();
                        write_readings(&mut store, &mut readings);
                        flushed = Instant::now();
                        let _ = done.send(());
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if batch.len() >= BATCH || flushed.elapsed() >= FLUSH_EVERY {
                    if !batch.is_empty()
                        && let Err(error) = store.write_samples(&batch)
                    {
                        tracing::error!(%error, lost = batch.len(), "cannot write samples");
                    }
                    batch.clear();
                    write_readings(&mut store, &mut readings);
                    flushed = Instant::now();
                }
                if rolled.elapsed() >= ROLL_UP_EVERY {
                    if let Err(error) = roll_up(&mut store, clock.wall_now()) {
                        tracing::error!(%error, "cannot roll up samples");
                    }
                    rolled = Instant::now();
                }
                if pruned.elapsed() >= PRUNE_EVERY {
                    match store.prune(clock.wall_now()) {
                        Ok(removed) if removed > 0 => tracing::info!(removed, "pruned raw samples"),
                        Ok(_) => {}
                        Err(error) => tracing::error!(%error, "cannot prune samples"),
                    }
                    pruned = Instant::now();
                }
            }
            if !batch.is_empty() {
                let _ = store.write_samples(&batch);
            }
            write_readings(&mut store, &mut readings);
        })?;
    Ok((tx, before))
}

fn write_readings(store: &mut Store, readings: &mut Vec<Reading>) {
    if !readings.is_empty()
        && let Err(error) = store.write_readings(readings)
    {
        tracing::error!(%error, lost = readings.len(), "cannot write readings");
    }
    readings.clear();
}

/// The highest epoch already stored at `path`, if any.
pub fn last_epoch(path: &std::path::Path) -> anyhow::Result<Option<u64>> {
    Ok(Store::open(path)?
        .last_epoch()?
        .and_then(|e| u64::try_from(e).ok()))
}

/// Each of `targets`' latest trace stored at `path`, routine or episode, for the
/// network map (ADR-0042). A trace toward another address than the target's own is
/// a path the target no longer takes, since its address was edited (ADR-0035), so
/// it is left out.
pub fn latest_traces(
    path: &std::path::Path,
    targets: &std::collections::BTreeMap<netwatch_shared::TargetId, std::net::IpAddr>,
) -> anyhow::Result<
    std::collections::BTreeMap<netwatch_shared::TargetId, netwatch_adapter_sqlite::StoredTrace>,
> {
    use netwatch_probing::trace::TraceKind;
    let store = Store::open(path)?;
    let end = netwatch_shared::WallTime::from_unix_micros(i64::MAX);
    let mut latest = std::collections::BTreeMap::new();
    for (&target, &destination) in targets {
        let role = netwatch_shared::Role::Target(target);
        let routine = store.latest_trace(&role, TraceKind::Routine, end)?;
        let episode = store.latest_trace(&role, TraceKind::Episode, end)?;
        let newest = [routine, episode]
            .into_iter()
            .flatten()
            .max_by_key(|t| t.at.as_unix_micros());
        if let Some(trace) = newest.filter(|t| t.destination == destination) {
            latest.insert(target, trace);
        }
    }
    Ok(latest)
}
