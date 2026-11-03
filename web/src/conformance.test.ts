// Trace conformance for the client's half of specs/live_backfill.qnt (ADR-0019):
// Quint generates traces, and the client's actions are replayed against the real
// `merge` and `latest` from series.ts. After every step the client's series must
// equal the spec's, and a reconnect must backfill from the same point the spec does.
// Needs `quint` on PATH, or set QUINT.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { parsePoint } from "./api/parse";
import type { Point } from "./model";
import { latest, merge } from "./series";

const SPEC = join(import.meta.dirname, "../../specs/live_backfill.qnt");

// ITF encodes integers as {"#bigint": "n"} and sets as {"#set": [...]}. Like any
// boundary, it's narrowed rather than cast (ADR-0019).
function field(v: unknown, name: string): unknown {
  if (typeof v !== "object" || v === null || !(name in v)) throw new Error(`ITF: no ${name}`);
  return (v as Record<string, unknown>)[name];
}
function list(v: unknown): unknown[] {
  if (!Array.isArray(v)) throw new Error("ITF: expected an array");
  return v;
}
const int = (v: unknown): number => Number(field(v, "#bigint"));
const set = (v: unknown): unknown[] => list(field(v, "#set"));

interface State {
  points: number[];
  request: number[];
  response: number[][];
  inbox: number[];
  action: string;
}

function state(raw: unknown): State {
  return {
    points: set(field(raw, "points"))
      .map(int)
      .sort((a, b) => a - b),
    request: set(field(raw, "request")).map(int),
    response: set(field(raw, "response")).map((r) => set(r).map(int)),
    inbox: list(field(raw, "inbox")).map(int),
    action: String(field(raw, "mbt::actionTaken")),
  };
}

// A spec sample time t is a point at t µs, epoch 1.
const point = (t: number): Point => parsePoint({ at_us: t, epoch: 1, rtt_us: 1000, loss: null });
const times = (ps: readonly Point[]): number[] => ps.map((p) => p.at);

describe("live stream and backfill conform to specs/live_backfill.qnt", () => {
  it("replays generated traces against merge and latest", () => {
    const dir = mkdtempSync(join(tmpdir(), "netwatch-traces-"));
    try {
      execFileSync(
        process.env["QUINT"] ?? "quint",
        [
          "run",
          SPEC,
          "--mbt",
          "--max-steps=30",
          "--n-traces=100",
          "--max-samples=100",
          "--seed=0x6c697665",
          `--out-itf=${dir}/trace_{seq}.itf.json`,
        ],
        { stdio: "ignore" },
      );
      const files = readdirSync(dir);
      expect(files.length).toBeGreaterThan(50);

      let backfills = 0;
      for (const file of files) {
        const trace: unknown = JSON.parse(readFileSync(join(dir, file), "utf8"));
        const states = list(field(trace, "states")).map(state);
        let client: Point[] = [];
        states.slice(1).forEach((after, i) => {
          const before = states[i];
          if (before === undefined) return;
          switch (after.action) {
            case "deliverLive":
              client = merge(client, [point(before.inbox[0] ?? -1)]);
              break;
            case "deliverBackfill":
              client = merge(client, (before.response[0] ?? []).sort((a, b) => a - b).map(point));
              backfills++;
              break;
            case "reconnect":
              // The client asks for everything since its latest sample, or from the start.
              expect([latest(client) ?? 0]).toEqual(after.request);
              break;
          }
          expect(times(client), `${file} step ${String(i + 1)}: ${after.action}`).toEqual(after.points);
        });
      }
      expect(backfills).toBeGreaterThan(0);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
