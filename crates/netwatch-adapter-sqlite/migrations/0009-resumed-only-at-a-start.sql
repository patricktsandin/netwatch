-- 0009: only an epoch that starts within 60 s of a start resumes one (ADR-0046).
-- Never edit once applied.
--
-- A start closes the epochs the last run left open at the moment it starts, so an
-- epoch that began more than 60 s after the one before it was closed came back
-- after an outage: a change, as migration 0008 didn't yet know.
UPDATE events SET kind = 'epoch-started'
WHERE kind = 'epoch-resumed' AND EXISTS (
    SELECT 1
    FROM epochs this
    JOIN epochs before ON before.role = this.role
        AND before.epoch = (SELECT MAX(epoch) FROM epochs
                            WHERE role = this.role AND epoch < this.epoch)
    WHERE this.epoch = events.epoch
      AND before.ended_us IS NOT NULL
      AND this.started_us - before.ended_us > 60000000
);
