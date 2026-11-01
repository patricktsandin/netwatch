// The app shell: backfill, the live stream, and the layered stack of rows. All the
// measurement decisions, statuses included, are the daemon's (ADR-0030); the pure
// modules here (series, roles) only merge and present, and this file only wires them
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
  parseStatuses,
  parseBindings,
  parseGauges,
  parseIncidents,
  parseReadingRollups,
  parseReadings,
  parseTargetNames,
} from "./api/parse";
import { createChart, type Chart } from "./chart";
import { formatValue, gaugeColumns, mergeReadings } from "./gauges";
import type {
  Binding,
  GaugeInfo,
  GaugeKey,
  GaugeUnit,
  Incident,
  LayerReport,
  Marker,
  Micros,
  Point,
  Reading,
  RoleKey,
} from "./model";
import { compareRoles, label } from "./roles";
import { latest, merge, plotColumns, since } from "./series";
import { tooltip, upsertIncident, visibleIncidents, type Words } from "./tooltip";
import type { Status } from "./model";

/** The time ranges the UI offers: raw samples for the last hour, then the rollup
 *  tiers (ADR-0006). */
type Range = "hour" | "day" | "month";
interface RangeSpec {
  readonly spanUs: number;
  readonly tier: "minute" | "hour" | null;
  readonly maxGapUs: number;
}
const RANGES: Readonly<Record<Range, RangeSpec>> = {
  hour: { spanUs: 3600e6, tier: null, maxGapUs: 15e6 },
  day: { spanUs: 24 * 3600e6, tier: "minute", maxGapUs: 2 * 60e6 },
  month: { spanUs: 30 * 24 * 3600e6, tier: "hour", maxGapUs: 2 * 3600e6 },
};
let range: Range = "hour";

interface Row {
  points: Point[];
  readings: Reading[];
  chart: Chart;
  plotEl: HTMLElement;
  /** One chart per unit among the row's gauges, created when readings first arrive. */
  gaugeCharts: Map<GaugeUnit, Chart>;
  rowEl: HTMLElement;
  titleEl: HTMLElement;
  statusEl: HTMLElement;
  valueEl: HTMLElement;
}

const rows = new Map<RoleKey, Row>();
let markers: Marker[] = [];
let incidents: Incident[] = [];
const layers = new Map<RoleKey, LayerReport>();
let names: ReadonlyMap<string, string> = new Map();
let statuses: ReadonlyMap<RoleKey, Status> = new Map();
let bindings: ReadonlyMap<RoleKey, Binding> = new Map();
let zoom: { min: number; max: number } | null = null;

const nowMicros = (): Micros => parseMicros(Date.now() * 1000);

let gauges: ReadonlyMap<GaugeKey, GaugeInfo> = new Map();

const words: Words = {
  time: (at) => new Date(at / 1000).toLocaleTimeString(),
  role: (key) => label(key, names),
  reading: (r) => {
    const info = gauges.get(r.gauge);
    return info === undefined ? String(r.value) : `${info.label} ${formatValue(info.unit, r.value)}`;
  },
};

/** Line colors, in gauge order. */
const LINE_COLORS = ["--line", "--line-2", "--line-3"] as const;

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

  // The chart reads the row's data while it is being constructed, so the data needs
  // a home that exists before the chart does.
  const series: { points: Point[]; readings: Reading[] } = { points: [], readings: [] };
  const chart = createChart(plotEl, {
    ...common(key, series),
    columns: () => {
      const { xs, ys } = plotColumns(series.points, RANGES[range].maxGapUs);
      return { xs, ys: [ys] };
    },
    lines: [{ label: "RTT", color: "--line" }],
    format: (v) => `${String(v)} ms`,
    losses: () => series.points,
  });
  const row: Row = {
    get points() {
      return series.points;
    },
    set points(points: Point[]) {
      series.points = points;
    },
    get readings() {
      return series.readings;
    },
    set readings(readings: Reading[]) {
      series.readings = readings;
    },
    gaugeCharts: new Map(),
    rowEl,
    statusEl,
    valueEl,
    titleEl,
    plotEl,
    chart,
  };
  rows.set(key, row);
  return row;
}

