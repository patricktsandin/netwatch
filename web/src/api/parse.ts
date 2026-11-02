// The boundary parser: the only place where untyped JSON becomes domain types, and
// the only file allowed to use `as` (ADR-0019). Anything malformed is rejected, never
// coerced.

import type { ConfigView } from "./generated/ConfigView";
import type { EventPoint } from "./generated/EventPoint";
import type { GaugeView } from "./generated/GaugeView";
import type { IncidentView } from "./generated/IncidentView";
import type { ReadingPoint } from "./generated/ReadingPoint";
import type { ReadingRollupPoint } from "./generated/ReadingRollupPoint";
import type { LayerReportView } from "./generated/LayerReportView";
import type { MapEdgeView } from "./generated/MapEdgeView";
import type { MapNodeView } from "./generated/MapNodeView";
import type { RowView } from "./generated/RowView";
import type { SightingView } from "./generated/SightingView";
import type { RoleSummary } from "./generated/RoleSummary";
import type { SamplePoint } from "./generated/SamplePoint";
import type {
  Binding,
  Config,
  TargetKind,
  TargetSpec,
  Epoch,
  EventKind,
  GaugeInfo,
  GaugeKey,
  GaugeUnit,
  Incident,
  LayerReport,
  Live,
  LossReason,
  MapEdge,
  MapNode,
  MapNodeId,
  Marker,
  NetworkMap,
  Micros,
  Point,
  Reading,
  RoleKey,
  Severity,
  SharedRow,
  Status,
} from "../model";

export class ParseError extends Error {
  override readonly name = "ParseError";
}

type Fields<T> = { readonly [K in keyof T]: unknown };

function record<T>(value: unknown, what: string): Fields<T> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ParseError(`${what}: expected an object`);
  }
  return value as Fields<T>;
}

function array(value: unknown, what: string): readonly unknown[] {
  if (!Array.isArray(value)) throw new ParseError(`${what}: expected an array`);
  return value;
}

function integer(value: unknown, what: string): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value)) {
    throw new ParseError(`${what}: expected an integer`);
  }
  return value;
}

function optionalBoolean(value: unknown, what: string): boolean | null {
  if (value === null || value === undefined) return null;
  if (typeof value !== "boolean") throw new ParseError(`${what}: expected a boolean`);
  return value;
}

function optionalNumber(value: unknown, what: string): number | null {
  if (value === null || value === undefined) return null;
  if (typeof value !== "number" || !Number.isFinite(value)) throw new ParseError(`${what}: expected a number`);
  return value;
}

function nullableString(value: unknown, what: string): string | null {
  if (value === null || typeof value === "string") return value;
  throw new ParseError(`${what}: expected a string or null`);
}

const ROLE_KEY = /^(kernel|nic|wifi|cellular|tunnel|local-interface|target:(0|[1-9]\d*)|hop:(0|[1-9]\d*):([1-9]\d*))$/;

function roleKey(value: unknown, what: string): RoleKey {
  if (typeof value !== "string" || !ROLE_KEY.test(value)) {
    throw new ParseError(`${what}: expected a role key`);
  }
  return value as RoleKey;
}

const micros = (value: unknown, what: string): Micros => integer(value, what) as Micros;

/** A time in µs since the Unix epoch, for example `Date.now() * 1000`. */
export const parseMicros = (value: unknown): Micros => micros(Math.round(Number(value)), "time");

function epoch(value: unknown, what: string): Epoch {
  const n = integer(value, what);
  if (n < 1) throw new ParseError(`${what}: epochs start at 1`);
  return n as Epoch;
}

const LOSS_REASONS: readonly LossReason[] = [
  "timed-out",
  "unreachable",
  "refused",
  "unexpected-response",
  "send-failed",
];
const EVENT_KINDS: readonly EventKind[] = [
  "epoch-started",
  "epoch-resumed",
  "started",
  "epoch-ended",
  "desynchronized",
  "resynchronized",
  "kernel-log",
  "source-disabled",
  "clock-jump",
  "config-drift",
  "radio",
  "public-address",
  "trace",
];

