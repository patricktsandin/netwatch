// One row's chart: RTT as a line, losses as red ticks along the bottom, and topology
// events as vertical markers. Charts share one time axis (ADR-0005).

import uPlot from "uplot";

import type { Marker, Point } from "./model";
import { plotColumns } from "./series";

/** Points further apart than this are a measurement gap, not a line. */
const MAX_GAP_US = 15e6;

const color = (name: string): string =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim();

const seconds = (micros: number): number => micros / 1e6;

function data(points: readonly Point[]): uPlot.AlignedData {
  const { xs, ys } = plotColumns(points, MAX_GAP_US);
  return [xs.map(seconds), ys];
}

/** Draws loss ticks and event markers on top of the plot. */
function overlay(points: () => readonly Point[], markers: () => readonly Marker[]): uPlot.Plugin {
  return {
    hooks: {
      draw: (u: uPlot) => {
        const { ctx } = u;
        const { top, height } = u.bbox;
        ctx.save();
        ctx.lineWidth = 2 * devicePixelRatio;
        ctx.strokeStyle = color("--loss");
        for (const p of points()) {
          if (p.measurement.kind !== "lost") continue;
          const x = u.valToPos(seconds(p.at), "x", true);
          ctx.beginPath();
          ctx.moveTo(x, top + height);
          ctx.lineTo(x, top + height - 8 * devicePixelRatio);
          ctx.stroke();
        }
        ctx.lineWidth = devicePixelRatio;
        ctx.setLineDash([4 * devicePixelRatio, 3 * devicePixelRatio]);
        ctx.strokeStyle = color("--marker");
        for (const m of markers()) {
          if (m.kind !== "epoch-started" && m.kind !== "desynchronized") continue;
          const x = u.valToPos(seconds(m.at), "x", true);
          ctx.beginPath();
          ctx.moveTo(x, top);
          ctx.lineTo(x, top + height);
          ctx.stroke();
        }
        ctx.restore();
      },
    },
  };
}

export interface Chart {
  update(points: readonly Point[]): void;
  redraw(): void;
  setWindow(min: number, max: number): void;
  readonly plot: uPlot;
}

export function createChart(
  container: HTMLElement,
  points: () => readonly Point[],
  markers: () => readonly Marker[],
  onZoom: (min: number, max: number) => void,
): Chart {
  const plot = new uPlot(
    {
      width: container.clientWidth,
      height: 120,
      cursor: { sync: { key: "netwatch" } },
      legend: { show: false },
      scales: { x: { time: true }, y: { range: (_u, _min, max) => [0, Math.max(max * 1.15, 1)] } },
      axes: [
        { stroke: color("--muted"), grid: { stroke: color("--grid") } },
        { stroke: color("--muted"), grid: { stroke: color("--grid") }, size: 48, values: (_u, v) => v.map((n) => `${String(n)} ms`) },
      ],
      series: [{}, { stroke: color("--line"), width: 1.5, spanGaps: false, points: { show: false } }],
      plugins: [overlay(points, markers)],
      hooks: {
        setSelect: [
          (u: uPlot) => {
            if (u.select.width <= 0) return;
            onZoom(u.posToVal(u.select.left, "x"), u.posToVal(u.select.left + u.select.width, "x"));
            u.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false);
          },
        ],
      },
    },
    data(points()),
    container,
  );
  new ResizeObserver(() => {
    plot.setSize({ width: container.clientWidth, height: 120 });
  }).observe(container);
  return {
    plot,
    // Rescales to the data. A zoom, if any, is reapplied by the caller afterwards.
    update: (ps) => {
      plot.setData(data(ps));
    },
    redraw: () => {
      plot.redraw();
    },
    setWindow: (min, max) => {
      plot.setScale("x", { min, max });
    },
  };
}
