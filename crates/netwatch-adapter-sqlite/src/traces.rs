//! Path traces (ADR-0040): written once when a trace finishes, read by the report.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::num::NonZeroU8;

use netwatch_probing::trace::{PathTrace, TraceHop, TraceKind, TracePath};
use netwatch_shared::uom::si::f64::Time;
use netwatch_shared::uom::si::time::microsecond;
use netwatch_shared::{HopTtl, Role, WallTime};
use rusqlite::{OptionalExtension, params};

use crate::{Store, StoreError};

/// A finished path trace, with the names of the routers that answered it.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredTrace {
    /// The target whose path was traced.
    pub role: Role,
    /// When it started.
    pub at: WallTime,
    /// Why it ran.
    pub kind: TraceKind,
    /// The address it was sent toward.
    pub destination: IpAddr,
    /// What it found.
    pub trace: PathTrace,
    /// Each responder's router name, in words, as it was when the trace ran.
    pub names: BTreeMap<IpAddr, String>,
    /// The path it ran on: the egress interface and the local address, such as
    /// `wlan0 from 192.168.1.20` (ADR-0056). `None` for traces from before it was
    /// kept.
    pub path: Option<TracePath>,
}

fn kind_text(kind: TraceKind) -> &'static str {
    match kind {
        TraceKind::Routine => "routine",
        TraceKind::Episode => "episode",
    }
}

fn micros(time: Option<Time>) -> Option<i64> {
    // `as` saturates, and no RTT approaches i64::MAX µs.
    time.map(|t| t.get::<microsecond>().round() as i64)
}

fn ttl(raw: i64) -> Option<HopTtl> {
    NonZeroU8::new(u8::try_from(raw).ok()?).map(HopTtl::new)
}

