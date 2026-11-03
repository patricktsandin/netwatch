-- 0013: traces are kept forever, and reports and the map look each target's up by
-- role, newest first (ADR-0056). Never edit once applied.

CREATE INDEX IF NOT EXISTS traces_by_role_and_time ON traces (role, at_us);
