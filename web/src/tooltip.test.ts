import fc from "fast-check";
import { describe, expect, it } from "vitest";

import {
  parseIncident,
  parseMarker,
  parseMicros,
  parsePoint,
  parseRoles,
} from "./api/parse";
import type { Incident, Marker, Micros, Point, RoleKey } from "./model";
import {
  formatDuration,
  tooltip,
  upsertIncident,
  visibleIncidents,
  type Words,
} from "./tooltip";

// Everything is built through the boundary parser, so the tests use real domain
// values without any casts.
const us = (seconds: number): Micros => parseMicros(seconds * 1e6);
function role(key: string): RoleKey {
  const [parsed] = parseRoles([{ key, latest_us: 0 }]);
  if (parsed === undefined) throw new Error("parseRoles returned nothing");
  return parsed;
}
const words: Words = {
  time: (at) => `t${String(at / 1e6)}`,
  role: (key) => key.toUpperCase(),
};
const target = role("target:1");

const rtt = (at: number, ms: number): Point =>
  parsePoint({ at_us: at * 1e6, epoch: 1, rtt_us: ms * 1000, loss: null });
const lost = (at: number): Point =>
  parsePoint({ at_us: at * 1e6, epoch: 1, rtt_us: null, loss: "timed-out" });
const incident = (
  started: number,
  ended: number | null,
  more: Partial<Incident> = {},
): Incident => ({
  ...parseIncident({
    role: "target:1",
    started_us: started * 1e6,
    ended_us: ended === null ? null : ended * 1e6,
    severity: "outage",
    cause: null,
  }),
  ...more,
});
const marker = (at: number, more: Partial<Marker> = {}): Marker => ({
  ...parseMarker({
    at_us: at * 1e6,
    kind: "kernel-log",
    role: null,
    epoch: null,
    detail: "eth0: link down",
    roles: ["target:1"],
  }),
  ...more,
});

describe("formatDuration", () => {
  it.each([
    [0, "0 s"],
    [45e6, "45 s"],
    [200e6, "3 min 20 s"],
    [3600e6 * 2 + 300e6, "2 h 5 min"],
    [86400e6 * 3 + 3600e6, "3 d 1 h"],
  ])("%d µs is %s", (micros, text) => {
    expect(formatDuration(micros)).toBe(text);
  });
});

describe("tooltip", () => {
  const row = (
    more: { points?: Point[]; incidents?: Incident[]; markers?: Marker[] } = {},
  ) => ({
    role: target,
    points: more.points ?? [],
    incidents: more.incidents ?? [],
    markers: more.markers ?? [],
  });
  const hover = (at: number) => ({ at: us(at), nearUs: 2e6, now: us(1000) });

  it("shows the nearest point's value, if it's near enough", () => {
    const points = [rtt(10, 4.5), lost(15), rtt(20, 3)];
    expect(tooltip(hover(11), row({ points }), words)).toContainEqual({
      tone: "value",
      text: "4.50 ms",
    });
    expect(tooltip(hover(14), row({ points }), words)).toContainEqual({
      tone: "value",
      text: "lost (timed out)",
    });
    expect(
      tooltip(hover(17.5), row({ points }), words).filter(
        (l) => l.tone === "value",
      ),
    ).toEqual([]);
  });

  it("names every incident covering the time, with its duration and probable cause", () => {
    const incidents = [
      incident(100, 166, { cause: role("hop:1:1") }),
      incident(300, null, { severity: "degradation" }),
    ];
    expect(tooltip(hover(120), row({ incidents }), words)).toContainEqual({
      tone: "outage",
      text: "Outage t100–t166 (1 min 6 s) · probable cause: HOP:1:1",
    });
    expect(tooltip(hover(400), row({ incidents }), words)).toContainEqual({
      tone: "degradation",
      text: "Degradation since t300 (ongoing, 11 min 40 s)",
    });
    expect(
      tooltip(hover(200), row({ incidents }), words).filter(
        (l) => l.tone !== "value",
      ),
    ).toEqual([]);
  });

  it("finds an incident too short to see when hovering near it", () => {
    const lines = tooltip(
      hover(51),
      row({ incidents: [incident(50, 50)] }),
      words,
    );
    expect(lines.map((l) => l.tone)).toContain("outage");
  });

  it("lists nearby events concerning the row, and only those", () => {
    const markers = [
      marker(10),
      marker(11, { concerns: [role("nic")] }),
      marker(30),
      marker(12, {
        kind: "epoch-started",
        role: role("nic"),
        detail: "eth1 (ifindex 7)",
      }),
    ];
    expect(
      tooltip(hover(11), row({ markers }), words).filter(
        (l) => l.tone === "event",
      ),
    ).toEqual([
      { tone: "event", text: "t10 · kernel: eth0: link down" },
      { tone: "event", text: "t12 · NIC is now eth1 (ifindex 7)" },
    ]);
  });
});

describe("upsertIncident", () => {
  it("replaces an incident with the same role and start, and keeps start order", () => {
    const open = incident(100, null);
    const closed = incident(100, 150);
    const later = incident(200, null);
    expect(upsertIncident(upsertIncident([open], later), closed)).toEqual([
      closed,
      later,
    ]);
  });

  it("is idempotent and order-insensitive across distinct incidents", () => {
    const anIncident = fc
      .record({
        started: fc.integer({ min: 0, max: 50 }),
        role: fc.constantFrom("nic", "target:1"),
      })
      .map(({ started, role: r }) =>
        incident(started, null, { role: role(r) }),
      );
    fc.assert(
      fc.property(fc.array(anIncident), (incidents) => {
        const forward = incidents.reduce<Incident[]>(upsertIncident, []);
        const twice = incidents.reduce(upsertIncident, forward);
        expect(twice).toEqual(forward);
        const keys = (list: readonly Incident[]) =>
          list.map((i) => `${i.role}@${String(i.started)}`);
        const backward = [...incidents]
          .reverse()
          .reduce<Incident[]>(upsertIncident, []);
        expect(new Set(keys(backward))).toEqual(new Set(keys(forward)));
        expect(forward.map((i) => i.started)).toEqual(
          [...forward.map((i) => i.started)].sort((a, b) => a - b),
        );
      }),
    );
  });
});

describe("visibleIncidents", () => {
  it("keeps the row's own incidents that overlap the window, open ones included", () => {
    const incidents = [
      incident(10, 20),
      incident(90, 110),
      incident(150, null),
      incident(120, 130, { role: role("nic") }),
    ];
    expect(visibleIncidents(incidents, target, us(100), us(200))).toEqual([
      incidents[1],
      incidents[2],
    ]);
  });
});
