-- 0006: gauge readings and their rollups (ADR-0032). Never edit once applied.

CREATE TABLE readings (
    role   TEXT    NOT NULL,
    epoch  INTEGER NOT NULL,
    at_us  INTEGER NOT NULL,
    gauge  TEXT    NOT NULL,
    value  REAL    NOT NULL
);
CREATE INDEX readings_by_role_and_time ON readings (role, at_us);
CREATE INDEX readings_by_time ON readings (at_us);

-- An empty interval is never stored, so min and max are always real values.
CREATE TABLE reading_rollups (
    role      TEXT    NOT NULL,
    epoch     INTEGER NOT NULL,
    tier      TEXT    NOT NULL CHECK (tier IN ('minute', 'hour')),
    start_us  INTEGER NOT NULL,
    gauge     TEXT    NOT NULL,
    count     INTEGER NOT NULL CHECK (count > 0),
    sum       REAL    NOT NULL,
    min       REAL    NOT NULL,
    max       REAL    NOT NULL,
    PRIMARY KEY (role, epoch, tier, gauge, start_us)
);
CREATE INDEX reading_rollups_by_role_tier_time ON reading_rollups (role, tier, start_us);
