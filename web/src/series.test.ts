import fc from "fast-check";
import { describe, expect, it } from "vitest";

import { parsePoint } from "./api/parse";
import type { Point } from "./model";
import { merge, plotColumns } from "./series";

// Points are built through the boundary parser, so the tests use real domain values
// without any casts.
const point = fc
  .record({
    at: fc.integer({ min: 0, max: 50 }),
    epoch: fc.integer({ min: 1, max: 3 }),
    rtt: fc.option(fc.integer({ min: 0, max: 100_000 }), { nil: null }),
  })
  .map(({ at, epoch, rtt }) =>
    parsePoint({
      at_us: at,
      epoch,
      rtt_us: rtt,
      loss: rtt === null ? "timed-out" : null,
    }),
  );

const key = (p: Point): string => `${String(p.at)}/${String(p.epoch)}`;

/** A valid series: ordered, with no duplicate samples. */
const series = fc.array(point, { maxLength: 30 }).map((ps) => merge([], ps.sort(order)));
function order(x: Point, y: Point): number {
  return x.at - y.at || x.epoch - y.epoch;
}

const keys = (ps: readonly Point[]): string[] => ps.map(key);

describe("merge", () => {
  it("is idempotent", () => {
    fc.assert(
      fc.property(series, (a) => {
        expect(keys(merge(a, a))).toEqual(keys(a));
      }),
    );
  });

  it("is commutative", () => {
    fc.assert(
      fc.property(series, series, (a, b) => {
        expect(keys(merge(a, b))).toEqual(keys(merge(b, a)));
      }),
    );
  });

  it("is associative", () => {
    fc.assert(
      fc.property(series, series, series, (a, b, c) => {
        expect(keys(merge(merge(a, b), c))).toEqual(keys(merge(a, merge(b, c))));
      }),
    );
  });

  it("returns an ordered series without duplicates", () => {
    fc.assert(
      fc.property(series, series, (a, b) => {
        const m = merge(a, b);
        m.slice(1).forEach((p, i) => {
          const prev = m[i];
          expect(prev === undefined || order(prev, p) < 0).toBe(true);
        });
      }),
    );
  });

  it("loses nothing and invents nothing", () => {
    fc.assert(
      fc.property(series, series, (a, b) => {
        const expected = new Set([...keys(a), ...keys(b)]);
        expect(new Set(keys(merge(a, b)))).toEqual(expected);
      }),
    );
  });
});

describe("plotColumns", () => {
  const at = (t: number, epoch = 1) => parsePoint({ at_us: t, epoch, rtt_us: 1000, loss: null });

  it("joins close points of one epoch", () => {
    expect(plotColumns([at(0), at(5)], 10).ys).toEqual([1, 1]);
  });

  it("breaks the line across a measurement gap", () => {
    expect(plotColumns([at(0), at(50)], 10)).toEqual({ xs: [0, 25, 50], ys: [1, null, 1] });
  });

  it("breaks the line between epochs, however close", () => {
    expect(plotColumns([at(0, 1), at(1, 2)], 10).ys).toEqual([1, null, 1]);
  });
});
