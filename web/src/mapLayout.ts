// Where each network map node is drawn (ADR-0042). The daemon decides the nodes,
// edges, and depths; this only places them: one row per depth that has nodes, in
// depth order, and within a row an order that keeps edges from crossing where it
// can (the barycenter heuristic). No I/O, so it is tested on its own.

import type { MapNodeId, NetworkMap, RoleKey } from "./model";
import { compareRoles } from "./roles";

export const NODE_WIDTH = 184;
export const NODE_HEIGHT = 58;
export const GAP_X = 20;
export const GAP_Y = 46;
export const MARGIN = 16;

/** A node's top-left corner. */
export interface Position {
  readonly x: number;
  readonly y: number;
}

export interface Layout {
  readonly width: number;
  readonly height: number;
  readonly positions: ReadonlyMap<MapNodeId, Position>;
}

const SWEEPS = 8;

/** Places every node of `map`. The same map always gets the same layout. */
export function layout(map: NetworkMap): Layout {
  const depths = [...new Set(map.nodes.map((n) => n.depth))].sort((a, b) => a - b);

  // Start each row grouped by the first target whose path goes through each node,
  // so that one target's path tends to stay in one column.
  const firstTarget = (targets: readonly RoleKey[]): RoleKey | undefined => [...targets].sort(compareRoles)[0];
  const rows: MapNodeId[][] = depths.map((depth) =>
    map.nodes
      .filter((n) => n.depth === depth)
      .sort((a, b) => {
        const [x, y] = [firstTarget(a.targets), firstTarget(b.targets)];
        if (x !== y) return x === undefined ? -1 : y === undefined ? 1 : compareRoles(x, y);
        return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
      })
      .map((n) => n.id),
  );

  // A node's place in its row, centered on 0 so that rows of different lengths line
  // up on their middles.
  const place = new Map<MapNodeId, number>();
  const measure = (): void => {
    for (const row of rows) row.forEach((id, i) => place.set(id, i - (row.length - 1) / 2));
  };
  measure();
  const neighbors = (id: MapNodeId, upward: boolean): MapNodeId[] =>
    map.edges.filter((e) => (upward ? e.to === id : e.from === id)).map((e) => (upward ? e.from : e.to));

  for (let sweep = 0; sweep < SWEEPS; sweep++) {
    const downward = sweep % 2 === 0;
    const order = downward ? rows.keys() : [...rows.keys()].reverse();
    for (const r of order) {
      const row = rows[r];
      if (row === undefined) continue;
      const bary = new Map(
        row.map((id) => {
          const near = neighbors(id, downward).map((n) => place.get(n) ?? 0);
          const own = place.get(id) ?? 0;
          return [id, near.length === 0 ? own : near.reduce((a, b) => a + b, 0) / near.length];
        }),
      );
      // Ties keep their current order, so the result is stable.
      const current = new Map(row.map((id, i) => [id, i]));
      row.sort((a, b) => (bary.get(a) ?? 0) - (bary.get(b) ?? 0) || (current.get(a) ?? 0) - (current.get(b) ?? 0));
      row.forEach((id, i) => place.set(id, i - (row.length - 1) / 2));
    }
  }

  const widest = Math.max(0, ...rows.map((r) => r.length));
  const span = widest * NODE_WIDTH + Math.max(0, widest - 1) * GAP_X;
  const positions = new Map<MapNodeId, Position>();
  rows.forEach((row, r) => {
    const own = row.length * NODE_WIDTH + Math.max(0, row.length - 1) * GAP_X;
    const left = MARGIN + (span - own) / 2;
    row.forEach((id, i) =>
      positions.set(id, { x: left + i * (NODE_WIDTH + GAP_X), y: MARGIN + r * (NODE_HEIGHT + GAP_Y) }),
    );
  });
  return {
    width: span + 2 * MARGIN,
    height: rows.length * NODE_HEIGHT + Math.max(0, rows.length - 1) * GAP_Y + 2 * MARGIN,
    positions,
  };
}