function oneOf<T extends string>(value: unknown, allowed: readonly T[], what: string): T {
  const found = allowed.find((a) => a === value);
  if (found === undefined) throw new ParseError(`${what}: unexpected ${JSON.stringify(value)}`);
  return found;
}

export function parsePoint(value: unknown): Point {
  const p = record<SamplePoint>(value, "sample");
  const at = micros(p.at_us, "sample.at_us");
  const sampleEpoch = epoch(p.epoch, "sample.epoch");
  // Exactly one of rtt_us and loss is set (ADR-0004).
  if (p.rtt_us !== null && p.loss === null) {
    const rttUs = integer(p.rtt_us, "sample.rtt_us");
    if (rttUs < 0) throw new ParseError("sample.rtt_us: negative");
    return { at, epoch: sampleEpoch, measurement: { kind: "round-trip", rttMs: rttUs / 1000 } };
  }
  if (p.rtt_us === null && p.loss !== null) {
    const reason = oneOf(p.loss, LOSS_REASONS, "sample.loss");
    return { at, epoch: sampleEpoch, measurement: { kind: "lost", reason } };
  }
  throw new ParseError("sample: exactly one of rtt_us and loss must be set");
}

export function parseMarker(value: unknown): Marker {
  const e = record<EventPoint>(value, "event");
  return {
    at: micros(e.at_us, "event.at_us"),
    kind: oneOf(e.kind, EVENT_KINDS, "event.kind"),
    role: e.role === null ? null : roleKey(e.role, "event.role"),
    epoch: e.epoch === null ? null : epoch(e.epoch, "event.epoch"),
    detail: nullableString(e.detail, "event.detail"),
    concerns: array(e.roles, "event.roles").map((r) => roleKey(r, "event.roles")),
  };
}

const GAUGE_KEY = /^[a-z][a-z0-9-]*$/;

function gaugeKey(value: unknown, what: string): GaugeKey {
  if (typeof value !== "string" || !GAUGE_KEY.test(value)) throw new ParseError(`${what}: expected a gauge key`);
  return value as GaugeKey;
}

function finite(value: unknown, what: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new ParseError(`${what}: expected a number`);
  return value;
}

const GAUGE_UNITS: readonly GaugeUnit[] = ["bit/s", "dBm", "dB", "%"];

/** Every gauge's label and unit, from `/api/gauges`. */
export function parseGauges(value: unknown): ReadonlyMap<GaugeKey, GaugeInfo> {
  return new Map(
    array(value, "gauges").map((g) => {
      const v = record<GaugeView>(g, "gauge");
      if (typeof v.label !== "string") throw new ParseError("gauge.label: expected a string");
      const key = gaugeKey(v.key, "gauge.key");
      return [key, { key, label: v.label, unit: oneOf(v.unit, GAUGE_UNITS, "gauge.unit") }];
    }),
  );
}

export function parseReading(value: unknown): Reading {
  const r = record<ReadingPoint>(value, "reading");
  return {
    at: micros(r.at_us, "reading.at_us"),
    epoch: epoch(r.epoch, "reading.epoch"),
    gauge: gaugeKey(r.gauge, "reading.gauge"),
    value: finite(r.value, "reading.value"),
  };
}

export function parseReadings(value: unknown): readonly Reading[] {
  return array(value, "readings").map(parseReading);
}

/** Reading rollups from `/api/reading-rollups`, as readings at each interval's mean. */
export function parseReadingRollups(value: unknown): readonly Reading[] {
  return array(value, "reading rollups").map((v) => {
    const r = record<ReadingRollupPoint>(v, "reading rollup");
    return {
      at: micros(r.start_us, "reading rollup.start_us"),
      epoch: epoch(r.epoch, "reading rollup.epoch"),
      gauge: gaugeKey(r.gauge, "reading rollup.gauge"),
      value: finite(r.mean, "reading rollup.mean"),
    };
  });
}

const SEVERITIES: readonly Severity[] = ["degradation", "outage"];

