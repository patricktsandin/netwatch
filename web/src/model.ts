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

export type LossReason = "timed-out" | "unreachable" | "refused" | "unexpected-response" | "send-failed";

export type Measurement =
  | { readonly kind: "round-trip"; readonly rttMs: number }
  | { readonly kind: "lost"; readonly reason: LossReason };

/** One sample, as the UI sees it. */
export interface Point {
  readonly at: Micros;
  readonly epoch: Epoch;
  readonly measurement: Measurement;
}

export type EventKind =
  | "epoch-started"
  | "epoch-ended"
  | "desynchronized"
  | "resynchronized"
  | "kernel-log"
  | "source-disabled"
  | "clock-jump"
  | "config-drift";

/** One topology event, drawn as an event marker on every row. */
export interface Marker {
  readonly at: Micros;
  readonly kind: EventKind;
  readonly role: RoleKey | null;
  readonly epoch: Epoch | null;
  readonly detail: string | null;
}

/** A layer's latest health report (ADR-0008): the kernel's or the NIC's. */
export interface LayerReport {
  readonly status: "healthy" | "degraded" | "failing" | "unknown";
  readonly increases: readonly { readonly name: string; readonly by: number }[];
  readonly cpuPressure: number | null;
  readonly conntrackFill: number | null;
  readonly clockSynchronized: boolean | null;
  readonly disabled: readonly string[];
}

/** A role's status, as graded by the daemon (ADR-0030). */
export type Status = "healthy" | "degraded" | "failing" | "unknown" | "rate-limited";

/** A role's current identity and epoch, from Topology. */
export interface Binding {
  readonly identity: string;
  readonly epoch: Epoch;
}

export type Live =
  | { readonly type: "sample"; readonly role: RoleKey; readonly point: Point }
  | { readonly type: "event"; readonly marker: Marker }
  | { readonly type: "layer"; readonly role: RoleKey; readonly report: LayerReport }
  | { readonly type: "statuses"; readonly statuses: ReadonlyMap<RoleKey, Status> }
  | { readonly type: "bindings"; readonly bindings: ReadonlyMap<RoleKey, Binding> };
