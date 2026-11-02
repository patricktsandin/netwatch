// Draws the network map (ADR-0042) as SVG. The daemon built the map and graded every
// node; mapLayout placed them. This file only turns both into elements, and lights up
// the paths through a node under the pointer.

import { layout, NODE_HEIGHT, NODE_WIDTH } from "./mapLayout";
import type { MapNode, Micros, NetworkMap, RoleKey, Status } from "./model";

export interface MapWords {
  /** A role's label, as the rows title it. */
  readonly role: (key: RoleKey) => string;
  /** A time of day. */
  readonly time: (at: Micros) => string;
}

const SVG = "http://www.w3.org/2000/svg";

function svg<K extends keyof SVGElementTagNameMap>(
  tag: K,
  attributes: Record<string, string | number>,
  text = "",
): SVGElementTagNameMap[K] {
  const el = document.createElementNS(SVG, tag);
  for (const [name, value] of Object.entries(attributes)) el.setAttribute(name, String(value));
  if (text !== "") el.textContent = text;
  return el;
}

/** `text`, cut to `max` characters with an ellipsis. The full text is in the title. */
function fit(text: string, max: number): string {
  return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}

const statusWord = (s: Status): string => (s === "rate-limited" ? "rate-limited?" : s);

function title(node: MapNode, words: MapWords): string {
  const hop = node.ttl === null ? "" : `Hop ${String(node.ttl)}`;
  switch (node.kind) {
    case "router":
      return hop;
    case "silent":
      return hop;
    case "role":
      return node.role === null ? node.id : words.role(node.role);
  }
}

function sightingText(node: MapNode, words: MapWords): string {
  const s = node.sightings[0];
  if (s === undefined) return "";
  const median = s.medianUs === null ? "" : ` · ${(s.medianUs / 1000).toFixed(1)} ms`;
  return `traced ${words.time(s.at)} · ${String(s.answered)}/${String(s.sent)}${median}`;
}

function details(node: MapNode, words: MapWords): string {
  return [
    title(node, words),
    node.identity,
    node.name,
    node.status === null ? "Not measured continuously; seen by a path trace." : `Status: ${statusWord(node.status)}`,
    ...node.roles.map((r) => `${words.role(r.key)}: ${statusWord(r.status)}`),
    ...node.sightings.map((s) => {
      const median = s.medianUs === null ? "" : `, median ${(s.medianUs / 1000).toFixed(1)} ms`;
      return `Trace toward ${words.role(s.target)} at ${words.time(s.at)}: ${String(s.answered)} of ${String(s.sent)} answered${median}`;
    }),
    node.targets.length === 0 ? "On no target's path" : `Paths: ${node.targets.map(words.role).join(", ")}`,
    node.roles.length > 0 ? "Click to see its graph." : "",
  ]
    .filter((line): line is string => line !== null && line !== "")
    .join("\n");
}

/** Replaces `container`'s contents with `map`. Clicking a node with a role calls
 *  `open` with it. */
export function renderMap(
  container: HTMLElement,
  map: NetworkMap,
  words: MapWords,
  open: (role: RoleKey) => void,
): void {
  if (map.nodes.length === 0) {
    const empty = document.createElement("p");
    empty.className = "hint";
    empty.textContent = "No paths yet. The map fills in once the daemon has graded its first probes.";
    container.replaceChildren(empty);
    return;
  }
  const { width, height, positions, routes } = layout(map);
  const root = svg("svg", {
    width,
    height,
    viewBox: `0 0 ${String(width)} ${String(height)}`,
    role: "img",
    "aria-label": "Network map",
  });

  const edges = map.edges.map((e, i) => {
    const [a, b] = [positions.get(e.from), positions.get(e.to)];
    if (a === undefined || b === undefined) return null;
    // From the bottom of one node to the top of the other, straight down through
    // each row the edge crosses, and curving across the gaps between rows.
    const points = [
      { x: a.x + NODE_WIDTH / 2, y: a.y + NODE_HEIGHT },
      ...(routes[i] ?? []).flatMap((p) => [
        { x: p.x, y: p.y - NODE_HEIGHT / 2 },
        { x: p.x, y: p.y + NODE_HEIGHT / 2 },
      ]),
      { x: b.x + NODE_WIDTH / 2, y: b.y },
    ];
    let d = "";
    for (let k = 0; k + 1 < points.length; k += 2) {
      const [p, q] = [points[k], points[k + 1]];
      if (p === undefined || q === undefined) continue;
      const bend = (q.y - p.y) / 2;
      d += `${k === 0 ? "M" : "L"} ${String(p.x)} ${String(p.y)} C ${String(p.x)} ${String(p.y + bend)}, ${String(q.x)} ${String(q.y - bend)}, ${String(q.x)} ${String(q.y)} `;
    }
    const path = svg("path", { d: d.trim(), class: `map-edge ${e.kind}` });
    path.append(
      svg("title", {}, `${e.kind}${e.targets.length > 0 ? `: ${e.targets.map(words.role).join(", ")}` : ""}`),
    );
    root.append(path);
    return { path, targets: e.targets };
  });

  const light = (targets: readonly RoleKey[] | null): void => {
    for (const edge of edges) {
      if (edge === null) continue;
      const on = targets !== null && edge.targets.some((t) => targets.includes(t));
      edge.path.classList.toggle("lit", on);
      edge.path.classList.toggle("dim", targets !== null && !on);
    }
  };

  for (const node of map.nodes) {
    const p = positions.get(node.id);
    if (p === undefined) continue;
    const g = svg("g", {
      class: `map-node ${node.status ?? "traced"}${node.roles.length > 0 ? " openable" : ""}`,
      transform: `translate(${String(p.x)} ${String(p.y)})`,
      tabindex: node.roles.length > 0 ? 0 : -1,
    });
    g.append(svg("title", {}, details(node, words)));
    g.append(svg("rect", { width: NODE_WIDTH, height: NODE_HEIGHT, rx: 7 }));
    g.append(svg("circle", { cx: 12, cy: 15, r: 4.5, class: "dot" }));
    g.append(svg("text", { x: 23, y: 19, class: "name" }, fit(title(node, words), 18)));
    g.append(
      svg(
        "text",
        { x: NODE_WIDTH - 8, y: 19, class: "badge", "text-anchor": "end" },
        node.status === null ? "traced" : statusWord(node.status),
      ),
    );
    const second = node.identity ?? (node.kind === "silent" ? "no reply" : "");
    const third = node.name ?? sightingText(node, words);
    g.append(svg("text", { x: 10, y: 37, class: "line" }, fit(second, 28)));
    g.append(svg("text", { x: 10, y: 51, class: "line muted" }, fit(third, 30)));
    g.addEventListener("mouseenter", () => {
      light(node.targets);
    });
    g.addEventListener("mouseleave", () => {
      light(null);
    });
    const role = node.roles[0]?.key;
    if (role !== undefined) {
      const go = (): void => {
        open(role);
      };
      g.addEventListener("click", go);
      g.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") go();
      });
    }
    root.append(g);
  }
  container.replaceChildren(root);
}