export function parseIncident(value: unknown): Incident {
  const i = record<IncidentView>(value, "incident");
  const started = micros(i.started_us, "incident.started_us");
  const ended = i.ended_us === null ? null : micros(i.ended_us, "incident.ended_us");
  if (ended !== null && ended < started) throw new ParseError("incident: ends before it starts");
  return {
    role: roleKey(i.role, "incident.role"),
    started,
    ended,
    severity: oneOf(i.severity, SEVERITIES, "incident.severity"),
    cause: i.cause === null ? null : roleKey(i.cause, "incident.cause"),
  };
}

export function parseIncidents(value: unknown): readonly Incident[] {
  return array(value, "incidents").map(parseIncident);
}

export function parseRoles(value: unknown): readonly RoleKey[] {
  return array(value, "roles").map((r) => roleKey(record<RoleSummary>(r, "role").key, "role.key"));
}

/**
 * Rollups from `/api/rollups`, as points the charts can draw: each interval is drawn
 * at its median RTT, or as a loss if nothing in it was answered.
 */
export function parseRollups(value: unknown): readonly Point[] {
  return array(value, "rollups").map((r) => {
    const rollup = record<{ start_us: number; epoch: number; count: number; lost: number; p50_us: number | null }>(
      r,
      "rollup",
    );
    const at = micros(rollup.start_us, "rollup.start_us");
    const rollupEpoch = epoch(rollup.epoch, "rollup.epoch");
    const count = integer(rollup.count, "rollup.count");
    const lost = integer(rollup.lost, "rollup.lost");
    if (lost > count) throw new ParseError("rollup: more lost than counted");
    if (rollup.p50_us === null) {
      return { at, epoch: rollupEpoch, measurement: { kind: "lost", reason: "timed-out" } };
    }
    const p50 = integer(rollup.p50_us, "rollup.p50_us");
    return { at, epoch: rollupEpoch, measurement: { kind: "round-trip", rttMs: p50 / 1000 } };
  });
}

const ROLE_STATUSES: readonly Status[] = ["healthy", "degraded", "failing", "unknown", "rate-limited"];

/** Every role's status, from `/api/status` or the live stream. */
export function parseStatuses(value: unknown): ReadonlyMap<RoleKey, Status> {
  const all = record<Record<string, unknown>>(value, "statuses");
  return new Map(
    Object.entries(all).map(([key, status]) => [roleKey(key, "statuses key"), oneOf(status, ROLE_STATUSES, "status")]),
  );
}

/** The rows several hop roles share (ADR-0045), from `/api/rows`. */
export function parseSharedRows(value: unknown): readonly SharedRow[] {
  return array(value, "rows").map((v) => {
    const r = record<RowView>(v, "row");
    const members = array(r.members, "row.members").map((m) => roleKey(m, "row.members"));
    const key = roleKey(r.key, "row.key");
    if (members[0] !== key) throw new ParseError("row.members: the owner comes first");
    return { key, members, status: oneOf(r.status, ROLE_STATUSES, "row.status") };
  });
}

/** Every role's binding, from `/api/bindings` or the live stream. */
export function parseBindings(value: unknown): ReadonlyMap<RoleKey, Binding> {
  const all = record<Record<string, unknown>>(value, "bindings");
  return new Map(
    Object.entries(all).map(([key, raw]) => {
      const b = record<{ identity: string; name: string | null; epoch: number }>(raw, "binding");
      if (typeof b.identity !== "string") throw new ParseError("binding.identity: expected a string");
      if (b.name !== null && typeof b.name !== "string") throw new ParseError("binding.name: expected a string");
      return [
        roleKey(key, "bindings key"),
        { identity: b.identity, name: b.name, epoch: epoch(b.epoch, "binding.epoch") },
      ];
    }),
  );
}

/** Every layer's latest report by role key, from `/api/layers`. */
export function parseLayers(value: unknown): ReadonlyMap<RoleKey, LayerReport> {
  const all = record<Record<string, unknown>>(value, "layers");
  return new Map(Object.entries(all).map(([key, report]) => [roleKey(key, "layers key"), parseLayerReport(report)]));
}

