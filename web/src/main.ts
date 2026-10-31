// The app shell: backfill, the live stream, and the layered stack of rows. All the
// decisions live in pure modules (series, status, roles); this file only wires them
// to the DOM and the network.

import "uplot/dist/uPlot.min.css";
import "./style.css";

import { ParseError, parseLive, parseMarkers, parseMicros, parsePoints, parseRoles } from "./api/parse";
import { createChart, type Chart } from "./chart";
import type { Marker, Micros, Point, RoleKey } from "./model";
import { compareRoles, label } from "./roles";
import { latest, merge, since } from "./series";
import { status } from "./status";

const WINDOW_US = 3600 * 1e6;
const STALE_AFTER_US = 15 * 1e6;

interface Row {
  points: Point[];
  chart: Chart;
  statusEl: HTMLElement;
  valueEl: HTMLElement;
}

const rows = new Map<RoleKey, Row>();
let markers: Marker[] = [];
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
  head.append(element("h2", "row-title", label(key)), statusEl, valueEl);
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
  const chart = createChart(plotEl, () => series.points, () => markers, setZoom);
  const row: Row = {
    get points() {
      return series.points;
    },
    set points(points: Point[]) {
      series.points = points;
    },
    statusEl,
    valueEl,
    chart,
  };
  rows.set(key, row);
  return row;
}

function setZoom(min: number, max: number): void {
  zoom = { min, max };
  for (const row of rows.values()) row.chart.setWindow(min, max);
  const reset = document.getElementById("reset-zoom");
  if (reset !== null) reset.hidden = false;
}

function render(row: Row): void {
  row.chart.update(row.points);
  if (zoom !== null) row.chart.setWindow(zoom.min, zoom.max);
  const s = status(row.points, nowMicros(), STALE_AFTER_US);
  row.statusEl.className = `status ${s}`;
  row.statusEl.textContent = s;
  const last = row.points.at(-1);
  row.valueEl.textContent =
    last === undefined ? "" : last.measurement.kind === "round-trip" ? `${last.measurement.rttMs.toFixed(2)} ms` : last.measurement.reason;
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
  el.replaceChildren(
    ...[...open.entries()]
      .filter(([role]) => !role.startsWith("target"))
      .map(([role, m]) => element("span", "identity", `${role}: ${m.detail ?? "?"} (epoch ${String(m.epoch)})`)),
  );
}

async function backfill(from: Micros): Promise<void> {
  const [roles, events] = await Promise.all([
    fetchJson("/api/roles").then(parseRoles),
    fetchJson(`/api/events?since_us=${String(from)}`).then(parseMarkers),
  ]);
  markers = markers.concat(events.filter((e) => !markers.some((m) => m.at === e.at && m.kind === e.kind && m.epoch === e.epoch)));
  await Promise.all(
    roles.map(async (key) => {
      const row = addRow(key);
      const fetched = parsePoints(await fetchJson(`/api/samples?role=${encodeURIComponent(key)}&since_us=${String(latest(row.points) ?? from)}`));
      row.points = merge(row.points, fetched);
      render(row);
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
    const from = earliestLatest.length > 0 ? Math.min(...earliestLatest) : nowMicros() - WINDOW_US;
    void backfill(parseMicros(from)).catch(report);
  };
  socket.onmessage = (event: MessageEvent) => {
    try {
      const live = parseLive(JSON.parse(String(event.data)));
      switch (live.type) {
        case "sample": {
          const row = addRow(live.role);
          row.points = merge(row.points, [live.point]);
          render(row);
          break;
        }
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
  const cutoff = nowMicros() - WINDOW_US;
  for (const row of rows.values()) {
    row.points = since(row.points, parseMicros(cutoff));
    render(row);
  }
  markers = markers.filter((m) => m.at >= cutoff);
}

document.getElementById("reset-zoom")?.addEventListener("click", (event) => {
  zoom = null;
  if (event.currentTarget instanceof HTMLElement) event.currentTarget.hidden = true;
  for (const row of rows.values()) render(row);
});
setInterval(trim, 60_000);
connect();
