import fc from "fast-check";
import { describe, expect, it } from "vitest";

import { parseMap } from "./api/parse";
import { GAP_Y, layout, NODE_HEIGHT, NODE_WIDTH } from "./mapLayout";
import type { NetworkMap } from "./model";

// Maps are built through the boundary parser, as the daemon's would be.
function node(id: string, depth: number, targets: string[] = []): unknown {
  return {
    id,
    kind: "role",
    depth,
    ttl: null,
    identity: null,
    name: null,
    status: null,
    roles: [],
    sightings: [],
    targets,
  };
}
function edge(from: string, to: string, targets: string[] = []): unknown {
  return { from, to, kind: "probed", targets };
}

/** An arbitrary DAG: nodes at random depths, edges only toward greater depths. */
const dag: fc.Arbitrary<NetworkMap> = fc
  .array(fc.integer({ min: 0, max: 8 }), { minLength: 1, maxLength: 24 })
  .chain((depths) => {
    const ids = depths.map((_, i) => `target:${String(i)}`);
    const pairs = depths.flatMap((d, i) => depths.flatMap((e, j) => (d < e ? [[i, j] as const] : [])));
    return fc.subarray(pairs).map((chosen) =>
      parseMap({
        nodes: depths.map((d, i) => node(ids[i] ?? "", d)),
        edges: chosen.map(([i, j]) => edge(ids[i] ?? "", ids[j] ?? "")),
      }),
    );
  });

describe("layout", () => {
  it("places every node once, inside the drawing", () => {
    fc.assert(
      fc.property(dag, (map) => {
        const { positions, width, height } = layout(map);
        expect(positions.size).toBe(map.nodes.length);
        for (const p of positions.values()) {
          expect(p.x).toBeGreaterThanOrEqual(0);
          expect(p.y).toBeGreaterThanOrEqual(0);
          expect(p.x + NODE_WIDTH).toBeLessThanOrEqual(width);
          expect(p.y + NODE_HEIGHT).toBeLessThanOrEqual(height);
        }
      }),
    );
  });

  it("draws one row per depth, deeper further down, with no two nodes overlapping", () => {
    fc.assert(
      fc.property(dag, (map) => {
        const { positions } = layout(map);
        for (const a of map.nodes) {
          for (const b of map.nodes) {
            const [p, q] = [positions.get(a.id), positions.get(b.id)];
            if (a === b || p === undefined || q === undefined) continue;
            if (a.depth === b.depth) {
              expect(p.y).toBe(q.y);
              expect(Math.abs(p.x - q.x)).toBeGreaterThanOrEqual(NODE_WIDTH);
            } else if (a.depth < b.depth) {
              expect(q.y - p.y).toBeGreaterThanOrEqual(NODE_HEIGHT + GAP_Y);
            }
          }
        }
      }),
    );
  });

  it("is the same every time for the same map", () => {
    fc.assert(
      fc.property(dag, (map) => {
        expect(layout(map)).toEqual(layout(map));
      }),
    );
  });

  it("keeps two paths that fork from one node in their own columns", () => {
    // The daemon lists target:2 first, which would cross the paths.
    const map = parseMap({
      nodes: [
        node("kernel", 0, ["target:1", "target:2"]),
        node("hop:1:1", 4, ["target:1"]),
        node("hop:2:1", 4, ["target:2"]),
        node("target:2", 5, ["target:2"]),
        node("target:1", 5, ["target:1"]),
      ],
      edges: [
        edge("kernel", "hop:1:1", ["target:1"]),
        edge("kernel", "hop:2:1", ["target:2"]),
        edge("hop:1:1", "target:1", ["target:1"]),
        edge("hop:2:1", "target:2", ["target:2"]),
      ],
    });
    const { positions } = layout(map);
    const x = (id: string): number => [...positions.entries()].find(([k]) => k === id)?.[1].x ?? NaN;
    expect(x("hop:1:1")).toBe(x("target:1"));
    expect(x("hop:2:1")).toBe(x("target:2"));
    expect(x("kernel")).toBe((x("hop:1:1") + x("hop:2:1")) / 2);
  });
});