/** Target names by role key, from `/api/targets`. */
export function parseTargetNames(value: unknown): ReadonlyMap<string, string> {
  return new Map(
    array(value, "targets").map((t) => {
      const target = record<{ key: string; name: string }>(t, "target");
      const name = target.name;
      if (typeof name !== "string") throw new ParseError("target.name: expected a string");
      return [roleKey(target.key, "target.key"), name];
    }),
  );
}

export function parsePoints(value: unknown): readonly Point[] {
  return array(value, "samples").map(parsePoint);
}

export function parseMarkers(value: unknown): readonly Marker[] {
  return array(value, "events").map(parseMarker);
}

const STATUSES: readonly LayerReport["status"][] = ["healthy", "degraded", "failing", "unknown"];

export function parseLayerReport(value: unknown): LayerReport {
  const r = record<LayerReportView>(value, "layer");
  const pressure = r.pressure === null ? null : record<{ cpu: number }>(r.pressure, "layer.pressure");
  const cpu = pressure === null ? null : pressure.cpu;
  if (cpu !== null && (typeof cpu !== "number" || !Number.isFinite(cpu))) {
    throw new ParseError("layer.pressure.cpu: expected a number");
  }
  return {
    status: oneOf(r.status, STATUSES, "layer.status"),
    increases: array(r.increases, "layer.increases").map((i) => {
      const inc = record<{ name: string; by: number }>(i, "layer.increase");
      if (typeof inc.name !== "string") throw new ParseError("layer.increase.name: expected a string");
      return { name: inc.name, by: integer(inc.by, "layer.increase.by") };
    }),
    cpuPressure: cpu,
    conntrackFill: optionalNumber(r.conntrack_fill, "layer.conntrack_fill"),
    clockSynchronized: optionalBoolean(r.clock_synchronized, "layer.clock_synchronized"),
    disabled: array(r.disabled, "layer.disabled").map((d) => {
      if (typeof d !== "string") throw new ParseError("layer.disabled: expected strings");
      return d;
    }),
    notes: array(r.notes, "layer.notes").map((n) => {
      if (typeof n !== "string") throw new ParseError("layer.notes: expected strings");
      return n;
    }),
  };
}

export function parseLive(value: unknown): Live {
  const m = record<{
    type: unknown;
    role: unknown;
    point: unknown;
    event: unknown;
    incident: unknown;
    reading: unknown;
    report: unknown;
    statuses: unknown;
    rows: unknown;
    bindings: unknown;
    map: unknown;
  }>(value, "live");
  switch (m.type) {
    case "sample":
      return { type: "sample", role: roleKey(m.role, "live.role"), point: parsePoint(m.point) };
    case "event":
      return { type: "event", marker: parseMarker(m.event) };
    case "incident":
      return { type: "incident", incident: parseIncident(m.incident) };
    case "cleared":
      return { type: "cleared" };
    case "reading":
      return { type: "reading", role: roleKey(m.role, "live.role"), reading: parseReading(m.reading) };
    case "statuses":
      return { type: "statuses", statuses: parseStatuses(m.statuses), rows: parseStatuses(m.rows) };
    case "bindings":
      return { type: "bindings", bindings: parseBindings(m.bindings) };
    case "layer":
      return { type: "layer", role: roleKey(m.role, "live.role"), report: parseLayerReport(m.report) };
    case "map":
      return { type: "map", map: parseMap(m.map) };
    default:
      throw new ParseError(`live: unknown type ${JSON.stringify(m.type)}`);
  }
}

const TARGET_KINDS: readonly TargetKind[] = ["icmp", "dns", "https"];

function parseTargetSpec(value: unknown): TargetSpec {
  const t = record<{
    id: unknown;
    name: unknown;
    kind: unknown;
    address: unknown;
    expect_status: unknown;
    expect_body: unknown;
  }>(value, "target");
  if (typeof t.address !== "string") throw new ParseError("target.address: expected a string");
  return {
    id: t.id === null ? null : integer(t.id, "target.id"),
    name: nullableString(t.name, "target.name"),
    kind: oneOf(t.kind, TARGET_KINDS, "target.kind"),
    address: t.address,
    expectStatus: t.expect_status === null ? null : integer(t.expect_status, "target.expect_status"),
    expectBody: nullableString(t.expect_body, "target.expect_body"),
  };
}

