-- 0004: the 1m and 1h rollup tiers (ADR-0006). Never edit once applied.

-- One row per role, epoch, tier, and interval. The histogram is a blob of 5-byte
-- entries: bucket (u8), then count (u32, little-endian), for non-empty buckets only.
CREATE TABLE rollups (
    role        TEXT    NOT NULL,
    epoch       INTEGER NOT NULL,
    tier        TEXT    NOT NULL CHECK (tier IN ('minute', 'hour')),
    start_us    INTEGER NOT NULL,
    count       INTEGER NOT NULL,
    lost        INTEGER NOT NULL,
    rtt_sum_us  INTEGER NOT NULL,
    rtt_min_us  INTEGER NOT NULL,
    rtt_max_us  INTEGER NOT NULL,
    histogram   BLOB    NOT NULL,
    PRIMARY KEY (role, epoch, tier, start_us)
);
CREATE INDEX rollups_by_role_tier_time ON rollups (role, tier, start_us);

-- How far each tier has been rolled up: every interval before `done_until_us` is done.
CREATE TABLE rollup_progress (
    tier          TEXT PRIMARY KEY CHECK (tier IN ('minute', 'hour')),
    done_until_us INTEGER NOT NULL
);
