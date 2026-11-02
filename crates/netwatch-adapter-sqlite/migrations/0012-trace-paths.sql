-- 0012: the path each trace ran on, the egress interface and the local address
-- (ADR-0056), so that the map draws only traces of the current path. NULL for traces
-- recorded before it was kept. Never edit once applied.

ALTER TABLE traces ADD COLUMN path TEXT;