/** The configuration, from `/api/config`. */
export function parseConfig(value: unknown): Config {
  const c = record<ConfigView>(value, "config");
  if (typeof c.editable !== "boolean") throw new ParseError("config.editable: expected a boolean");
  return {
    path: nullableString(c.path, "config.path"),
    editable: c.editable,
    reason: nullableString(c.reason, "config.reason"),
    targets: array(c.targets, "config.targets").map(parseTargetSpec),
  };
}

/** The public identity, in words, from `/api/host`; `null` until known. */
export function parsePublic(value: unknown): string | null {
  const host = record<{ public: unknown }>(value, "host");
  return nullableString(host.public ?? null, "host.public");
}

const NODE_KINDS: readonly MapNode["kind"][] = ["role", "router", "silent"];
const EDGE_KINDS: readonly MapEdge["kind"][] = ["probed", "traced", "unmeasured", "off-path"];

function strings<T>(value: unknown, what: string, each: (v: unknown, what: string) => T): T[] {
  return array(value, what).map((v) => each(v, what));
}

function parseMapNode(value: unknown): MapNode {
  const n = record<MapNodeView>(value, "map node");
  if (typeof n.id !== "string" || n.id === "") throw new ParseError("map node.id: expected a string");
  const ttl = n.ttl === null ? null : integer(n.ttl, "map node.ttl");
  const kind = oneOf(n.kind, NODE_KINDS, "map node.kind");
  return {
    id: n.id as MapNodeId,
    kind,
    role: kind === "role" ? roleKey(n.id, "map node.id") : null,
    depth: integer(n.depth, "map node.depth"),
    ttl,
    identity: nullableString(n.identity, "map node.identity"),
    name: nullableString(n.name, "map node.name"),
    status: n.status === null ? null : oneOf(n.status, ROLE_STATUSES, "map node.status"),
    roles: array(n.roles, "map node.roles").map((r) => {
      const role = record<{ key: string; status: string }>(r, "map node.role");
      return {
        key: roleKey(role.key, "map node.role.key"),
        status: oneOf(role.status, ROLE_STATUSES, "map node.role.status"),
      };
    }),
    sightings: array(n.sightings, "map node.sightings").map((v) => {
      const s = record<SightingView>(v, "sighting");
      return {
        target: roleKey(s.target, "sighting.target"),
        at: micros(s.at_us, "sighting.at_us"),
        sent: integer(s.sent, "sighting.sent"),
        answered: integer(s.answered, "sighting.answered"),
        medianUs: s.median_us === null ? null : integer(s.median_us, "sighting.median_us"),
      };
    }),
    targets: strings(n.targets, "map node.targets", roleKey),
  };
}

/** The network map (ADR-0042), from `/api/map` or the live stream. Every edge must
 *  join two of its nodes, and go deeper. */
export function parseMap(value: unknown): NetworkMap {
  const m = record<{ nodes: unknown; edges: unknown }>(value, "map");
  const nodes = array(m.nodes, "map.nodes").map(parseMapNode);
  const byId = new Map<string, MapNode>(nodes.map((n) => [n.id, n]));
  if (byId.size !== nodes.length) throw new ParseError("map.nodes: duplicate id");
  const edges = array(m.edges, "map.edges").map((value): MapEdge => {
    const e = record<MapEdgeView>(value, "map edge");
    const end = (id: unknown, what: string): MapNodeId => {
      const found = typeof id === "string" ? byId.get(id) : undefined;
      if (found === undefined) throw new ParseError(`${what}: no such node ${JSON.stringify(id)}`);
      return found.id;
    };
    const from = end(e.from, "map edge.from");
    const to = end(e.to, "map edge.to");
    if ((byId.get(from)?.depth ?? 0) >= (byId.get(to)?.depth ?? 0)) throw new ParseError("map edge: must go deeper");
    return {
      from,
      to,
      kind: oneOf(e.kind, EDGE_KINDS, "map edge.kind"),
      targets: strings(e.targets, "map edge.targets", roleKey),
    };
  });
  return { nodes, edges };
}
