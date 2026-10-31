// A row's status (glossary: *status*), from its most recent points.

import type { Micros, Point } from "./model";

export type Status = "healthy" | "degraded" | "failing" | "unknown";

/** How many recent samples the status looks at. */
const WINDOW = 12;

/**
 * `unknown` when there's nothing recent (no sample within `staleAfterUs` of `now`);
 * otherwise by the share of losses among the last `WINDOW` samples: none is
 * `healthy`, under half is `degraded`, and half or more is `failing`.
 */
export function status(points: readonly Point[], now: Micros, staleAfterUs: number): Status {
  const last = points.at(-1);
  if (last === undefined || now - last.at > staleAfterUs) return "unknown";
  const recent = points.slice(-WINDOW);
  const lost = recent.filter((p) => p.measurement.kind === "lost").length;
  if (lost === 0) return "healthy";
  return lost * 2 < recent.length ? "degraded" : "failing";
}
