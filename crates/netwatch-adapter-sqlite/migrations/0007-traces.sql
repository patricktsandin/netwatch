-- 0007: path traces (ADR-0040), kept forever like incidents. Never edit once
-- applied.

CREATE TABLE traces (
    id           INTEGER PRIMARY KEY,
    role         TEXT    NOT NULL,
    at_us        INTEGER NOT NULL,
    kind         TEXT    NOT NULL CHECK (kind IN ('routine', 'episode')),
    destination  TEXT    NOT NULL,
    reached      INTEGER NOT NULL,
    -- The first TTL from which no hop answered, if the path didn't end.
    silent_from  INTEGER
);
CREATE INDEX traces_by_time ON traces (at_us);

-- One row per TTL the trace kept.
CREATE TABLE trace_hops (
    trace_id   INTEGER NOT NULL,
    ttl        INTEGER NOT NULL,
    sent       INTEGER NOT NULL,
    answered   INTEGER NOT NULL,
    best_us    INTEGER,
    median_us  INTEGER,
    PRIMARY KEY (trace_id, ttl)
);

-- Who answered at each TTL, in order of first answer, with the router name as it
-- was when the trace ran.
CREATE TABLE trace_responders (
    trace_id  INTEGER NOT NULL,
    ttl       INTEGER NOT NULL,
    position  INTEGER NOT NULL,
    address   TEXT    NOT NULL,
    name      TEXT,
    PRIMARY KEY (trace_id, ttl, position)
);
