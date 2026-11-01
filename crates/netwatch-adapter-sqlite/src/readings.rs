//! Readings and reading rollups (ADR-0032): stored, retained, and rolled up beside
//! samples, keyed the same way.

use netwatch_history::{Reading, ReadingRollup, ReadingSummary, Tier};
use netwatch_shared::{Gauge, IdentityEpoch, Role, WallTime};
use rusqlite::{Row, params};

use crate::{Store, StoreError, epoch_value, tier_text};

fn epoch(value: i64) -> Option<IdentityEpoch> {
    u64::try_from(value)
        .ok()
        .and_then(std::num::NonZeroU64::new)
        .map(IdentityEpoch::new)
}

/// A reading row: role, epoch, at_us, gauge, value. `None` if it no longer parses.
fn reading(row: &Row<'_>) -> rusqlite::Result<Option<Reading>> {
    let (role, e, at, gauge, value): (String, i64, i64, String, f64) = (
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    );
    Ok((|| {
        Some(Reading {
            role: Role::from_key(&role)?,
            epoch: epoch(e)?,
            at: WallTime::from_unix_micros(at),
            gauge: Gauge::from_key(&gauge)?,
            value,
        })
    })())
}

/// A rollup row: role, epoch, start_us, gauge, count, sum, min, max.
fn rollup(row: &Row<'_>, tier: Tier) -> rusqlite::Result<Option<ReadingRollup>> {
    let (role, e, start, gauge): (String, i64, i64, String) =
        (row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?);
    let summary = ReadingSummary {
        count: u64::try_from(row.get::<_, i64>(4)?).unwrap_or(0),
        sum: row.get(5)?,
        min: row.get(6)?,
        max: row.get(7)?,
    };
    Ok((|| {
        Some(ReadingRollup {
            role: Role::from_key(&role)?,
            epoch: epoch(e)?,
            tier,
            start: WallTime::from_unix_micros(start),
            gauge: Gauge::from_key(&gauge)?,
            summary,
        })
    })())
}

impl Store {
    /// Writes a batch of readings in one transaction.
    pub fn write_readings(&mut self, readings: &[Reading]) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT INTO readings (role, epoch, at_us, gauge, value) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for r in readings {
                insert.execute(params![
                    r.role.key(),
                    epoch_value(r.epoch),
                    r.at.as_unix_micros(),
                    r.gauge.key(),
                    r.value
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// The role's readings at or after `since`, oldest first.
    pub fn readings(&self, role: &Role, since: WallTime) -> Result<Vec<Reading>, StoreError> {
        let mut query = self.connection.prepare_cached(
            "SELECT role, epoch, at_us, gauge, value FROM readings
             WHERE role = ?1 AND at_us >= ?2 ORDER BY at_us, gauge",
        )?;
        let rows = query.query_map(params![role.key(), since.as_unix_micros()], reading)?;
        Ok(rows
            .filter_map(Result::transpose)
            .collect::<Result<_, _>>()?)
    }

    /// Every reading, of every role, with `from <= at < until`.
    pub fn readings_between(
        &self,
        from: WallTime,
        until: WallTime,
    ) -> Result<Vec<Reading>, StoreError> {
        let mut query = self.connection.prepare_cached(
            "SELECT role, epoch, at_us, gauge, value FROM readings
             WHERE at_us >= ?1 AND at_us < ?2",
        )?;
        let rows = query.query_map(
            params![from.as_unix_micros(), until.as_unix_micros()],
            reading,
        )?;
        Ok(rows
            .filter_map(Result::transpose)
            .collect::<Result<_, _>>()?)
    }

    /// Writes reading rollups, replacing any for the same interval: an interval is
    /// rolled up again only if it was incomplete.
    pub fn write_reading_rollups(&mut self, rollups: &[ReadingRollup]) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        {
            let mut insert = transaction.prepare_cached(
                "INSERT OR REPLACE INTO reading_rollups
                 (role, epoch, tier, start_us, gauge, count, sum, min, max)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for r in rollups.iter().filter(|r| r.summary.count > 0) {
                insert.execute(params![
                    r.role.key(),
                    epoch_value(r.epoch),
                    tier_text(r.tier),
                    r.start.as_unix_micros(),
                    r.gauge.key(),
                    i64::try_from(r.summary.count).unwrap_or(i64::MAX),
                    r.summary.sum,
                    r.summary.min,
                    r.summary.max
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// The role's reading rollups of `tier` starting at or after `since`, oldest first.
    pub fn reading_rollups(
        &self,
        role: &Role,
        tier: Tier,
        since: WallTime,
    ) -> Result<Vec<ReadingRollup>, StoreError> {
        let mut query = self.connection.prepare_cached(
            "SELECT role, epoch, start_us, gauge, count, sum, min, max FROM reading_rollups
             WHERE role = ?1 AND tier = ?2 AND start_us >= ?3 ORDER BY start_us, gauge",
        )?;
        let rows = query.query_map(
            params![role.key(), tier_text(tier), since.as_unix_micros()],
            |row| rollup(row, tier),
        )?;
        Ok(rows
            .filter_map(Result::transpose)
            .collect::<Result<_, _>>()?)
    }

    /// Every reading rollup of `tier` with `from <= start < until`.
    pub fn reading_rollups_between(
        &self,
        tier: Tier,
        from: WallTime,
        until: WallTime,
    ) -> Result<Vec<ReadingRollup>, StoreError> {
        let mut query = self.connection.prepare_cached(
            "SELECT role, epoch, start_us, gauge, count, sum, min, max FROM reading_rollups
             WHERE tier = ?1 AND start_us >= ?2 AND start_us < ?3",
        )?;
        let rows = query.query_map(
            params![
                tier_text(tier),
                from.as_unix_micros(),
                until.as_unix_micros()
            ],
            |row| rollup(row, tier),
        )?;
        Ok(rows
            .filter_map(Result::transpose)
            .collect::<Result<_, _>>()?)
    }
}
