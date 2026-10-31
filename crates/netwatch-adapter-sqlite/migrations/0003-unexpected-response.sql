-- 0003: a response that isn't the expected one (an HTTP status) is its own kind of
-- loss, with the status as its code. Rebuilt for the same reason as 0002. Never edit
-- once applied.

CREATE TABLE samples_0003 (
    role      TEXT    NOT NULL,
    epoch     INTEGER NOT NULL,
    at_us     INTEGER NOT NULL,
    rtt_us    INTEGER,
    loss      TEXT CHECK (loss IN ('timed-out', 'unreachable', 'refused', 'unexpected-response', 'send-failed')),
    loss_code INTEGER,
    CHECK ((rtt_us IS NULL) <> (loss IS NULL)),
    CHECK ((loss IN ('unreachable', 'refused', 'unexpected-response')) = (loss_code IS NOT NULL))
);
INSERT INTO samples_0003 SELECT role, epoch, at_us, rtt_us, loss, loss_code FROM samples;
DROP TABLE samples;
ALTER TABLE samples_0003 RENAME TO samples;
CREATE INDEX samples_by_role_and_time ON samples (role, at_us);
