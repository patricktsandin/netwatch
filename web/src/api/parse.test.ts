import { describe, expect, it } from "vitest";

import { ParseError, parseBindings, parseLive, parseMap, parsePoint, parseRollups } from "./parse";

describe("parsePoint", () => {
  it("accepts a round trip and converts µs to ms", () => {
    expect(parsePoint({ at_us: 1, epoch: 1, rtt_us: 3688, loss: null }).measurement).toEqual({
      kind: "round-trip",
      rttMs: 3.688,
    });
  });

  it("accepts a loss with a known reason", () => {
    expect(parsePoint({ at_us: 1, epoch: 1, rtt_us: null, loss: "send-failed" }).measurement).toEqual({
      kind: "lost",
      reason: "send-failed",
    });
  });

  it.each([
    [{ at_us: 1, epoch: 1, rtt_us: 5, loss: "timed-out" }, "both rtt and loss"],
    [{ at_us: 1, epoch: 1, rtt_us: null, loss: null }, "neither"],
    [{ at_us: 1, epoch: 0, rtt_us: 5, loss: null }, "epoch zero"],
    [{ at_us: 1.5, epoch: 1, rtt_us: 5, loss: null }, "fractional time"],
    [{ at_us: 1, epoch: 1, rtt_us: -1, loss: null }, "negative rtt"],
    [{ at_us: 1, epoch: 1, rtt_us: null, loss: "eaten" }, "unknown loss"],
    [null, "null"],
    [[], "array"],
  ])("rejects %j (%s)", (value: unknown, why: string) => {
    expect(() => parsePoint(value), why).toThrow(ParseError);
  });
});

describe("parseLive", () => {
  it("rejects a malformed role key", () => {
    expect(() =>
      parseLive({ type: "sample", role: "target:01", point: { at_us: 1, epoch: 1, rtt_us: 1, loss: null } }),
    ).toThrow(ParseError);
  });

  it("rejects an unknown message type", () => {
    expect(() => parseLive({ type: "gossip" })).toThrow(ParseError);
  });
});

describe("parseRollups", () => {
  it("draws an interval at its median, or as a loss if nothing was answered", () => {
    const [answered, silent] = parseRollups([
      { start_us: 60e6, epoch: 1, count: 12, lost: 1, mean_us: 3000, p50_us: 2900, p95_us: 4000 },
      { start_us: 120e6, epoch: 1, count: 12, lost: 12, mean_us: null, p50_us: null, p95_us: null },
    ]);
    expect(answered?.measurement).toEqual({ kind: "round-trip", rttMs: 2.9 });
    expect(silent?.measurement.kind).toBe("lost");
  });

  it("rejects more losses than samples", () => {
    expect(() => parseRollups([{ start_us: 0, epoch: 1, count: 1, lost: 2, p50_us: null }])).toThrow(ParseError);
  });
});

describe("parseBindings", () => {
  it("carries a hop's router name, or null until the daemon has one", () => {
    const bindings = parseBindings({
      "hop:1:2": { identity: "81.2.69.142", name: "ae1.example.net · AS64500", epoch: 3 },
      nic: { identity: "eth0 (ifindex 2)", name: null, epoch: 1 },
    });
    const names = Object.fromEntries([...bindings].map(([key, b]) => [String(key), b.name]));
    expect(names).toEqual({ "hop:1:2": "ae1.example.net · AS64500", nic: null });
  });

  it("accepts a path trace as an event", () => {
    const live = parseLive({
      type: "event",
      event: { at_us: 1, kind: "trace", role: "target:1", epoch: null, detail: "routine path trace", roles: [] },
    });
    expect(live.type === "event" && live.marker.kind).toBe("trace");
  });
});

describe("parseMap", () => {
  const node = (id: string, depth: number) => ({
    id,
    kind: "role",
    depth,
    ttl: null,
    identity: null,
    name: null,
    status: "healthy",
    roles: [{ key: id, status: "healthy" }],
    sightings: [],
    targets: [],
  });
  const edge = (from: string, to: string) => ({ from, to, kind: "probed", targets: [] });

  it("accepts edges that join two of its nodes and go deeper", () => {
    const map = parseMap({ nodes: [node("kernel", 0), node("nic", 1)], edges: [edge("kernel", "nic")] });
    expect(map.edges[0]?.to).toBe("nic");
  });

  it.each([
    [{ nodes: [node("kernel", 0)], edges: [edge("kernel", "nic")] }, "an edge to no node"],
    [{ nodes: [node("kernel", 0), node("nic", 1)], edges: [edge("nic", "kernel")] }, "an edge going up"],
    [{ nodes: [node("kernel", 0), node("kernel", 1)], edges: [] }, "a duplicate id"],
    [{ nodes: [{ ...node("kernel", 0), status: "fine" }], edges: [] }, "an unknown status"],
  ])("rejects %j (%s)", (value: unknown, why: string) => {
    expect(() => parseMap(value), why).toThrow(ParseError);
  });
});
