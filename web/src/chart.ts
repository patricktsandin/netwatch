// One row's chart: RTT as a line, losses as red ticks along the bottom, incidents as
// shaded bands behind it, and the events that concern the row as vertical markers.
// Hovering shows a tooltip. Charts share one time axis (ADR-0005).

import uPlot from "uplot";

import { parseMicros } from "./api/parse";
import { ticks } from "./gauges";
import type { Incident, Marker, Micros, Point } from "./model";
import type { TooltipLine } from "./tooltip";

const color = (name: string): string => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

const seconds = (micros: number): number => micros / 1e6;

/** Every row's y axis is this wide, so the time axes line up (ADR-0005). It fits
 *  labels up to `1000 ms` and `12.5 Mbit/s`. */
const Y_AXIS_WIDTH = 84;
const HEIGHT = 120;
/** How close (in pixels) something must be to the cursor to show in the tooltip. */
const NEAR_PX = 6;

/** What a row draws, besides its points. The caller has already picked the row's
 *  own incidents and the markers that concern it. */
export interface Annotations {
  readonly incidents: readonly Incident[];
  readonly markers: readonly Marker[];
}

function markerColor(m: Marker): string {
  switch (m.kind) {
    case "epoch-started":
    case "epoch-resumed":
    case "started":
    case "epoch-ended":
    case "desynchronized":
    case "resynchronized":
    case "radio":
    case "public-address":
    case "trace":
      return color("--marker");
    case "kernel-log":
    case "clock-jump":
    case "config-drift":
    case "source-disabled":
      return color("--kernel-log");
  }
}

/** Shades incidents behind the series. */
function bands(annotations: () => Annotations, now: () => number): uPlot.Plugin {
  return {
    hooks: {
      drawClear: (u: uPlot) => {
        const { ctx } = u;
        const { left, top, width, height } = u.bbox;
        ctx.save();
        ctx.beginPath();
        ctx.rect(left, top, width, height);
        ctx.clip();
        for (const i of annotations().incidents) {
          const x0 = u.valToPos(seconds(i.started), "x", true);
          const x1 = u.valToPos(seconds(i.ended ?? now()), "x", true);
          ctx.fillStyle = color(i.severity === "outage" ? "--outage-band" : "--degradation-band");
          // At least 2 px, so a brief incident is still visible.
          ctx.fillRect(x0, top, Math.max(x1 - x0, 2 * devicePixelRatio), height);
        }
        ctx.restore();
      },
    },
  };
}

