// Pure operations on a row's points: the live stream and backfill meet here, and they
// must merge without gaps or duplicates (ADR-0019, glossary: *backfill*).

import type { Micros, Point } from "./model";

/**
 * Merges two time-ordered point lists into one, ordered by time with no duplicates.
 * Two points are the same sample if they have the same time and epoch; the samples
 * from one daemon never share both.
 *
 * Properties, checked by fast-check: idempotent, commutative, associative, ordered,
 * and lossless (every input point is in the output, and nothing else is).
 */
export function merge(a: readonly Point[], b: readonly Point[]): Point[] {
  const out: Point[] = [];
  let i = 0;
  let j = 0;
  const push = (p: Point): void => {
    const last = out.at(-1);
    if (last === undefined || last.at !== p.at || last.epoch !== p.epoch) out.push(p);
  };
  for (;;) {
    const x = a[i];
    const y = b[j];
    if (x === undefined && y === undefined) return out;
    if (y === undefined || (x !== undefined && before(x, y))) {
      if (x !== undefined) push(x);
      i++;
    } else {
      push(y);
      j++;
    }
  }
}

/** Sample order: by time, then by epoch, so that the order is total. */
function before(x: Point, y: Point): boolean {
  return x.at < y.at || (x.at === y.at && x.epoch <= y.epoch);
}

/** Drops points older than `since`, as the visible window moves on. */
export function since(points: readonly Point[], cutoff: Micros): Point[] {
  return points.filter((p) => p.at >= cutoff);
}

/** The latest point's time, from which a reconnecting client backfills. */
export function latest(points: readonly Point[]): Micros | undefined {
  return points.at(-1)?.at;
}

/**
 * The points as plot columns, with a gap (`null`) wherever the line must not be
 * drawn: between points more than `maxGapUs` apart (the daemon wasn't measuring),
 * and between epochs (different identities are never joined, ADR-0003). Losses are
 * gaps too, and are drawn separately.
 */
export function plotColumns(
  points: readonly Point[],
  maxGapUs: number,
): { xs: number[]; ys: (number | null)[] } {
  const xs: number[] = [];
  const ys: (number | null)[] = [];
  points.forEach((p, i) => {
    const prev = i > 0 ? points[i - 1] : undefined;
    if (prev !== undefined && (p.at - prev.at > maxGapUs || p.epoch !== prev.epoch)) {
      xs.push((prev.at + p.at) / 2);
      ys.push(null);
    }
    xs.push(p.at);
    ys.push(p.measurement.kind === "round-trip" ? p.measurement.rttMs : null);
  });
  return { xs, ys };
}
