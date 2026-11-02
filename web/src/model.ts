// The frontend's domain types. Values arrive as plain JSON, are parsed once at the
// boundary (api/parse.ts), and are only ever these branded types afterwards
// (ADR-0019).

declare const brand: unique symbol;
type Brand<T, B extends string> = T & { readonly [brand]: B };

/** A role's stable key, such as `nic` or `target:1` (see `Role::key` in Rust). */
export type RoleKey = Brand<string, "RoleKey">;
/** Microseconds since the Unix epoch. */
export type Micros = Brand<number, "Micros">;
/** An identity epoch. */
export type Epoch = Brand<number, "Epoch">;

export type LossReason =
  | "timed-out"
  | "unreachable"
  | "time-exceeded"
  | "refused"
  | "unexpected-response"
  | "send-failed";

export type Measurement =
  { readonly kind: "round-trip"; readonly rttMs: number } | { readonly kind: "lost"; readonly reason: LossReason };

/** One sample, as the UI sees it. */
export interface Point {
  readonly at: Micros;
  readonly epoch: Epoch;
  readonly measurement: Measurement;
}

export type EventKind =
  | "epoch-started"
  | "epoch-resumed"
  | "started"
  | "epoch-ended"
  | "desynchronized"
  | "resynchronized"
  | "kernel-log"
  | "source-disabled"
  | "clock-jump"
  | "config-drift"
  | "radio"
  | "public-address"
  | "trace";

/** One event, drawn as an event marker on the rows it concerns. */
export interface Marker {
  readonly at: Micros;
  readonly kind: EventKind;
  readonly role: RoleKey | null;
  readonly epoch: Epoch | null;
  readonly detail: string | null;
  /** The rows it belongs on, as the daemon decided (ADR-0031). */
  readonly concerns: readonly RoleKey[];
}

/** A gauge's key, such as `rx-throughput` (see `Gauge::key` in Rust). */
export type GaugeKey = Brand<string, "GaugeKey">;

/** The units gauges come in (ADR-0032). The unit says how to format a value. */
export type GaugeUnit = "bit/s" | "dBm" | "dB" | "%";

export interface GaugeInfo {
  readonly key: GaugeKey;
  readonly label: string;
  readonly unit: GaugeUnit;
}

/** One gauge value of a role. In the rollup ranges, an interval's mean. */
export interface Reading {
  readonly at: Micros;
  readonly epoch: Epoch;
  readonly gauge: GaugeKey;
  readonly value: number;
}

export type Severity = "degradation" | "outage";

/** One of a role's degraded or failing periods, as the daemon recorded it
 *  (ADR-0031). An incident is identified by its role and start. */
export interface Incident {
  readonly role: RoleKey;
  readonly started: Micros;
  /** `null` while it's still open. */
  readonly ended: Micros | null;
  readonly severity: Severity;
  readonly cause: RoleKey | null;
}

/** A layer's latest health report (ADR-0008): the kernel's or the NIC's. */
export interface LayerReport {
  readonly status: "healthy" | "degraded" | "failing" | "unknown";
  readonly increases: readonly { readonly name: string; readonly by: number }[];
  readonly cpuPressure: number | null;
  readonly conntrackFill: number | null;
  readonly clockSynchronized: boolean | null;
  readonly disabled: readonly string[];
  /** What the layer is doing, in the daemon's words, such as a radio's state. */
  readonly notes: readonly string[];
}

/** A role's status, as graded by the daemon (ADR-0030). */
export type Status = "healthy" | "degraded" | "failing" | "unknown" | "rate-limited";

/** A role's current identity and epoch, from Topology. */
export interface Binding {
  readonly identity: string;
  /** For an address, its router name (ADR-0040), once the daemon has looked it up. */
  readonly name: string | null;
  readonly epoch: Epoch;
}

/** A network map node's stable key (ADR-0042): a role key, `router:<address>:<ttl>`,
 *  or `silent:<target>:<ttl>`. */
export type MapNodeId = Brand<string, "MapNodeId">;

/** How a map edge's step is known (glossary: *map edge*). */
export type EdgeKind = "probed" | "traced" | "unmeasured" | "off-path";

/** What one target's latest trace found at a map node. */
export interface Sighting {
  readonly target: RoleKey;
  readonly at: Micros;
  readonly sent: number;
  readonly answered: number;
  readonly medianUs: number | null;
}

/** One node of the network map (glossary: *map node*). */
export interface MapNode {
  readonly id: MapNodeId;
  readonly kind: "role" | "router" | "silent";
  /** For a node of its own role, that role: its id is the role's key. */
  readonly role: RoleKey | null;
  /** Its position along the path. Every edge goes to a greater depth. */
  readonly depth: number;
  readonly ttl: number | null;
  readonly identity: string | null;
  readonly name: string | null;
  /** `null` for a node seen only by a trace. */
  readonly status: Status | null;
  readonly roles: readonly { readonly key: RoleKey; readonly status: Status }[];
  readonly sightings: readonly Sighting[];
  readonly targets: readonly RoleKey[];
}

export interface MapEdge {
  readonly from: MapNodeId;
  readonly to: MapNodeId;
  readonly kind: EdgeKind;
  readonly targets: readonly RoleKey[];
}

/** The network map, as the daemon built it (ADR-0042). */
export interface NetworkMap {
  readonly nodes: readonly MapNode[];
  readonly edges: readonly MapEdge[];
}

/** A row several hop roles share, because one probe stream serves them all
 *  (ADR-0045). Its key is its owner's, whose samples it draws. */
export interface SharedRow {
  readonly key: RoleKey;
  /** Every role it shows, the owner first. */
  readonly members: readonly RoleKey[];
  readonly status: Status;
}

export type Live =
  | { readonly type: "sample"; readonly role: RoleKey; readonly point: Point }
  | { readonly type: "event"; readonly marker: Marker }
  | { readonly type: "incident"; readonly incident: Incident }
  | { readonly type: "cleared" }
  | { readonly type: "reading"; readonly role: RoleKey; readonly reading: Reading }
  | { readonly type: "layer"; readonly role: RoleKey; readonly report: LayerReport }
  | {
      readonly type: "statuses";
      readonly statuses: ReadonlyMap<RoleKey, Status>;
      /** Each shared row's status, by row key. */
      readonly rows: ReadonlyMap<RoleKey, Status>;
    }
  | { readonly type: "bindings"; readonly bindings: ReadonlyMap<RoleKey, Binding> }
  | { readonly type: "map"; readonly map: NetworkMap };

export type TargetKind = "icmp" | "dns" | "https";

/** One target as configured (ADR-0035). */
export interface TargetSpec {
  readonly id: number | null;
  readonly name: string | null;
  readonly kind: TargetKind;
  readonly address: string;
  readonly expectStatus: number | null;
  readonly expectBody: string | null;
}

/** The configuration as the settings page sees it. */
export interface Config {
  readonly path: string | null;
  readonly editable: boolean;
  readonly reason: string | null;
  readonly targets: readonly TargetSpec[];
}
