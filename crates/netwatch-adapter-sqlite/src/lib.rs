//! Persistence for the History context: schema, numbered migrations, writes, and the
//! read queries behind the UI (ADR-0006).
//!
//! Here, and only here, quantities become unit-suffixed integers (`rtt_us`). Roles are
//! stored as their [`Role::key`]. A migration that has been applied is never edited:
//! a schema change is a new file in `migrations/`, appended to [`MIGRATIONS`].
//!
//! Not yet implemented: rollup tiers. Retention currently prunes raw samples only.

use std::path::Path;

use netwatch_history::{Loss, Measurement, Sample};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::microsecond;
use netwatch_shared::{Identity, IdentityEpoch, Role, WallTime};
use netwatch_topology::TopologyEvent;
use rusqlite::{Connection, OptionalExtension, params};

/// The schema, as an ordered list of migrations. The database's `user_version` is the
/// number of migrations applied.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001-initial.sql"),
    include_str!("../migrations/0002-refused-loss.sql"),
];

/// How long raw samples are kept (ADR-0006).
pub const RAW_RETENTION_DAYS: i64 = 7;

/// Why the store failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// SQLite reported an error.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// The database is from a newer netwatch than this one.
    #[error("database schema version {0} is newer than this build supports")]
    SchemaTooNew(i64),
}

/// A connection to the store. One `Store` writes, from a single task. Readers open
/// their own `Store`, which WAL mode allows to run concurrently with the writer.
#[derive(Debug)]
pub struct Store {
    connection: Connection,
}

/// One row of a sample query, in API units.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredSample {
    /// When the sample was taken.
    pub at: WallTime,
    /// Its pin's epoch.
    pub epoch: IdentityEpoch,
    /// Its value.
    pub measurement: Measurement,
}

/// One row of an event query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// When the event happened.
    pub at: WallTime,
    /// `epoch-started`, `epoch-ended`, `desynchronized`, or `resynchronized`.
    pub kind: String,
    /// The role it concerns, as a key, if any.
    pub role: Option<String>,
    /// The epoch it concerns, if any.
    pub epoch: Option<i64>,
    /// A description for people: the new identity, or the generation.
    pub detail: Option<String>,
}

