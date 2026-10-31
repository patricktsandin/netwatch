//! The single writer (ADR-0006): batches samples and records events on one thread.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use netwatch_adapter_sqlite::Store;
use netwatch_history::Sample;
use netwatch_shared::WallTime;
use netwatch_topology::TopologyEvent;

pub enum Write {
    Sample(Sample),
    Event(TopologyEvent, WallTime),
}

const BATCH: usize = 256;
const FLUSH_EVERY: Duration = Duration::from_secs(1);
const PRUNE_EVERY: Duration = Duration::from_secs(3600);

/// Starts the writer. Writes are lost only if the database itself fails, and that is
/// logged.
pub fn spawn(path: PathBuf, clock: crate::clock::Clock) -> anyhow::Result<mpsc::Sender<Write>> {
    let mut store = Store::open(&path)?;
    let closed = store.close_epochs_left_open(clock.wall_now())?;
    if closed > 0 {
        tracing::info!(closed, "closed epochs left open by the previous run");
    }
    let (tx, rx) = mpsc::channel::<Write>();
    thread::Builder::new()
        .name("sqlite-writer".to_owned())
        .spawn(move || {
            let mut batch = Vec::with_capacity(BATCH);
            let mut flushed = Instant::now();
            let mut pruned = Instant::now() - PRUNE_EVERY;
            loop {
                let next = rx.recv_timeout(FLUSH_EVERY);
                match next {
                    Ok(Write::Sample(sample)) => batch.push(sample),
                    Ok(Write::Event(event, at)) => {
                        if let Err(error) = store.record_event(&event, at) {
                            tracing::error!(%error, "cannot record topology event");
                        }
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
                    flushed = Instant::now();
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
        })?;
    Ok(tx)
}

/// The highest epoch already stored at `path`, if any.
pub fn last_epoch(path: &std::path::Path) -> anyhow::Result<Option<u64>> {
    Ok(Store::open(path)?
        .last_epoch()?
        .and_then(|e| u64::try_from(e).ok()))
}
