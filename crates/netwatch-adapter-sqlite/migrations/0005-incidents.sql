-- 0005: incidents, a role's degraded and failing periods (ADR-0031). Never edit
-- once applied.

-- An incident is identified by its role and start, which never change. It is
-- written when it opens and rewritten whenever it changes, until it closes.
CREATE TABLE incidents (
    role        TEXT    NOT NULL,
    started_us  INTEGER NOT NULL,
    ended_us    INTEGER,
    severity    TEXT    NOT NULL CHECK (severity IN ('degradation', 'outage')),
    cause       TEXT,
    PRIMARY KEY (role, started_us)
);
CREATE INDEX incidents_by_start ON incidents (started_us);
