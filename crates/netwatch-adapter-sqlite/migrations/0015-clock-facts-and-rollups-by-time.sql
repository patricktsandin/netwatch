-- 0015: facts the daemon keeps about this host across runs, such as when its clock
-- was last synchronized, which decides whether pruning may trust it (ADR-0066); and
-- rollups looked up by tier and time, as rolling up and pruning do. Never edit once
-- applied.

CREATE TABLE IF NOT EXISTS facts (
    name  TEXT    PRIMARY KEY,
    value INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS rollups_by_tier_time ON rollups (tier, start_us);
CREATE INDEX IF NOT EXISTS reading_rollups_by_tier_time ON reading_rollups (tier, start_us);
