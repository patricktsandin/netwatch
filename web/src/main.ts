// The app shell: backfill, the live stream, and the layered stack of rows. All the
// decisions live in pure modules (series, status, roles); this file only wires them
// to the DOM and the network.

import "uplot/dist/uPlot.min.css";
import "./style.css";

import {
  ParseError,
  parseLayers,
  parseLive,
  parseMarkers,
  parseMicros,
  parsePoints,
  parseRoles,
  parseRollups,
  parseTargetNames,
} from "./api/parse";
import { createChart, type Chart } from "./chart";
import type { LayerReport, Marker, Micros, Point, RoleKey } from "./model";
import { compareRoles, label } from "./roles";
import { latest, merge, since } from "./series";
import { pathAware, status, worse, type Status } from "./status";

/** The time ranges the UI offers: raw samples for the last hour, then the rollup
 *  tiers (ADR-0006). */
type Range = "hour" | "day" | "month";
interface RangeSpec {
  readonly spanUs: number;
  readonly tier: "minute" | "hour" | null;
  readonly maxGapUs: number;
  /** A row with nothing newer than this is `unknown`. Rollups appear a few minutes
   *  after their interval starts, so the longer ranges allow more. */
  readonly staleAfterUs: number;
}
const RANGES: Readonly<Record<Range, RangeSpec>> = {
  hour: { spanUs: 3600e6, tier: null, maxGapUs: 15e6, staleAfterUs: 15e6 },
  day: { spanUs: 24 * 3600e6, tier: "minute", maxGapUs: 2 * 60e6, staleAfterUs: 5 * 60e6 },
  month: { spanUs: 30 * 24 * 3600e6, tier: "hour", maxGapUs: 2 * 3600e6, staleAfterUs: 3 * 3600e6 },
};
let range: Range = "hour";

interface Row {
  points: Point[];
  chart: Chart;
  plotEl: HTMLElement;
  titleEl: HTMLElement;
  statusEl: HTMLElement;
  valueEl: HTMLElement;
}

const rows = new Map<RoleKey, Row>();
let markers: Marker[] = [];
const layers = new Map<RoleKey, LayerReport>();
let names: ReadonlyMap<string, string> = new Map();
let zoom: { min: number; max: number } | null = null;

const nowMicros = (): Micros => parseMicros(Date.now() * 1000);

async function fetchJson(path: string): Promise<unknown> {
  const response = await fetch(path);
  if (!response.ok) throw new Error(`${path}: HTTP ${String(response.status)}`);
  const body: unknown = await response.json();
  return body;
}

function element(tag: string, className: string, text = ""): HTMLElement {
  const el = document.createElement(tag);
  el.className = className;
  el.textContent = text;
  return el;
}

function addRow(key: RoleKey): Row {
  const existing = rows.get(key);
  if (existing !== undefined) return existing;
  const stack = document.getElementById("stack");
  if (stack === null) throw new Error("index.html has no #stack");
  const rowEl = element("section", "row");
  const head = element("header", "row-head");
  const statusEl = element("span", "status unknown", "unknown");
  const valueEl = element("span", "value");
  const titleEl = element("h2", "row-title", label(key, names));
  head.append(titleEl, statusEl, valueEl);
  const plotEl = element("div", "plot");
  rowEl.append(head, plotEl);
  rowEl.dataset["role"] = key;

  const ordered = [...rows.keys(), key].sort(compareRoles);
  const next = ordered[ordered.indexOf(key) + 1];
  const nextEl = next === undefined ? null : stack.querySelector(`[data-role="${next}"]`);
  stack.insertBefore(rowEl, nextEl);

  // The chart reads the row's points while it is being constructed, so the points
  // need a home that exists before the chart does.
  const series: { points: Point[] } = { points: [] };
  const chart = createChart(plotEl, () => series.points, () => markers, setZoom, () => RANGES[range].maxGapUs);
  const row: Row = {
    get points() {
      return series.points;
    },
    set points(points: Point[]) {
      series.points = points;
    },
    statusEl,
    valueEl,
    titleEl,
    plotEl,
    chart,
  };
  rows.set(key, row);
  return row;
}

/** The selected range, ending now. */
function sharedWindow(): { min: number; max: number } {
  const now = nowMicros() / 1e6;
  return { min: now - RANGES[range].spanUs / 1e6, max: now };
}

