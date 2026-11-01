// Pure operations on a row's gauge readings (ADR-0032): merging the live stream with
// backfill, laying them out as plot columns, and formatting values in their unit.

import type { GaugeKey, GaugeUnit, Reading } from "./model";

const sameReading = (a: Reading, b: Reading): boolean => a.at === b.at && a.gauge === b.gauge && a.epoch === b.epoch;

function before(a: Reading, b: Reading): boolean {
  return a.at < b.at || (a.at === b.at && (a.gauge < b.gauge || (a.gauge === b.gauge && a.epoch < b.epoch)));
}

/** Merges two lists of readings into one, ordered by time with no duplicates (the
 *  same time, gauge, and epoch is the same reading). */
export function mergeReadings(a: readonly Reading[], b: readonly Reading[]): Reading[] {
  const all = [...a, ...b].sort((x, y) => (before(x, y) ? -1 : before(y, x) ? 1 : 0));
  return all.filter((r, i) => {
    const previous = all[i - 1];
    return previous === undefined || !sameReading(previous, r);
  });
}

/**
 * The readings as plot columns: one shared time column, and one value column per
 * gauge in `gauges`, `null` where that gauge has no reading. Like RTT lines, gauge
 * lines break where readings are more than `maxGapUs` apart and between epochs.
 */
export function gaugeColumns(
  readings: readonly Reading[],
  gauges: readonly GaugeKey[],
  maxGapUs: number,
): { xs: number[]; ys: (number | null)[][] } {
  const xs: number[] = [];
  const ys: (number | null)[][] = gauges.map(() => []);
  let last: Reading | undefined;
  for (const r of readings) {
    const column = gauges.indexOf(r.gauge);
    if (column === -1) continue;
    if (last !== undefined && last.at !== r.at && (r.at - last.at > maxGapUs || r.epoch !== last.epoch)) {
      xs.push((last.at + r.at) / 2);
      for (const y of ys) y.push(null);
    }
    if (last === undefined || last.at !== r.at) {
      xs.push(r.at);
      for (const y of ys) y.push(null);
    }
    const y = ys[column];
    if (y !== undefined) y[y.length - 1] = r.value;
    last = r;
  }
  return { xs, ys };
}

const PREFIXES: readonly (readonly [number, string])[] = [
  [1e9, "G"],
  [1e6, "M"],
  [1e3, "k"],
];

/** A value in its unit, with an SI prefix and at most three significant digits. */
export function formatValue(unit: GaugeUnit, value: number): string {
  const rounded = Number(value.toPrecision(3));
  for (const [size, prefix] of PREFIXES) {
    if (Math.abs(rounded) >= size) return `${String(Number((rounded / size).toPrecision(3)))} ${prefix}${unit}`;
  }
  return `${String(Math.round(value))} ${unit}`;
}

/** At most `count` evenly spaced tick values covering `min..max`, on a 1–2–5 step,
 *  always including zero when the range spans it. */
export function ticks(min: number, max: number, count: number): number[] {
  const span = max - min;
  if (!(span > 0) || count < 2) return [min];
  const rough = span / (count - 1);
  const magnitude = 10 ** Math.floor(Math.log10(rough));
  const inRange = (step: number): number => Math.floor(max / step + 1e-9) - Math.ceil(min / step - 1e-9) + 1;
  const step = [1, 2, 5, 10].map((m) => m * magnitude).find((s) => inRange(s) <= count) ?? 10 * magnitude;
  const out: number[] = [];
  for (let t = Math.ceil(min / step) * step; t <= max + step * 1e-9; t += step) {
    // Rounded to the step's precision, so 0.1 + 0.2 doesn't print as 0.30000000000000004.
    out.push(Number(t.toPrecision(12)));
  }
  return out;
}
