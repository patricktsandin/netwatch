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
  parseMap,
  parseMarkers,
  parseMicros,
  parsePoints,
  parseRoles,
  parseRollups,
  parseStatuses,
  parseBindings,
  parseGauges,
  parsePublic,
  parseIncidents,
  parseReadingRollups,
  parseReadings,
  parseSharedRows,
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
  NetworkMap,
  Point,
  Reading,
  RoleKey,
  SharedRow,
} from "./model";
import { renderMap } from "./mapView";
import { compareRoles, label, rowLabel } from "./roles";
import { membersOf, rowOf } from "./rows";
import { latest, merge, plotColumns, since } from "./series";
import { openSettings } from "./settingsDialog";
import { tooltip, upsertIncident, visibleIncidents, visibleMarkers, type Words } from "./tooltip";
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
/** The rows several hop roles share, and their statuses (ADR-0045). */
let shared: readonly SharedRow[] = [];
let rowStatuses: ReadonlyMap<RoleKey, Status> = new Map();
let bindings: ReadonlyMap<RoleKey, Binding> = new Map();
let zoom: { min: number; max: number } | null = null;
/** This host's public address and where it enters Cloudflare (ADR-0039). */
let publicIdentity: string | null = null;
/** The network map, as the daemon last sent it (ADR-0042). */
let networkMap: NetworkMap = { nodes: [], edges: [] };
type Tab = "graphs" | "map";
let tab: Tab = "graphs";

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
  const titleEl = element("h2", "row-title", rowLabel(membersOf(shared, key), names));
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
      const members = membersOf(shared, key);
      return {
        incidents: visibleIncidents(incidents, members, parseMicros(window.min * 1e6), parseMicros(window.max * 1e6)),
        markers: visibleMarkers(markers, members),
      };
    },
    tooltip: (at: Micros, nearUs: number) =>
      tooltip(
        { at, nearUs, now: nowMicros() },
        { roles: membersOf(shared, key), points: series.points, readings: series.readings, incidents, markers },
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
  return [...report.notes, cpu, conntrack, clock, ...grew, ...disabled].filter((s) => s !== "").join(" · ");
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
  const s: Status = rowStatuses.get(key) ?? statuses.get(key) ?? "unknown";
  row.statusEl.className = `status ${s}`;
  row.statusEl.textContent = s === "rate-limited" ? "rate-limited?" : s;
  // A shared row's members are judged along their own paths, so they can differ.
  const own = membersOf(shared, key).map((m) => [m, statuses.get(m) ?? "unknown"] as const);
  const differ = new Set(own.map(([, status]) => status)).size > 1;
  row.statusEl.title = differ
    ? own.map(([m, status]) => `${label(m, names)}: ${status}`).join("\n")
    : s === "rate-limited"
      ? "Loss here doesn't carry through to later hops: the router is deprioritizing ICMP."
      : "";
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
    ...(publicIdentity === null ? [] : [element("span", "identity public", `public: ${publicIdentity}`)]),
    ...[...bindings.entries()]
      .filter(([role]) => !role.startsWith("target") && !role.startsWith("hop"))
      .map(([role, b]) => element("span", "identity", `${role}: ${b.identity} (epoch ${String(b.epoch)})`)),
  );
  for (const [key, row] of rows) {
    const binding = bindings.get(key);
    const router = binding === undefined ? "" : [binding.identity, binding.name].filter((s) => s !== null).join(" · ");
    const title = rowLabel(membersOf(shared, key), names);
    row.titleEl.textContent = key.startsWith("hop") && router !== "" ? `${title} · ${router}` : title;
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
  shared = parseSharedRows(await fetchJson("/api/rows"));
  rowStatuses = new Map(shared.map((r) => [r.key, r.status]));
  // A row made for a role before its shared row was known goes.
  for (const [key, row] of rows) {
    if (rowOf(shared, key) !== key) {
      row.rowEl.remove();
      rows.delete(key);
    }
  }
  gauges = parseGauges(await fetchJson("/api/gauges"));
  statuses = parseStatuses(await fetchJson("/api/status"));
  bindings = parseBindings(await fetchJson("/api/bindings"));
  publicIdentity = parsePublic(await fetchJson("/api/host"));
  // The map is a view of its own: a payload it can't parse mustn't stop the graphs.
  try {
    networkMap = parseMap(await fetchJson("/api/map"));
    drawMap();
  } catch (error) {
    report(error);
  }
  await Promise.all(
    // A shared row draws its owner's samples; its other members' are copies.
    roles
      .filter((key) => rowOf(shared, key) === key)
      .map(async (key) => {
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
          if (range !== "hour" || rowOf(shared, live.role) !== live.role) break;
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
          rowStatuses = live.rows;
          for (const [key, row] of rows) render(row, key);
          break;
        case "bindings":
          bindings = live.bindings;
          renderIdentities();
          break;
        case "map":
          networkMap = live.map;
          drawMap();
          break;
        case "event":
          markers.push(live.marker);
          if (live.marker.kind === "public-address") {
            void fetchJson("/api/host")
              .then((host) => {
                publicIdentity = parsePublic(host);
                renderIdentities();
              })
              .catch(report);
          }
          renderIdentities();
          // Members of a shared row are each concerned, but their row is drawn once.
          for (const key of new Set(live.marker.concerns.map((c) => rowOf(shared, c)))) redraw(key);
          break;
        case "incident":
          incidents = upsertIncident(incidents, live.incident);
          redraw(rowOf(shared, live.incident.role));
          break;
        case "cleared":
          // All history was flushed (ADR-0038): start over from the empty store.
          markers = [];
          incidents = [];
          rebuild();
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

/** Drops every row and backfills the selected range: after a flush, or when the
 *  theme changes, since charts take their colors when they're created. */
function rebuild(): void {
  for (const row of rows.values()) row.rowEl.remove();
  rows.clear();
  void backfill(parseMicros(nowMicros() - RANGES[range].spanUs)).catch(report);
}

// The theme: Auto follows the system; Light and Dark are remembered per browser.
// index.html applies the saved choice before the first paint.
const THEME_KEY = "netwatch-theme";
const themeSelect = document.getElementById("theme");
if (themeSelect instanceof HTMLSelectElement) {
  themeSelect.value = document.documentElement.dataset["theme"] ?? "auto";
  themeSelect.addEventListener("change", () => {
    const theme = themeSelect.value;
    if (theme === "light" || theme === "dark") document.documentElement.dataset["theme"] = theme;
    else delete document.documentElement.dataset["theme"];
    try {
      localStorage.setItem(THEME_KEY, theme);
    } catch {
      // Storage may be unavailable; the choice then lasts until the page closes.
    }
    rebuild();
  });
}

/** Draws the network map, if it's showing. Switching to it draws it then. */
function drawMap(): void {
  const el = document.getElementById("map-canvas");
  if (el === null || tab !== "map") return;
  renderMap(el, networkMap, { role: (key) => label(key, names), time: (at) => words.time(at) }, (role) => {
    selectTab("graphs");
    const row = rows.get(rowOf(shared, role))?.rowEl;
    if (row === undefined) return;
    row.scrollIntoView({ block: "center" });
    row.classList.add("flash");
    setTimeout(() => {
      row.classList.remove("flash");
    }, 1500);
  });
}

// The tabs: the graphs, or the network map (ADR-0042), which `#map` bookmarks.
/** Sets the address's hash, keeping its path and query. */
function setHash(hash: string): void {
  history.replaceState(null, "", `${location.pathname}${location.search}${hash}`);
}

function selectTab(selected: Tab): void {
  tab = selected;
  for (const button of document.querySelectorAll<HTMLButtonElement>("#tabs button")) {
    button.setAttribute("aria-selected", String(button.dataset["tab"] === selected));
  }
  const [stack, mapView, ranges] = ["stack", "map-view", "ranges"].map((id) => document.getElementById(id));
  if (stack) stack.hidden = selected !== "graphs";
  if (mapView) mapView.hidden = selected !== "map";
  if (ranges) ranges.hidden = selected !== "graphs";
  const reset = document.getElementById("reset-zoom");
  if (reset !== null && selected === "map") reset.hidden = true;
  if (reset !== null && selected === "graphs") reset.hidden = zoom === null;
  setHash(selected === "map" ? "#map" : range === "hour" ? "" : `#${range}`);
  drawMap();
  if (selected === "graphs") for (const [key, row] of rows) render(row, key);
}
for (const button of document.querySelectorAll<HTMLButtonElement>("#tabs button")) {
  button.addEventListener("click", () => {
    const selected = button.dataset["tab"];
    if (selected === "graphs" || selected === "map") selectTab(selected);
  });
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
  setHash(selected === "hour" ? "" : `#${selected}`);
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

document.getElementById("open-settings")?.addEventListener("click", () => {
  // After a save, the daemon restarts: the live stream drops and reconnects, and the
  // backfill brings the new targets' rows.
  void openSettings(() => {
    for (const row of rows.values()) row.rowEl.remove();
    rows.clear();
  }).catch(report);
});

// The incident report (ADR-0033) is the daemon's page; the dialog only picks the span,
// starting from what's on screen, and passes the ticket details along (ADR-0040). The
// details that stay the same from ticket to ticket are remembered in this browser.
const TICKET_FIELDS = ["account", "service_address", "contact", "ticket", "description"] as const;
const REMEMBERED: readonly string[] = ["account", "service_address", "contact"];
const TICKET_KEY = "netwatch-ticket";
function ticketInput(field: string): HTMLInputElement | HTMLTextAreaElement | null {
  const el = document.getElementById(`report-${field}`);
  return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement ? el : null;
}
function restoreTicket(): void {
  let saved: unknown = null;
  try {
    saved = JSON.parse(localStorage.getItem(TICKET_KEY) ?? "null");
  } catch {
    // Storage may be unavailable or hold something else; the fields just start empty.
  }
  if (typeof saved !== "object" || saved === null) return;
  for (const [field, value] of Object.entries(saved)) {
    const input = ticketInput(field);
    if (input !== null && REMEMBERED.includes(field) && typeof value === "string") input.value = value;
  }
}
function saveTicket(): void {
  const kept = Object.fromEntries(REMEMBERED.map((f) => [f, ticketInput(f)?.value ?? ""]));
  try {
    localStorage.setItem(TICKET_KEY, JSON.stringify(kept));
  } catch {
    // Not remembering is fine.
  }
}
function localInput(seconds: number): string {
  const d = new Date(seconds * 1000);
  const pad = (n: number): string => String(n).padStart(2, "0");
  return `${String(d.getFullYear())}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}
const reportDialog = document.getElementById("report-dialog");
document.getElementById("open-report")?.addEventListener("click", () => {
  if (!(reportDialog instanceof HTMLDialogElement)) return;
  const window = zoom ?? sharedWindow();
  const from = document.getElementById("report-from");
  const until = document.getElementById("report-until");
  if (from instanceof HTMLInputElement) from.value = localInput(window.min);
  if (until instanceof HTMLInputElement) until.value = localInput(window.max);
  restoreTicket();
  reportDialog.showModal();
});
reportDialog?.addEventListener("close", () => {
  if (!(reportDialog instanceof HTMLDialogElement)) return;
  const from = document.getElementById("report-from");
  const until = document.getElementById("report-until");
  if (!(from instanceof HTMLInputElement) || !(until instanceof HTMLInputElement)) return;
  const path = { page: "/report", text: "/report.txt" }[reportDialog.returnValue];
  if (path === undefined) return;
  const micros = (input: HTMLInputElement): number => new Date(input.value).getTime() * 1000;
  // The viewer's offset, so the report can show local times beside UTC.
  const offset = -new Date().getTimezoneOffset();
  const query = new URLSearchParams({
    from_us: String(micros(from)),
    until_us: String(micros(until)),
    offset_min: String(offset),
  });
  for (const field of TICKET_FIELDS) {
    const value = ticketInput(field)?.value.trim() ?? "";
    if (value !== "") query.set(field, value);
  }
  saveTicket();
  open(`${path}?${query.toString()}`, "_blank");
});

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

if (location.hash === "#map") selectTab("map");
// Back and Forward, and edited addresses: `#map`, or the graphs at `#day`, `#month`,
// or the hour.
addEventListener("hashchange", () => {
  if (location.hash === "#map") {
    if (tab !== "map") selectTab("map");
    return;
  }
  if (tab !== "graphs") selectTab("graphs");
  const wanted: Range = location.hash === "#day" ? "day" : location.hash === "#month" ? "month" : "hour";
  if (wanted !== range) selectRange(wanted);
});

setInterval(() => {
  trim();
  // Rollups close once a minute; re-read them so the long ranges stay current.
  if (range !== "hour") void backfill(parseMicros(nowMicros() - RANGES[range].spanUs)).catch(report);
}, 60_000);
connect();
