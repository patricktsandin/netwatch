// What a row shows on hover, and the incidents it shades. The daemon decides what
// happened, and to which roles (ADR-0031); this module only picks out what lies
// under the cursor and puts it into words. It does no I/O.

import type { EventKind, Incident, Marker, Micros, Point, Reading, RoleKey } from "./model";

/** How times and roles are written. Injected, so the words don't depend on the
 *  viewer's time zone in tests. */
export interface Words {
  time(at: Micros): string;
  role(key: RoleKey): string;
  /** A reading with its gauge's label and unit, such as `received 1.25 Mbit/s`. */
  reading(reading: Reading): string;
}

export interface TooltipLine {
  readonly tone: "value" | "outage" | "degradation" | "event";
  readonly text: string;
}

/** Where the cursor is. `nearUs` is how close (in time) something must be to count
 *  as under the cursor: a few pixels' worth at the current zoom. */
export interface Hover {
  readonly at: Micros;
  readonly nearUs: number;
  readonly now: Micros;
}

export interface RowContent {
  /** The roles the row shows: one, or a shared row's members (ADR-0045). */
  readonly roles: readonly RoleKey[];
  readonly points: readonly Point[];
  readonly readings: readonly Reading[];
  readonly incidents: readonly Incident[];
  readonly markers: readonly Marker[];
}

const UNITS: readonly (readonly [string, number])[] = [
  ["d", 86400e6],
  ["h", 3600e6],
  ["min", 60e6],
  ["s", 1e6],
];

/** A duration in its two largest units: `3 min 20 s`, `2 h 5 min`. */
export function formatDuration(micros: number): string {
  const parts: string[] = [];
  let rest = Math.max(0, Math.round(micros / 1e6) * 1e6);
  for (const [unit, size] of UNITS) {
    const n = Math.floor(rest / size);
    rest -= n * size;
    if (n > 0 || (parts.length > 0 && parts.length < 2)) parts.push(`${String(n)} ${unit}`);
    if (parts.length === 2) break;
  }
  // A trailing zero unit says nothing: "3 min 0 s" is "3 min".
  const trimmed = parts.length === 2 && parts[1]?.startsWith("0 ") === true ? parts.slice(0, 1) : parts;
  return trimmed.length === 0 ? "0 s" : trimmed.join(" ");
}

const sameIncident = (a: Incident, b: Incident): boolean => a.role === b.role && a.started === b.started;

/** Adds an incident, or replaces the earlier report of the same one (same role and
 *  start), keeping the list in start order. */
export function upsertIncident(incidents: readonly Incident[], incident: Incident): Incident[] {
  const others = incidents.filter((i) => !sameIncident(i, incident));
  const at = others.findIndex((i) => i.started > incident.started);
  return at === -1 ? [...others, incident] : [...others.slice(0, at), incident, ...others.slice(at)];
}

/** The same event: members of a shared row each get a copy of it. */
const event = (m: Marker): string => `${String(m.at)} ${m.kind} ${m.detail ?? ""}`;

/** The markers a row draws: those that concern any of its roles, each event once. */
export function visibleMarkers(markers: readonly Marker[], roles: readonly RoleKey[]): Marker[] {
  const seen = new Set<string>();
  return markers.filter((m) => {
    const shown = m.concerns.some((c) => roles.includes(c)) && !seen.has(event(m));
    if (shown) seen.add(event(m));
    return shown;
  });
}

/** The same band: members of a shared row often have identical incidents. A
 *  different probable cause is a different incident, so the tooltip names each. */
const band = (i: Incident): string => `${String(i.started)} ${String(i.ended)} ${i.severity} ${i.cause ?? ""}`;

/** A row's incidents that overlap `from..until`, open ones included. Identical
 *  bands of a shared row's members are drawn once. */
export function visibleIncidents(
  incidents: readonly Incident[],
  roles: readonly RoleKey[],
  from: Micros,
  until: Micros,
): Incident[] {
  const seen = new Set<string>();
  return incidents.filter((i) => {
    const shown =
      roles.includes(i.role) && i.started <= until && (i.ended === null || i.ended >= from) && !seen.has(band(i));
    if (shown) seen.add(band(i));
    return shown;
  });
}

