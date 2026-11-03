-- 0014: rolling up and pruning look samples up by time alone, every minute and every
-- hour; without this index each scans all 7 days. Never edit once applied.

CREATE INDEX IF NOT EXISTS samples_by_time ON samples (at_us);
