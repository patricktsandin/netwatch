import { describe, expect, it } from "vitest";

import { parseMicros, parsePoint } from "./api/parse";
import { pathAware, status } from "./status";

const ok = (at: number) => parsePoint({ at_us: at, epoch: 1, rtt_us: 1000, loss: null });
const lost = (at: number) => parsePoint({ at_us: at, epoch: 1, rtt_us: null, loss: "timed-out" });
const now = parseMicros;

describe("status", () => {
  it("is unknown with no samples, or only stale ones", () => {
    expect(status([], now(100), 10)).toBe("unknown");
    expect(status([ok(0)], now(100), 10)).toBe("unknown");
  });

  it("grades recent loss", () => {
    expect(status([ok(1), ok(2)], now(2), 10)).toBe("healthy");
    expect(status([ok(1), ok(2), lost(3)], now(3), 10)).toBe("degraded");
    expect(status([ok(1), lost(2)], now(2), 10)).toBe("failing");
  });
});

describe("pathAware", () => {
  it("calls a lossy hop rate-limited when the path beyond it is healthy", () => {
    expect(pathAware("failing", true, ["healthy", "healthy"])).toBe("rate-limited");
  });

  it("keeps loss that carries through to the target", () => {
    expect(pathAware("failing", true, ["failing", "failing"])).toBe("failing");
  });

  it("never reinterprets a target's own loss", () => {
    expect(pathAware("failing", false, ["healthy"])).toBe("failing");
  });
});
