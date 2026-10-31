-- 0002: a server's error answer (a DNS rcode) is its own kind of loss. SQLite can't
-- alter a CHECK constraint, so the table is rebuilt; `unreachable_code` becomes
-- `loss_code`, the code of any loss that has one. Never edit once applied.

CREATE TABLE samples_0002 (
    role      TEXT    NOT NULL,
    epoch     INTEGER NOT NULL,
    at_us     INTEGER NOT NULL,
    rtt_us    INTEGER,
    loss      TEXT CHECK (loss IN ('timed-out', 'unreachable', 'refused', 'send-failed')),
    loss_code INTEGER,
    CHECK ((rtt_us IS NULL) <> (loss IS NULL)),
    CHECK ((loss IN ('unreachable', 'refused')) = (loss_code IS NOT NULL))
);
INSERT INTO samples_0002 (role, epoch, at_us, rtt_us, loss, loss_code)
    SELECT role, epoch, at_us, rtt_us, loss, unreachable_code FROM samples;
DROP TABLE samples;
ALTER TABLE samples_0002 RENAME TO samples;
CREATE INDEX samples_by_role_and_time ON samples (role, at_us);