impl Store {
    /// Opens (creating if needed) and migrates the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StoreError> {
        let applied: i64 = self
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        let known = i64::try_from(MIGRATIONS.len()).unwrap_or(i64::MAX);
        if applied > known {
            return Err(StoreError::SchemaTooNew(applied));
        }
        for (number, sql) in MIGRATIONS
            .iter()
            .enumerate()
            .skip(usize::try_from(applied).unwrap_or(0))
        {
            let transaction = self.connection.transaction()?;
            transaction.execute_batch(sql)?;
            transaction.pragma_update(
                None,
                "user_version",
                i64::try_from(number + 1).unwrap_or(i64::MAX),
            )?;
            transaction.commit()?;
        }
        Ok(())
    }

    /// Writes a batch of samples in one transaction.
    pub fn write_samples(&mut self, samples: &[Sample]) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT INTO samples (role, epoch, at_us, rtt_us, loss, loss_code)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for sample in samples {
                let (rtt_us, loss, code) = match sample.measurement {
                    Measurement::RoundTrip(rtt) => (Some(micros(rtt)), None, None),
                    Measurement::Lost(Loss::TimedOut) => (None, Some("timed-out"), None),
                    Measurement::Lost(Loss::Unreachable(code)) => {
                        (None, Some("unreachable"), Some(code))
                    }
                    Measurement::Lost(Loss::Refused(code)) => (None, Some("refused"), Some(code)),
                    Measurement::Lost(Loss::SendFailed) => (None, Some("send-failed"), None),
                };
                insert.execute(params![
                    sample.role.key(),
                    epoch_value(sample.epoch),
                    sample.at.as_unix_micros(),
                    rtt_us,
                    loss,
                    code
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Records a topology event, and keeps the `epochs` table in step with it.
    pub fn record_event(&mut self, event: &TopologyEvent, at: WallTime) -> Result<(), StoreError> {
        let at_us = at.as_unix_micros();
        let transaction = self.connection.transaction()?;
        match event {
            TopologyEvent::EpochStarted {
                epoch,
                role,
                identity,
            } => {
                transaction.execute(
                    "INSERT INTO epochs (epoch, role, identity, started_us) VALUES (?1, ?2, ?3, ?4)",
                    params![epoch_value(*epoch), role.key(), identity_text(identity), at_us],
                )?;
            }
            TopologyEvent::EpochEnded { epoch, .. } => {
                transaction.execute(
                    "UPDATE epochs SET ended_us = ?2 WHERE epoch = ?1",
                    params![epoch_value(*epoch), at_us],
                )?;
            }
            TopologyEvent::Desynchronized { .. } | TopologyEvent::Resynchronized { .. } => {}
        }
        let row = describe(event, at);
        transaction.execute(
            "INSERT INTO events (at_us, kind, role, epoch, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![at_us, row.kind, row.role, row.epoch, row.detail],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Closes every epoch an earlier run left open, at `now`. A run that stops can't
    /// record its epochs' ends, but each of them ended no later than the restart, and
    /// the next run starts new epochs from scratch.
    pub fn close_epochs_left_open(&mut self, now: WallTime) -> Result<usize, StoreError> {
        Ok(self.connection.execute(
            "UPDATE epochs SET ended_us = ?1 WHERE ended_us IS NULL",
            [now.as_unix_micros()],
        )?)
    }

    /// Deletes raw samples older than the retention period, relative to `now`.
    pub fn prune(&mut self, now: WallTime) -> Result<usize, StoreError> {
        let cutoff = now.as_unix_micros() - RAW_RETENTION_DAYS * 24 * 3600 * 1_000_000;
        Ok(self
            .connection
            .execute("DELETE FROM samples WHERE at_us < ?1", [cutoff])?)
    }

    /// Every role that has samples, with the time of its latest one.
    pub fn roles(&self) -> Result<Vec<(Role, WallTime)>, StoreError> {
        let mut query = self
            .connection
            .prepare_cached("SELECT role, MAX(at_us) FROM samples GROUP BY role ORDER BY role")?;
        let rows = query.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut roles = Vec::new();
        for row in rows {
            let (key, at) = row?;
            if let Some(role) = Role::from_key(&key) {
                roles.push((role, WallTime::from_unix_micros(at)));
            }
        }
        Ok(roles)
    }

    /// The role's samples at or after `since`, oldest first.
    pub fn samples(&self, role: &Role, since: WallTime) -> Result<Vec<StoredSample>, StoreError> {
        let mut query = self.connection.prepare_cached(
            "SELECT at_us, epoch, rtt_us, loss, loss_code FROM samples
             WHERE role = ?1 AND at_us >= ?2 ORDER BY at_us",
        )?;
        let rows = query.query_map(params![role.key(), since.as_unix_micros()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<u8>>(4)?,
            ))
        })?;
        let mut samples = Vec::new();
        for row in rows {
            let (at, epoch, rtt_us, loss, code) = row?;
            let measurement = match (rtt_us, loss.as_deref(), code) {
                (Some(rtt), None, _) => {
                    Measurement::RoundTrip(Time::new::<microsecond>(rtt as f64))
                }
                (None, Some("timed-out"), _) => Measurement::Lost(Loss::TimedOut),
                (None, Some("unreachable"), Some(code)) => {
                    Measurement::Lost(Loss::Unreachable(code))
                }
                (None, Some("refused"), Some(code)) => Measurement::Lost(Loss::Refused(code)),
                (None, Some("send-failed"), _) => Measurement::Lost(Loss::SendFailed),
                _ => continue, // The CHECK constraints make this unreachable.
            };
            let Some(epoch) = u64::try_from(epoch)
                .ok()
                .and_then(std::num::NonZeroU64::new)
            else {
                continue;
            };
            samples.push(StoredSample {
                at: WallTime::from_unix_micros(at),
                epoch: IdentityEpoch::new(epoch),
                measurement,
            });
        }
        Ok(samples)
    }

    /// Topology events at or after `since`, oldest first.
    pub fn events(&self, since: WallTime) -> Result<Vec<StoredEvent>, StoreError> {
        let mut query = self.connection.prepare_cached(
            "SELECT at_us, kind, role, epoch, detail FROM events WHERE at_us >= ?1 ORDER BY id",
        )?;
        let rows = query.query_map([since.as_unix_micros()], |row| {
            Ok(StoredEvent {
                at: WallTime::from_unix_micros(row.get(0)?),
                kind: row.get(1)?,
                role: row.get(2)?,
                epoch: row.get(3)?,
                detail: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The highest epoch ever stored, so that a restarted daemon's epochs don't
    /// collide with earlier ones.
    pub fn last_epoch(&self) -> Result<Option<i64>, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT MAX(epoch) FROM epochs", [], |row| row.get(0))
            .optional()?
            .flatten())
    }
}

/// The stored form of a topology event, exactly as [`Store::events`] would return it.
/// The live UI uses this too, so live and stored events read identically.
pub fn describe(event: &TopologyEvent, at: WallTime) -> StoredEvent {
    let (kind, role, epoch, detail) = match event {
        TopologyEvent::EpochStarted {
            epoch,
            role,
            identity,
        } => (
            "epoch-started",
            Some(role.key()),
            Some(epoch_value(*epoch)),
            Some(identity_text(identity)),
        ),
        TopologyEvent::EpochEnded { epoch, role } => (
            "epoch-ended",
            Some(role.key()),
            Some(epoch_value(*epoch)),
            None,
        ),
        TopologyEvent::Desynchronized { generation } => (
            "desynchronized",
            None,
            None,
            Some(format!("generation {}", generation.get())),
        ),
        TopologyEvent::Resynchronized { generation } => (
            "resynchronized",
            None,
            None,
            Some(format!("generation {}", generation.get())),
        ),
    };
    StoredEvent {
        at,
        kind: kind.to_owned(),
        role,
        epoch,
        detail,
    }
}

fn micros(time: Time) -> i64 {
    // `as` saturates on overflow, and no RTT approaches i64::MAX µs.
    time.get::<microsecond>().round() as i64
}

fn epoch_value(epoch: IdentityEpoch) -> i64 {
    i64::try_from(epoch.get().get()).unwrap_or(i64::MAX)
}

fn identity_text(identity: &Identity) -> String {
    match identity {
        Identity::Interface { index, name } => format!("{name} (ifindex {})", index.get()),
        Identity::Address(address) => address.to_string(),
    }
}