function setZoom(min: number, max: number): void {
  zoom = { min, max };
  for (const row of rows.values()) row.chart.setWindow(min, max);
  const reset = document.getElementById("reset-zoom");
  if (reset !== null) reset.hidden = false;
}

/** The roles further along the same path: later hops toward the same target, and
 *  the target itself. */
function downstreamOf(key: RoleKey): RoleKey[] {
  const [kind, target, ttl] = key.split(":");
  if (kind !== "hop" || target === undefined || ttl === undefined) return [];
  return [...rows.keys()].filter((other) => {
    const [k, t, n] = other.split(":");
    return t === target && ((k === "hop" && Number(n) > Number(ttl)) || k === "target");
  });
}

function ownStatus(row: Row): Status {
  return status(row.points, nowMicros(), RANGES[range].staleAfterUs);
}

/** A layer row combines its probes, if any, with its health report (ADR-0008). */
function layerStatus(key: RoleKey, row: Row): Status {
  const report = layers.get(key);
  if (report === undefined) return ownStatus(row);
  return row.points.length === 0 ? report.status : worse(ownStatus(row), report.status);
}

function layerSummary(key: RoleKey): string {
  const report = layers.get(key);
  if (report === undefined) return "";
  const cpu = report.cpuPressure === null ? "" : `CPU pressure ${report.cpuPressure.toFixed(1)}%`;
  const grew = report.increases.map((i) => `${i.name} +${String(i.by)}`);
  return [cpu, ...grew].filter((s) => s !== "").join(" · ");
}

function render(row: Row, key: RoleKey): void {
  row.chart.update(row.points);
  // Every row shares one time axis (ADR-0005): the zoom if there is one, otherwise
  // from the earliest point of any row up to now.
  const window = zoom ?? sharedWindow();
  row.chart.setWindow(window.min, window.max);
  const downstream = downstreamOf(key).flatMap((k) => {
    const other = rows.get(k);
    return other === undefined ? [] : [ownStatus(other)];
  });
  row.plotEl.hidden = row.points.length === 0;
  const own = layerStatus(key, row);
  const s = pathAware(own, key.startsWith("hop:"), downstream);
  row.statusEl.className = `status ${s}`;
  row.statusEl.textContent = s === "rate-limited" ? "rate-limited?" : s;
  row.statusEl.title =
    s === "rate-limited" ? "Loss here doesn't carry through to later hops: the router is deprioritizing ICMP." : "";
  const last = row.points.at(-1);
  const latestValue =
    last === undefined
      ? ""
      : last.measurement.kind === "round-trip"
        ? `${last.measurement.rttMs.toFixed(2)} ms`
        : last.measurement.reason;
  const extra = layerSummary(key);
  row.valueEl.textContent = [extra, latestValue].filter((s) => s !== "").join(" · ");
}

/** The current identity of each global role, from the latest unended epoch. */
function renderIdentities(): void {
  const el = document.getElementById("identities");
  if (el === null) return;
  const open = new Map<string, Marker>();
  for (const m of markers) {
    if (m.role === null) continue;
    if (m.kind === "epoch-started") open.set(m.role, m);
    if (m.kind === "epoch-ended" && open.get(m.role)?.epoch === m.epoch) open.delete(m.role);
  }
  // Global roles go in the header; a hop's router goes in its own row's title.
  el.replaceChildren(
    ...[...open.entries()]
      .filter(([role]) => !role.startsWith("target") && !role.startsWith("hop"))
      .map(([role, m]) => element("span", "identity", `${role}: ${m.detail ?? "?"} (epoch ${String(m.epoch)})`)),
  );
  for (const [key, row] of rows) {
    const identity = open.get(key)?.detail;
    row.titleEl.textContent = key.startsWith("hop") && identity != null ? `${label(key, names)} · ${identity}` : label(key, names);
  }
}

