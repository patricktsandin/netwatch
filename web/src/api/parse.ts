// The boundary parser: the only place where untyped JSON becomes domain types, and
// the only file allowed to use `as` (ADR-0019). Anything malformed is rejected, never
// coerced.

import type { EventPoint } from "./generated/EventPoint";
import type { LiveMessage } from "./generated/LiveMessage";
import type { RoleSummary } from "./generated/RoleSummary";
import type { SamplePoint } from "./generated/SamplePoint";
import type { Epoch, EventKind, Live, LossReason, Marker, Micros, Point, RoleKey } from "../model";

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

function nullableString(value: unknown, what: string): string | null {
  if (value === null || typeof value === "string") return value;
  throw new ParseError(`${what}: expected a string or null`);
}

const ROLE_KEY = /^(nic|tunnel|local-interface|target:(0|[1-9]\d*)|hop:(0|[1-9]\d*):([1-9]\d*))$/;

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

const LOSS_REASONS: readonly LossReason[] = ["timed-out", "unreachable", "send-failed"];
const EVENT_KINDS: readonly EventKind[] = [
  "epoch-started",
  "epoch-ended",
  "desynchronized",
  "resynchronized",
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

export function parsePoints(value: unknown): readonly Point[] {
  return array(value, "samples").map(parsePoint);
}

export function parseMarkers(value: unknown): readonly Marker[] {
  return array(value, "events").map(parseMarker);
}

export function parseLive(value: unknown): Live {
  const m = record<LiveMessage & { role: unknown; point: unknown; event: unknown }>(value, "live");
  switch (m.type) {
    case "sample":
      return { type: "sample", role: roleKey(m.role, "live.role"), point: parsePoint(m.point) };
    case "event":
      return { type: "event", marker: parseMarker(m.event) };
    default:
      throw new ParseError(`live: unknown type ${JSON.stringify(m.type)}`);
  }
}
