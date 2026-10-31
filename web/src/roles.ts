// How roles appear on the main screen: the layered stack, from the host outward
// (ADR-0005).

import type { RoleKey } from "./model";

const LAYER_ORDER = ["nic", "tunnel", "local-interface", "hop", "target"] as const;
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

/** A role's label, for people. */
export function label(key: RoleKey): string {
  const l = layer(key);
  const [first, second] = numbers(key);
  switch (l) {
    case "nic":
      return "NIC";
    case "tunnel":
      return "Tunnel";
    case "local-interface":
      return "Local interface";
    case "hop":
      return `Hop ${String(second)} → target ${String(first)}`;
    case "target":
      return `Target ${String(first)}`;
  }
}