function nearest(points: readonly Point[], at: Micros): Point | undefined {
  let lo = 0;
  let hi = points.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    const p = points[mid];
    if (p !== undefined && p.at < at) lo = mid + 1;
    else hi = mid;
  }
  const candidates = [points[lo - 1], points[lo]].filter((p) => p !== undefined);
  return candidates.reduce<Point | undefined>(
    (best, p) => (best === undefined || Math.abs(p.at - at) < Math.abs(best.at - at) ? p : best),
    undefined,
  );
}

function nearestReading(readings: readonly Reading[], at: Micros): Reading | undefined {
  return readings.reduce<Reading | undefined>(
    (best, r) => (best === undefined || Math.abs(r.at - at) < Math.abs(best.at - at) ? r : best),
    undefined,
  );
}

function valueText(point: Point): string {
  const m = point.measurement;
  return m.kind === "round-trip" ? `${m.rttMs.toFixed(2)} ms` : `lost (${m.reason.replace(/-/g, " ")})`;
}

function incidentText(i: Incident, now: Micros, words: Words): string {
  const what = i.severity === "outage" ? "Outage" : "Degradation";
  const when =
    i.ended === null
      ? `since ${words.time(i.started)} (ongoing, ${formatDuration(now - i.started)})`
      : `${words.time(i.started)}–${words.time(i.ended)} (${formatDuration(i.ended - i.started)})`;
  const cause = i.cause === null ? "" : ` · probable cause: ${words.role(i.cause)}`;
  return `${what} ${when}${cause}`;
}

function eventText(m: Marker, words: Words): string {
  const detail = m.detail ?? "";
  const role = m.role === null ? "" : words.role(m.role);
  const texts: Record<EventKind, string> = {
    "epoch-started": `${role} is now ${detail}`,
    "epoch-resumed": `${role} is still ${detail}`,
    started: detail,
    "epoch-ended": `${role} lost its identity`,
    desynchronized: "lost track of topology changes; resyncing",
    resynchronized: "topology resynchronized",
    "kernel-log": `kernel: ${detail}`,
    "source-disabled": `disabled: ${detail}`,
    "clock-jump": detail,
    "config-drift": `sysctl changed: ${detail}`,
    radio: `${role} changed: ${detail}`,
    "public-address": `public address: ${detail}`,
    trace: detail,
  };
  return `${words.time(m.at)} · ${texts[m.kind]}`;
}

/** Everything under the cursor on one row: the nearest value, the incidents
 *  covering that time, and the nearby events that concern the row. */
export function tooltip(hover: Hover, row: RowContent, words: Words): TooltipLine[] {
  const lines: TooltipLine[] = [];
  const point = nearest(row.points, hover.at);
  if (point !== undefined && Math.abs(point.at - hover.at) <= hover.nearUs) {
    lines.push({ tone: "value", text: valueText(point) });
  }
  const reading = nearestReading(row.readings, hover.at);
  if (reading !== undefined && Math.abs(reading.at - hover.at) <= hover.nearUs) {
    const together = row.readings.filter((r) => r.at === reading.at);
    lines.push({ tone: "value", text: together.map((r) => words.reading(r)).join(" · ") });
  }
  // On a shared row, one line per band, naming every path it is on.
  const under = new Map<string, Incident[]>();
  for (const i of row.incidents) {
    const end = i.ended ?? hover.now;
    if (row.roles.includes(i.role) && i.started - hover.nearUs <= hover.at && hover.at <= end + hover.nearUs) {
      under.set(band(i), [...(under.get(band(i)) ?? []), i]);
    }
  }
  for (const same of under.values()) {
    const [first] = same;
    if (first === undefined) continue;
    const on = row.roles.length > 1 ? ` · on ${same.map((i) => words.role(i.role)).join(", ")}` : "";
    lines.push({ tone: first.severity, text: `${incidentText(first, hover.now, words)}${on}` });
  }
  for (const m of visibleMarkers(row.markers, row.roles)) {
    if (Math.abs(m.at - hover.at) <= hover.nearUs) {
      lines.push({ tone: "event", text: eventText(m, words) });
    }
  }
  return lines;
}
