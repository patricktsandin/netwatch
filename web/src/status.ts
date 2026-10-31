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

/** The worse of two statuses: failing, then degraded, then healthy, then unknown. */
export function worse(a: Status, b: Status): Status {
  const rank: Record<Status, number> = { unknown: 0, healthy: 1, degraded: 2, failing: 3 };
  return rank[a] >= rank[b] ? a : b;
}

/** A role's status, with loss at a hop reinterpreted where the path says so. */
export type RowStatus = Status | "rate-limited";

/**
 * Loss at an intermediate hop that doesn't carry through to the hops beyond it, or to
 * the target, is the router deprioritizing ICMP, not loss on the path (ADR-0004). So
 * a degraded or failing hop with a healthy role further along the same path is
 * shown as `rate-limited`, and is never alarmed on.
 */
export function pathAware(own: Status, isHop: boolean, downstream: readonly Status[]): RowStatus {
  const lossy = own === "degraded" || own === "failing";
  return isHop && lossy && downstream.includes("healthy") ? "rate-limited" : own;
}