/** What every chart of a row shares: its incidents, markers, tooltip, and zoom. */
function common(key: RoleKey, series: { points: Point[]; readings: Reading[] }) {
  return {
    annotations: () => {
      const window = zoom ?? sharedWindow();
      return {
        incidents: visibleIncidents(incidents, key, parseMicros(window.min * 1e6), parseMicros(window.max * 1e6)),
        markers: markers.filter((m) => m.concerns.includes(key)),
      };
    },
    tooltip: (at: Micros, nearUs: number) =>
      tooltip(
        { at, nearUs, now: nowMicros() },
        { role: key, points: series.points, readings: series.readings, incidents, markers },
        words,
      ),
    onZoom: setZoom,
    now: () => nowMicros(),
  };
}

/** Creates a gauge chart for each unit among the row's readings that has none yet. */
function addGaugeCharts(row: Row, key: RoleKey): void {
  const present = new Set(row.readings.map((r) => r.gauge));
  const byUnit = new Map<GaugeUnit, GaugeInfo[]>();
  for (const info of gauges.values()) byUnit.set(info.unit, [...(byUnit.get(info.unit) ?? []), info]);
  for (const [unit, ofUnit] of byUnit) {
    if (row.gaugeCharts.has(unit) || !ofUnit.some((g) => present.has(g.key))) continue;
    const el = element("div", "plot");
    row.rowEl.append(el);
    const chart = createChart(el, {
      ...common(key, row),
      columns: () =>
        gaugeColumns(
          row.readings,
          ofUnit.map((g) => g.key),
          RANGES[range].maxGapUs,
        ),
      lines: ofUnit.map((g, i) => ({ label: g.label, color: LINE_COLORS[i % LINE_COLORS.length] ?? "--line" })),
      format: (v) => formatValue(unit, v),
      losses: () => [],
    });
    row.gaugeCharts.set(unit, chart);
  }
}

/** The selected range, ending now. */
function sharedWindow(): { min: number; max: number } {
  const now = nowMicros() / 1e6;
  return { min: now - RANGES[range].spanUs / 1e6, max: now };
}

function setZoom(min: number, max: number): void {
  zoom = { min, max };
  for (const row of rows.values()) for (const chart of charts(row)) chart.setWindow(min, max);
  const reset = document.getElementById("reset-zoom");
  if (reset !== null) reset.hidden = false;
}

function layerSummary(key: RoleKey): string {
  const report = layers.get(key);
  if (report === undefined) return "";
  const cpu = report.cpuPressure === null ? "" : `CPU pressure ${report.cpuPressure.toFixed(1)}%`;
  const conntrack = report.conntrackFill === null ? "" : `conntrack ${report.conntrackFill.toFixed(1)}% full`;
  const clock = report.clockSynchronized === false ? "clock not synchronized" : "";
  const grew = report.increases.map((i) => `${i.name} +${String(i.by)}`);
  const disabled = report.disabled.map((d) => `disabled: ${d}`);
  return [cpu, conntrack, clock, ...grew, ...disabled].filter((s) => s !== "").join(" · ");
}

function charts(row: Row): Chart[] {
  return [row.chart, ...row.gaugeCharts.values()];
}

function render(row: Row, key: RoleKey): void {
  addGaugeCharts(row, key);
  // Every row shares one time axis (ADR-0005): the zoom if there is one, otherwise
  // the selected range up to now.
  const window = zoom ?? sharedWindow();
  for (const chart of charts(row)) {
    chart.update();
    chart.setWindow(window.min, window.max);
  }
  row.plotEl.hidden = row.points.length === 0;
  const s: Status = statuses.get(key) ?? "unknown";
  row.statusEl.className = `status ${s}`;
  row.statusEl.textContent = s === "rate-limited" ? "rate-limited?" : s;
  row.statusEl.title =
    s === "rate-limited" ? "Loss here doesn't carry through to later hops: the router is deprioritizing ICMP." : "";
  const last = row.points.at(-1);
  const lastReading = row.readings.at(-1);
  const latestValue =
    last !== undefined
      ? last.measurement.kind === "round-trip"
        ? `${last.measurement.rttMs.toFixed(2)} ms`
        : last.measurement.reason
      : lastReading !== undefined
        ? row.readings
            .filter((r) => r.at === lastReading.at)
            .map((r) => words.reading(r))
            .join(" · ")
        : "";
  const extra = layerSummary(key);
  row.valueEl.textContent = [extra, latestValue].filter((s) => s !== "").join(" · ");
}

/** Shows Topology's current bindings: global roles in the header, and each hop's
 *  router in its row's title. Nothing here is derived; the daemon says what holds. */
