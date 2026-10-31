// The boundary parser: the only place where untyped JSON becomes domain types, and
// the only file allowed to use `as` (ADR-0019). Anything malformed is rejected, never
// coerced.

import type { EventPoint } from "./generated/EventPoint";
import type { LayerReportView } from "./generated/LayerReportView";
import type { RoleSummary } from "./generated/RoleSummary";
import type { SamplePoint } from "./generated/SamplePoint";
import type {
  Binding,
  Epoch,
  EventKind,
  LayerReport,
  Live,
  LossReason,
  Marker,
  Micros,
  Point,
  RoleKey,
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

const ROLE_KEY = /^(kernel|nic|tunnel|local-interface|target:(0|[1-9]\d*)|hop:(0|[1-9]\d*):([1-9]\d*))$/;

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
  "epoch-ended",
  "desynchronized",
  "resynchronized",
  "kernel-log",
  "source-disabled",
  "clock-jump",
  "config-drift",
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
  };
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

/** Every role's binding, from `/api/bindings` or the live stream. */
export function parseBindings(value: unknown): ReadonlyMap<RoleKey, Binding> {
  const all = record<Record<string, unknown>>(value, "bindings");
  return new Map(
    Object.entries(all).map(([key, raw]) => {
      const b = record<{ identity: string; epoch: number }>(raw, "binding");
      if (typeof b.identity !== "string") throw new ParseError("binding.identity: expected a string");
      return [roleKey(key, "bindings key"), { identity: b.identity, epoch: epoch(b.epoch, "binding.epoch") }];
    }),
  );
}

/** Every layer's latest report by role key, from `/api/layers`. */
export function parseLayers(value: unknown): ReadonlyMap<RoleKey, LayerReport> {
  const all = record<Record<string, unknown>>(value, "layers");
  return new Map(
    Object.entries(all).map(([key, report]) => [roleKey(key, "layers key"), parseLayerReport(report)]),
  );
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
  };
}

export function parseLive(value: unknown): Live {
  const m = record<{
    type: unknown;
    role: unknown;
    point: unknown;
    event: unknown;
    report: unknown;
    statuses: unknown;
    bindings: unknown;
  }>(
    value,
    "live",
  );
  switch (m.type) {
    case "sample":
      return { type: "sample", role: roleKey(m.role, "live.role"), point: parsePoint(m.point) };
    case "event":
      return { type: "event", marker: parseMarker(m.event) };
    case "statuses":
      return { type: "statuses", statuses: parseStatuses(m.statuses) };
    case "bindings":
      return { type: "bindings", bindings: parseBindings(m.bindings) };
    case "layer":
      return { type: "layer", role: roleKey(m.role, "live.role"), report: parseLayerReport(m.report) };
    default:
      throw new ParseError(`live: unknown type ${JSON.stringify(m.type)}`);
  }
}