async function backfill(from: Micros): Promise<void> {
  const [roles, events] = await Promise.all([
    fetchJson("/api/roles").then(parseRoles),
    fetchJson(`/api/events?since_us=${String(from)}`).then(parseMarkers),
  ]);
  markers = markers.concat(events.filter((e) => !markers.some((m) => m.at === e.at && m.kind === e.kind && m.epoch === e.epoch)));
  for (const [key, report] of parseLayers(await fetchJson("/api/layers"))) {
    layers.set(key, report);
    render(addRow(key), key);
  }
  names = parseTargetNames(await fetchJson("/api/targets"));
  await Promise.all(
    roles.map(async (key) => {
      const row = addRow(key);
      const tier = RANGES[range].tier;
      if (tier === null) {
        const fetched = parsePoints(
          await fetchJson(`/api/samples?role=${encodeURIComponent(key)}&since_us=${String(latest(row.points) ?? from)}`),
        );
        row.points = merge(row.points, fetched);
      } else {
        // Rollups are re-read whole: the latest interval keeps growing until it closes.
        row.points = [
          ...parseRollups(
            await fetchJson(`/api/rollups?role=${encodeURIComponent(key)}&tier=${tier}&since_us=${String(from)}`),
          ),
        ];
      }
      render(row, key);
    }),
  );
  renderIdentities();
}

function connect(): void {
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  const socket = new WebSocket(`${scheme}://${location.host}/api/live`);
  const banner = document.getElementById("connection");
  socket.onopen = () => {
    if (banner !== null) banner.hidden = true;
    // Anything that happened while disconnected is fetched, and merged without
    // duplicates.
    const earliestLatest = [...rows.values()].map((r) => latest(r.points)).filter((t) => t !== undefined);
    const from =
      range === "hour" && earliestLatest.length > 0 ? Math.min(...earliestLatest) : nowMicros() - RANGES[range].spanUs;
    void backfill(parseMicros(from)).catch(report);
  };
  socket.onmessage = (event: MessageEvent) => {
    try {
      const live = parseLive(JSON.parse(String(event.data)));
      switch (live.type) {
        case "sample": {
          // Live samples are raw, so they only join the raw range.
          if (range !== "hour") break;
          const row = addRow(live.role);
          row.points = merge(row.points, [live.point]);
          render(row, live.role);
          break;
        }
        case "layer":
          layers.set(live.role, live.report);
          render(addRow(live.role), live.role);
          break;
        case "event":
          markers.push(live.marker);
          renderIdentities();
          for (const row of rows.values()) row.chart.redraw();
          break;
      }
    } catch (error) {
      report(error);
    }
  };
  socket.onclose = () => {
    if (banner !== null) banner.hidden = false;
    setTimeout(connect, 2000);
  };
}

function report(error: unknown): void {
  const kind = error instanceof ParseError ? "unexpected data from the daemon" : "error";
  console.error(kind, error);
}

function trim(): void {
  const cutoff = nowMicros() - RANGES[range].spanUs;
  for (const [key, row] of rows) {
    row.points = since(row.points, parseMicros(cutoff));
    render(row, key);
  }
  markers = markers.filter((m) => m.at >= cutoff);
}

document.getElementById("reset-zoom")?.addEventListener("click", (event) => {
  zoom = null;
  if (event.currentTarget instanceof HTMLElement) event.currentTarget.hidden = true;
  for (const [key, row] of rows) render(row, key);
});
function selectRange(selected: Range): void {
  range = selected;
  history.replaceState(null, "", selected === "hour" ? location.pathname : `#${selected}`);
  zoom = null;
  for (const button of document.querySelectorAll<HTMLButtonElement>("#ranges button")) {
    button.setAttribute("aria-pressed", String(button.dataset["range"] === selected));
  }
  for (const row of rows.values()) row.points = [];
  void backfill(parseMicros(nowMicros() - RANGES[range].spanUs)).catch(report);
}

for (const button of document.querySelectorAll<HTMLButtonElement>("#ranges button")) {
  button.addEventListener("click", () => {
    const selected = Object.keys(RANGES).find((r) => r === button.dataset["range"]);
    if (selected === "hour" || selected === "day" || selected === "month") selectRange(selected);
  });
}

// A range can be bookmarked: `#day` or `#month`.
const bookmarked = Object.keys(RANGES).find((r) => `#${r}` === location.hash);
if (bookmarked === "day" || bookmarked === "month") {
  range = bookmarked;
  for (const button of document.querySelectorAll<HTMLButtonElement>("#ranges button")) {
    button.setAttribute("aria-pressed", String(button.dataset["range"] === bookmarked));
  }
}

setInterval(() => {
  trim();
  // Rollups close once a minute; re-read them so the long ranges stay current.
  if (range !== "hour") void backfill(parseMicros(nowMicros() - RANGES[range].spanUs)).catch(report);
}, 60_000);
connect();