function renderIdentities(): void {
  const el = document.getElementById("identities");
  if (el === null) return;
  el.replaceChildren(
    ...[...bindings.entries()]
      .filter(([role]) => !role.startsWith("target") && !role.startsWith("hop"))
      .map(([role, b]) => element("span", "identity", `${role}: ${b.identity} (epoch ${String(b.epoch)})`)),
  );
  for (const [key, row] of rows) {
    const binding = bindings.get(key);
    row.titleEl.textContent =
      key.startsWith("hop") && binding !== undefined ? `${label(key, names)} · ${binding.identity}` : label(key, names);
  }
}

async function backfill(from: Micros): Promise<void> {
  const [roles, events, recorded] = await Promise.all([
    fetchJson("/api/roles").then(parseRoles),
    fetchJson(`/api/events?since_us=${String(from)}`).then(parseMarkers),
    fetchJson(`/api/incidents?since_us=${String(from)}`).then(parseIncidents),
  ]);
  markers = markers.concat(
    events.filter((e) => !markers.some((m) => m.at === e.at && m.kind === e.kind && m.epoch === e.epoch)),
  );
  incidents = recorded.reduce(upsertIncident, incidents);
  for (const [key, report] of parseLayers(await fetchJson("/api/layers"))) {
    layers.set(key, report);
    render(addRow(key), key);
  }
  names = parseTargetNames(await fetchJson("/api/targets"));
  gauges = parseGauges(await fetchJson("/api/gauges"));
  statuses = parseStatuses(await fetchJson("/api/status"));
  bindings = parseBindings(await fetchJson("/api/bindings"));
  await Promise.all(
    roles.map(async (key) => {
      const row = addRow(key);
      const tier = RANGES[range].tier;
      if (tier === null) {
        const fetched = parsePoints(
          await fetchJson(
            `/api/samples?role=${encodeURIComponent(key)}&since_us=${String(latest(row.points) ?? from)}`,
          ),
        );
        row.points = merge(row.points, fetched);
        const readingsSince = row.readings.at(-1)?.at ?? from;
        const readings = parseReadings(
          await fetchJson(`/api/readings?role=${encodeURIComponent(key)}&since_us=${String(readingsSince)}`),
        );
        row.readings = mergeReadings(row.readings, readings);
      } else {
        // Rollups are re-read whole: the latest interval keeps growing until it closes.
        row.points = [
          ...parseRollups(
            await fetchJson(`/api/rollups?role=${encodeURIComponent(key)}&tier=${tier}&since_us=${String(from)}`),
          ),
        ];
        row.readings = [
          ...parseReadingRollups(
            await fetchJson(
              `/api/reading-rollups?role=${encodeURIComponent(key)}&tier=${tier}&since_us=${String(from)}`,
            ),
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
        case "statuses":
          statuses = live.statuses;
          for (const [key, row] of rows) render(row, key);
          break;
        case "bindings":
          bindings = live.bindings;
          renderIdentities();
          break;
        case "event":
          markers.push(live.marker);
          renderIdentities();
          for (const key of live.marker.concerns) redraw(key);
          break;
        case "incident":
          incidents = upsertIncident(incidents, live.incident);
          redraw(live.incident.role);
          break;
        case "reading": {
          // Live readings are raw, so they only join the raw range.
          if (range !== "hour") break;
          const row = addRow(live.role);
          row.readings = mergeReadings(row.readings, [live.reading]);
          render(row, live.role);
          break;
        }
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

function redraw(key: RoleKey): void {
  const row = rows.get(key);
  if (row !== undefined) for (const chart of charts(row)) chart.redraw();
}

function report(error: unknown): void {
  const kind = error instanceof ParseError ? "unexpected data from the daemon" : "error";
  console.error(kind, error);
}

function trim(): void {
  const cutoff = nowMicros() - RANGES[range].spanUs;
  for (const [key, row] of rows) {
    row.points = since(row.points, parseMicros(cutoff));
    row.readings = row.readings.filter((r) => r.at >= cutoff);
    render(row, key);
  }
  markers = markers.filter((m) => m.at >= cutoff);
  incidents = incidents.filter((i) => i.ended === null || i.ended >= cutoff);
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
  for (const row of rows.values()) {
    row.points = [];
    row.readings = [];
  }
  incidents = [];
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
