-- 0010: an index of epochs by role. Never edit once applied.
--
-- Each start looks up every role's latest epoch (ADR-0044), and the epochs table is
-- kept forever.
CREATE INDEX IF NOT EXISTS epochs_by_role ON epochs (role, epoch);
