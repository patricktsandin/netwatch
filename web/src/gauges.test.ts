import fc from "fast-check";
import { describe, expect, it } from "vitest";

import { parseReading } from "./api/parse";
import { formatValue, gaugeColumns, mergeReadings, ticks } from "./gauges";
import type { GaugeKey, Reading } from "./model";

const reading = (at: number, gauge: string, value: number, epoch = 1): Reading =>
  parseReading({ at_us: at * 1e6, epoch, gauge, value });
const key = (gauge: string): GaugeKey => reading(0, gauge, 0).gauge;

describe("formatValue", () => {
  it.each([
    [0, "0 bit/s"],
    [950, "950 bit/s"],
    [12_345, "12.3 kbit/s"],
    [1_250_000, "1.25 Mbit/s"],
    [999_999_999, "1 Gbit/s"],
    [5_000_000, "5 Mbit/s"],
  ])("%d bit/s reads %s", (value, text) => {
    expect(formatValue("bit/s", value)).toBe(text);
  });

  it("writes signal levels and percentages without prefixes", () => {
    expect(formatValue("dBm", -67.4)).toBe("-67 dBm");
    expect(formatValue("dB", 12.5)).toBe("13 dB");
    expect(formatValue("%", 64)).toBe("64 %");
  });
});

describe("mergeReadings", () => {
  const aReading = fc
    .record({ at: fc.integer({ min: 0, max: 20 }), gauge: fc.constantFrom("rx-throughput", "tx-throughput") })
    .map(({ at, gauge }) => reading(at, gauge, at));

  it("is ordered, free of duplicates, and idempotent", () => {
    fc.assert(
      fc.property(fc.array(aReading), fc.array(aReading), (a, b) => {
        const merged = mergeReadings(mergeReadings([], a), b);
        const ids = merged.map((r) => `${String(r.at)}/${r.gauge}`);
        expect(new Set(ids).size).toBe(ids.length);
        expect(merged.map((r) => r.at)).toEqual([...merged.map((r) => r.at)].sort((x, y) => x - y));
        expect(mergeReadings(merged, b)).toEqual(merged);
        expect(new Set(ids)).toEqual(new Set([...a, ...b].map((r) => `${String(r.at)}/${r.gauge}`)));
      }),
    );
  });
});

describe("gaugeColumns", () => {
  it("lines up each gauge's values on one time axis", () => {
    const columns = gaugeColumns(
      [reading(1, "rx-throughput", 10), reading(1, "tx-throughput", 1), reading(2, "rx-throughput", 20)],
      [key("rx-throughput"), key("tx-throughput")],
      5e6,
    );
    expect(columns).toEqual({
      xs: [1e6, 2e6],
      ys: [
        [10, 20],
        [1, null],
      ],
    });
  });

  it("breaks the lines at a measurement gap and between epochs", () => {
    const columns = gaugeColumns(
      [reading(1, "rx-throughput", 10), reading(20, "rx-throughput", 20), reading(21, "rx-throughput", 30, 2)],
      [key("rx-throughput")],
      5e6,
    );
    expect(columns.ys[0]).toEqual([10, null, 20, null, 30]);
  });
});

describe("ticks", () => {
  it.each([
    [0, 9.3e6, [0, 5e6]],
    [0, 1.15, [0, 0.5, 1]],
    [0, 46, [0, 20, 40]],
    [0, 1, [0, 0.5, 1]],
  ])("from %d to %d: %j", (min, max, expected) => {
    expect(ticks(min, max, 3)).toEqual(expected);
  });

  it("spans negative ranges too, through zero", () => {
    expect(ticks(-95, 0, 3)).toEqual([-50, 0]);
  });
});