/** Draws loss ticks and event markers on top of the plot. */
function overlay(points: () => readonly Point[], annotations: () => Annotations): uPlot.Plugin {
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
        // Identity changes are dashed purple; host events (kernel log lines, clock
        // jumps, config drift) are dashed amber.
        for (const m of annotations().markers) {
          ctx.strokeStyle = markerColor(m);
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

/** A tooltip that follows the cursor, on the hovered chart only (the cursor itself
 *  is synced across rows). */
function hoverTip(describe: (at: Micros, nearUs: number) => readonly TooltipLine[]): uPlot.Plugin {
  const tip = document.createElement("div");
  tip.className = "tooltip";
  tip.hidden = true;
  let hovered = false;
  return {
    hooks: {
      init: (u: uPlot) => {
        u.over.append(tip);
        u.over.addEventListener("mouseenter", () => {
          hovered = true;
        });
        u.over.addEventListener("mouseleave", () => {
          hovered = false;
          tip.hidden = true;
        });
      },
      setCursor: (u: uPlot) => {
        const { left, top } = u.cursor;
        if (!hovered || left === undefined || top === undefined || left < 0) {
          tip.hidden = true;
          return;
        }
        const at = u.posToVal(left, "x");
        const nearUs = (u.posToVal(left + NEAR_PX, "x") - at) * 1e6;
        const lines = describe(parseMicros(at * 1e6), nearUs);
        const time = document.createElement("div");
        time.className = "tooltip-time";
        time.textContent = new Date(at * 1000).toLocaleString();
        tip.replaceChildren(
          time,
          ...lines.map((line) => {
            const el = document.createElement("div");
            el.className = `tooltip-line tone-${line.tone}`;
            el.textContent = line.text;
            return el;
          }),
        );
        tip.hidden = false;
        // Keep it inside the plot: flip to the cursor's left past the middle.
        const flip = left > u.over.clientWidth / 2;
        tip.style.left = flip ? "" : `${String(left + 12)}px`;
        tip.style.right = flip ? `${String(u.over.clientWidth - left + 12)}px` : "";
        tip.style.top = `${String(Math.min(top, Math.max(0, u.over.clientHeight - 40)))}px`;
      },
    },
  };
}

export interface Chart {
  /** Re-reads the data. A zoom, if any, is reapplied by the caller afterwards. */
  update(): void;
  redraw(): void;
  setWindow(min: number, max: number): void;
  readonly plot: uPlot;
}

/** One line on a chart. */
export interface Line {
  readonly label: string;
  /** A CSS custom property holding its color, such as `--line`. */
  readonly color: string;
}

export interface ChartSources {
  /** The plot columns: times in µs, then one value column per line. */
  readonly columns: () => { readonly xs: readonly number[]; readonly ys: readonly (readonly (number | null)[])[] };
  readonly lines: readonly Line[];
  /** How the y axis writes a value. */
  readonly format: (value: number) => string;
  /** Points whose losses are ticked along the bottom (none for gauges). */
  readonly losses: () => readonly Point[];
  readonly annotations: () => Annotations;
  readonly tooltip: (at: Micros, nearUs: number) => readonly TooltipLine[];
  readonly onZoom: (min: number, max: number) => void;
  readonly now: () => number;
}

function aligned(sources: ChartSources): uPlot.AlignedData {
  const { xs, ys } = sources.columns();
  return [xs.map(seconds), ...ys.map((y) => [...y])];
}

export function createChart(container: HTMLElement, sources: ChartSources): Chart {
  const plot = new uPlot(
    {
      width: container.clientWidth,
      height: HEIGHT,
      cursor: { sync: { key: "netwatch" } },
      legend: { show: false },
      scales: {
        x: { time: true },
        // From zero, unless values go below it (signal levels in dBm do).
        y: { range: (_u, min, max) => [Math.min(0, min), Math.max(max * 1.15, 1)] },
      },
      axes: [
        { stroke: color("--muted"), grid: { stroke: color("--grid") } },
        {
          stroke: color("--muted"),
          grid: { stroke: color("--grid") },
          size: Y_AXIS_WIDTH,
          // Our own 1–2–5 ticks: uPlot's choice can leave a short chart with a lone
          // zero, and the axis would say nothing about the scale.
          splits: (_u, _axis, min, max) => ticks(min, max, 3),
          values: (_u, v) => v.map(sources.format),
        },
      ],
      series: [
        {},
        ...sources.lines.map((line) => ({
          label: line.label,
          stroke: color(line.color),
          width: 1.5,
          spanGaps: false,
          points: { show: false },
        })),
      ],
      plugins: [
        bands(sources.annotations, sources.now),
        overlay(sources.losses, sources.annotations),
        hoverTip(sources.tooltip),
      ],
      hooks: {
        setSelect: [
          (u: uPlot) => {
            if (u.select.width <= 0) return;
            sources.onZoom(u.posToVal(u.select.left, "x"), u.posToVal(u.select.left + u.select.width, "x"));
            u.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false);
          },
        ],
      },
    },
    aligned(sources),
    container,
  );
  new ResizeObserver(() => {
    plot.setSize({ width: container.clientWidth, height: HEIGHT });
  }).observe(container);
  return {
    plot,
    update: () => {
      plot.setData(aligned(sources));
    },
    redraw: () => {
      plot.redraw();
    },
    setWindow: (min, max) => {
      plot.setScale("x", { min, max });
    },
  };
}
