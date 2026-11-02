// Where each network map node is drawn (ADR-0042). The daemon decides the nodes,
// edges, and depths; this only places them: one row per depth that has nodes, in
// depth order. An edge that skips rows gets a narrow slot of its own in each row it
// crosses, so it never runs through a node. Within a row, nodes and slots are ordered
// to keep edges from crossing where they can (the barycenter heuristic). No I/O, so
// it is tested on its own.

import type { MapNodeId, NetworkMap, RoleKey } from "./model";
import { compareRoles } from "./roles";

export const NODE_WIDTH = 184;
export const NODE_HEIGHT = 58;
export const GAP_X = 20;
export const GAP_Y = 46;
export const MARGIN = 16;
/** The width of a slot that an edge passes through. */
export const SLOT_WIDTH = 8;

/** A node's top-left corner, or a point on an edge's route. */
export interface Position {
  readonly x: number;
  readonly y: number;
}

export interface Layout {
  readonly width: number;
  readonly height: number;
  readonly positions: ReadonlyMap<MapNodeId, Position>;
  /** For each edge, by its index in `map.edges`, where it crosses the rows between
   *  its ends: the middle of its slot in each, top to bottom. */
  readonly routes: readonly (readonly Position[])[];
}

const SWEEPS = 8;

/** One thing in a row: a node, or one edge's slot. */
type Item = { readonly node: MapNodeId } | { readonly edge: number; readonly row: number };
const keyOf = (item: Item): string =>
  "node" in item ? `n ${item.node}` : `e ${String(item.edge)} ${String(item.row)}`;
const widthOf = (item: Item): number => ("node" in item ? NODE_WIDTH : SLOT_WIDTH);

/** Places every node of `map`, and routes every edge. The same map always gets the
 *  same layout. */
export function layout(map: NetworkMap): Layout {
  const depths = [...new Set(map.nodes.map((n) => n.depth))].sort((a, b) => a - b);
  const rowOf = new Map(map.nodes.map((n) => [n.id, depths.indexOf(n.depth)]));

  // Start each row grouped by the first target whose path goes through each node,
  // so that one target's path tends to stay in one column.
  const firstTarget = (targets: readonly RoleKey[]): RoleKey | undefined => [...targets].sort(compareRoles)[0];
  const rows: Item[][] = depths.map((depth) =>
    map.nodes
      .filter((n) => n.depth === depth)
      .sort((a, b) => {
        const [x, y] = [firstTarget(a.targets), firstTarget(b.targets)];
        if (x !== y) return x === undefined ? -1 : y === undefined ? 1 : compareRoles(x, y);
        return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
      })
      .map((n): Item => ({ node: n.id })),
  );

  // Each edge becomes a chain of links between adjacent rows, through its slots.
  const above = new Map<string, string[]>();
  const below = new Map<string, string[]>();
  const link = (a: string, b: string): void => {
    below.set(a, [...(below.get(a) ?? []), b]);
    above.set(b, [...(above.get(b) ?? []), a]);
  };
  map.edges.forEach((e, i) => {
    const [from, to] = [rowOf.get(e.from) ?? 0, rowOf.get(e.to) ?? 0];
    let previous = keyOf({ node: e.from });
    for (let r = from + 1; r < to; r++) {
      const slot: Item = { edge: i, row: r };
      rows[r]?.push(slot);
      link(previous, keyOf(slot));
      previous = keyOf(slot);
    }
    link(previous, keyOf({ node: e.to }));
  });

  // Each item's horizontal extent, with every row centered on 0.
  const extent = new Map<string, { left: number; right: number }>();
  const measure = (row: readonly Item[]): void => {
    const total = row.reduce((sum, item) => sum + widthOf(item), 0) + Math.max(0, row.length - 1) * GAP_X;
    let x = -total / 2;
    for (const item of row) {
      extent.set(keyOf(item), { left: x, right: x + widthOf(item) });
      x += widthOf(item) + GAP_X;
    }
  };
  const center = (key: string): number => {
    const e = extent.get(key);
    return e === undefined ? 0 : (e.left + e.right) / 2;
  };
  rows.forEach(measure);

  for (let sweep = 0; sweep < SWEEPS; sweep++) {
    const downward = sweep % 2 === 0;
    const order = downward ? [...rows.keys()] : [...rows.keys()].reverse();
    for (const r of order) {
      const row = rows[r];
      if (row === undefined) continue;
      const rank = new Map(
        row.map((item, i) => {
          const key = keyOf(item);
          const near = ((downward ? above : below).get(key) ?? []).map(center);
          const bary = near.length === 0 ? center(key) : near.reduce((a, b) => a + b, 0) / near.length;
          return [key, { bary, i }];
        }),
      );
      // Ties keep their current order, so the result is stable.
      row.sort((a, b) => {
        const [x, y] = [rank.get(keyOf(a)), rank.get(keyOf(b))];
        return x === undefined || y === undefined ? 0 : x.bary - y.bary || x.i - y.i;
      });
      measure(row);
    }
  }

  const extents = [...extent.values()];
  const left = Math.min(0, ...extents.map((e) => e.left));
  const right = Math.max(0, ...extents.map((e) => e.right));
  const x = (key: string): number => MARGIN + center(key) - left;
  const top = (r: number): number => MARGIN + r * (NODE_HEIGHT + GAP_Y);

  const positions = new Map<MapNodeId, Position>();
  for (const n of map.nodes) {
    positions.set(n.id, { x: x(keyOf({ node: n.id })) - NODE_WIDTH / 2, y: top(rowOf.get(n.id) ?? 0) });
  }
  const routes = map.edges.map((e, i) => {
    const [from, to] = [rowOf.get(e.from) ?? 0, rowOf.get(e.to) ?? 0];
    const route: Position[] = [];
    for (let r = from + 1; r < to; r++) route.push({ x: x(keyOf({ edge: i, row: r })), y: top(r) + NODE_HEIGHT / 2 });
    return route;
  });
  return {
    width: right - left + 2 * MARGIN,
    height: rows.length * NODE_HEIGHT + Math.max(0, rows.length - 1) * GAP_Y + 2 * MARGIN,
    positions,
    routes,
  };
}
