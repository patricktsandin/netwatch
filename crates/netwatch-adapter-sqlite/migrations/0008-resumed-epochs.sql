-- 0008: a restart is not a change (ADR-0044). Never edit once applied.
--
-- An epoch that a restart ended has no epoch-ended event. When the epoch after it,
-- for the same role, has the same identity, that epoch resumed it.
UPDATE events SET kind = 'epoch-resumed'
WHERE kind = 'epoch-started' AND EXISTS (
    SELECT 1
    FROM epochs this
    JOIN epochs before ON before.role = this.role
        AND before.epoch = (SELECT MAX(epoch) FROM epochs
                            WHERE role = this.role AND epoch < this.epoch)
    WHERE this.epoch = events.epoch
      AND before.identity = this.identity
      AND NOT EXISTS (SELECT 1 FROM events ended
                      WHERE ended.kind = 'epoch-ended' AND ended.epoch = before.epoch)
);

-- The first public-address note of a run has no "before → after". When it repeats
-- the address noted before it, it says nothing.
DELETE FROM events
WHERE kind = 'public-address' AND instr(detail, ' → ') = 0 AND detail = (
    SELECT CASE WHEN instr(p.detail, ' → ') > 0
                THEN substr(p.detail, instr(p.detail, ' → ') + 3)
                ELSE p.detail END
    FROM events p
    WHERE p.kind = 'public-address'
      AND (p.at_us < events.at_us OR (p.at_us = events.at_us AND p.id < events.id))
    ORDER BY p.at_us DESC, p.id DESC
    LIMIT 1
);
