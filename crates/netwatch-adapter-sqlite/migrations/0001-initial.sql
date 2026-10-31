-- 0001: samples, epochs, and topology events (ADR-0006). Never edit once applied.

-- One row per sample. Exactly one of rtt_us and loss is set (ADR-0004).
CREATE TABLE samples (
    role             TEXT    NOT NULL,
    epoch            INTEGER NOT NULL,
    at_us            INTEGER NOT NULL,
    rtt_us           INTEGER,
    loss             TEXT CHECK (loss IN ('timed-out', 'unreachable', 'send-failed')),
    unreachable_code INTEGER,
    CHECK ((rtt_us IS NULL) <> (loss IS NULL)),
    CHECK ((loss = 'unreachable') = (unreachable_code IS NOT NULL))
);
CREATE INDEX samples_by_role_and_time ON samples (role, at_us);

-- Every identity epoch, with the span during which its role held its identity.
CREATE TABLE epochs (
    epoch      INTEGER PRIMARY KEY,
    role       TEXT    NOT NULL,
    identity   TEXT    NOT NULL,
    started_us INTEGER NOT NULL,
    ended_us   INTEGER
);

-- Every topology event, in order, for the graphs' event markers.
CREATE TABLE events (
    id         INTEGER PRIMARY KEY,
    at_us      INTEGER NOT NULL,
    kind       TEXT    NOT NULL,
    role       TEXT,
    epoch      INTEGER,
    detail     TEXT
);
CREATE INDEX events_by_time ON events (at_us);
