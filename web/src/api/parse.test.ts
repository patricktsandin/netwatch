import { describe, expect, it } from "vitest";

import { ParseError, parseLive, parsePoint } from "./parse";

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
