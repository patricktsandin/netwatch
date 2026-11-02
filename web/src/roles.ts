// How roles appear on the main screen: the layered stack, from the host outward
// (ADR-0005).

import type { RoleKey } from "./model";

const LAYER_ORDER = ["kernel", "nic", "wifi", "cellular", "tunnel", "local-interface", "hop", "target"] as const;
type Layer = (typeof LAYER_ORDER)[number];

function layer(key: RoleKey): Layer {
  const head = key.split(":")[0];
  return LAYER_ORDER.find((l) => l === head) ?? "target";
}

function numbers(key: RoleKey): number[] {
  return key.split(":").slice(1).map(Number);
}

/** The stack order: by layer, then numerically within a layer. */
export function compareRoles(a: RoleKey, b: RoleKey): number {
  const byLayer = LAYER_ORDER.indexOf(layer(a)) - LAYER_ORDER.indexOf(layer(b));
  if (byLayer !== 0) return byLayer;
  const [x, y] = [numbers(a), numbers(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}

/** A row's label: its role's, or for a row shared by several hops (ADR-0045), the hop
 *  and every target it leads to. */
export function rowLabel(members: readonly RoleKey[], names: ReadonlyMap<string, string> = new Map()): string {
  const [first] = members;
  if (first === undefined) return "";
  if (members.length === 1) return label(first, names);
  const hop = numbers(first)[1];
  const targets = members.map((m) => targetName(numbers(m)[0], names));
  return `Hop ${String(hop)} → ${targets.join(", ")}`;
}

/** A target's configured name, or else its number. */
function targetName(id: number | undefined, names: ReadonlyMap<string, string>): string {
  return names.get(`target:${String(id)}`) ?? `target ${String(id)}`;
}

/** A role's label, for people. `names` maps target role keys (`target:1`) to their
 *  configured names; targets without one are numbered. */
export function label(key: RoleKey, names: ReadonlyMap<string, string> = new Map()): string {
  const l = layer(key);
  const [first, second] = numbers(key);
  switch (l) {
    case "kernel":
      return "Kernel (loopback)";
    case "nic":
      return "NIC";
    case "wifi":
      return "Wi-Fi";
    case "cellular":
      return "Cellular";
    case "tunnel":
      return "Tunnel";
    case "local-interface":
      return "Local interface";
    case "hop":
      return `Hop ${String(second)} → ${targetName(first, names)}`;
    case "target":
      return names.get(key) ?? `Target ${String(first)}`;
  }
}