impl Store {
    /// Writes a finished trace.
    pub fn write_trace(&mut self, trace: &StoredTrace) -> Result<(), StoreError> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO traces (role, at_us, kind, destination, reached, silent_from, path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                trace.role.key(),
                trace.at.as_unix_micros(),
                kind_text(trace.kind),
                trace.destination.to_string(),
                trace.trace.reached,
                trace.trace.silent_from.map(|t| t.get().get()),
                trace.path.as_ref().map(TracePath::as_str),
            ],
        )?;
        let id = transaction.last_insert_rowid();
        for hop in &trace.trace.hops {
            let ttl = hop.ttl.get().get();
            transaction.execute(
                "INSERT INTO trace_hops (trace_id, ttl, sent, answered, best_us, median_us)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    ttl,
                    hop.sent,
                    hop.answered,
                    micros(hop.best),
                    micros(hop.median)
                ],
            )?;
            for (position, address) in hop.responders.iter().enumerate() {
                transaction.execute(
                    "INSERT INTO trace_responders (trace_id, ttl, position, address, name)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id,
                        ttl,
                        i64::try_from(position).unwrap_or(i64::MAX),
                        address.to_string(),
                        trace.names.get(address),
                    ],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Every trace that started in `from..until`, oldest first.
    pub fn traces(&self, from: WallTime, until: WallTime) -> Result<Vec<StoredTrace>, StoreError> {
        let mut query = self.connection.prepare_cached(
            "SELECT id FROM traces WHERE at_us >= ?1 AND at_us < ?2 ORDER BY at_us, id",
        )?;
        let ids = query
            .query_map([from.as_unix_micros(), until.as_unix_micros()], |row| {
                row.get(0)
            })?
            .collect::<Result<Vec<i64>, _>>()?;
        let mut traces = Vec::new();
        for id in ids {
            traces.extend(self.trace(id)?);
        }
        Ok(traces)
    }

    /// The latest trace of `role` of `kind` that started before `before`, and, given
    /// a `path`, ran on it (ADR-0056). A trace from before paths were recorded may
    /// have run on any.
    pub fn latest_trace(
        &self,
        role: &Role,
        kind: TraceKind,
        before: WallTime,
        path: Option<&TracePath>,
    ) -> Result<Option<StoredTrace>, StoreError> {
        let id: Option<i64> = self
            .connection
            .query_row(
                "SELECT id FROM traces WHERE role = ?1 AND kind = ?2 AND at_us < ?3
                 AND (?4 IS NULL OR path IS NULL OR path = ?4)
                 ORDER BY at_us DESC LIMIT 1",
                params![
                    role.key(),
                    kind_text(kind),
                    before.as_unix_micros(),
                    path.map(TracePath::as_str)
                ],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.trace(id),
            None => Ok(None),
        }
    }

    /// The path `role` was last traced on before `before`, by a trace of either kind:
    /// the path it took then (ADR-0056). `None` if it has no trace that recorded one.
    pub fn latest_path(
        &self,
        role: &Role,
        before: WallTime,
    ) -> Result<Option<TracePath>, StoreError> {
        Ok(self
            .connection
            .query_row(
                "SELECT path FROM traces WHERE role = ?1 AND at_us < ?2 AND path IS NOT NULL
                 ORDER BY at_us DESC LIMIT 1",
                params![role.key(), before.as_unix_micros()],
                |row| row.get(0),
            )
            .optional()?
            .map(TracePath::from_stored))
    }

    /// Every path `role` has a trace of, besides traces from before paths were
    /// recorded (`None`).
    pub fn trace_paths(&self, role: &Role) -> Result<Vec<Option<TracePath>>, StoreError> {
        let mut query = self
            .connection
            .prepare_cached("SELECT DISTINCT path FROM traces WHERE role = ?1")?;
        let paths = query
            .query_map([role.key()], |row| row.get::<_, Option<String>>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(paths
            .into_iter()
            .map(|p| p.map(TracePath::from_stored))
            .collect())
    }

    /// One trace by its row id. `None` if its row no longer parses.
    fn trace(&self, id: i64) -> Result<Option<StoredTrace>, StoreError> {
        let (role, at, kind, destination, reached, silent_from, path) = self.connection.query_row(
            "SELECT role, at_us, kind, destination, reached, silent_from, path FROM traces
             WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )?;
        let (Some(role), Ok(destination)) = (Role::from_key(&role), destination.parse()) else {
            return Ok(None);
        };
        let mut responders: BTreeMap<i64, Vec<IpAddr>> = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut query = self.connection.prepare_cached(
            "SELECT ttl, address, name FROM trace_responders WHERE trace_id = ?1
             ORDER BY ttl, position",
        )?;
        for row in query.query_map([id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })? {
            let (ttl, address, name) = row?;
            let Ok(address) = address.parse::<IpAddr>() else {
                continue;
            };
            responders.entry(ttl).or_default().push(address);
            if let Some(name) = name {
                names.insert(address, name);
            }
        }
        let mut query = self.connection.prepare_cached(
            "SELECT ttl, sent, answered, best_us, median_us FROM trace_hops WHERE trace_id = ?1
             ORDER BY ttl",
        )?;
        let mut hops = Vec::new();
        for row in query.query_map([id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })? {
            let (raw, sent, answered, best, median) = row?;
            let Some(hop_ttl) = ttl(raw) else { continue };
            let time = |us: Option<i64>| us.map(|us| Time::new::<microsecond>(us as f64));
            hops.push(TraceHop {
                ttl: hop_ttl,
                responders: responders.remove(&raw).unwrap_or_default(),
                sent,
                answered,
                best: time(best),
                median: time(median),
            });
        }
        Ok(Some(StoredTrace {
            role,
            at: WallTime::from_unix_micros(at),
            kind: if kind == "episode" {
                TraceKind::Episode
            } else {
                TraceKind::Routine
            },
            destination,
            trace: PathTrace {
                hops,
                reached,
                silent_from: silent_from.and_then(ttl),
            },
            names,
            path: path.map(TracePath::from_stored),
        }))
    }
}
