// The settings page's draft of the target list (ADR-0035). The daemon validates and
// numbers targets; this module only holds the edits and writes them back in the
// API's shape. It does no I/O.

import type { TargetSpec as Payload } from "./api/generated/TargetSpec";
import type { Config, TargetKind } from "./model";

/** One target being edited. `key` identifies the row on the page, since new targets
 *  have no id yet. Text fields hold exactly what was typed. */
export interface TargetDraft {
  readonly key: number;
  readonly id: number | null;
  readonly name: string;
  readonly kind: TargetKind;
  readonly address: string;
  readonly expectStatus: string;
  readonly expectBody: string;
}

export type Draft = readonly TargetDraft[];

export function toDraft(config: Config): Draft {
  return config.targets.map((t, key) => ({
    key,
    id: t.id,
    name: t.name ?? "",
    kind: t.kind,
    address: t.address,
    expectStatus: t.expectStatus === null ? "" : String(t.expectStatus),
    expectBody: t.expectBody ?? "",
  }));
}

export function addTarget(draft: Draft, kind: TargetKind): Draft {
  const key = Math.max(-1, ...draft.map((t) => t.key)) + 1;
  return [
    ...draft,
    { key, id: null, name: "", kind, address: "", expectStatus: kind === "https" ? "200" : "", expectBody: "" },
  ];
}

export function removeTarget(draft: Draft, key: number): Draft {
  return draft.filter((t) => t.key !== key);
}

export function updateTarget(draft: Draft, key: number, change: Partial<Omit<TargetDraft, "key" | "id">>): Draft {
  return draft.map((t) => (t.key === key ? { ...t, ...change } : t));
}

const blank = (text: string): string | null => (text.trim() === "" ? null : text.trim());

/** The draft as the body of `PUT /api/config`. A status that isn't a number is sent
 *  as `null`, and the daemon says what's missing. */
export function payload(draft: Draft): Payload[] {
  return draft.map((t) => {
    const https = t.kind === "https";
    const status = Number.parseInt(t.expectStatus, 10);
    return {
      id: t.id,
      name: blank(t.name),
      kind: t.kind,
      address: t.address.trim(),
      expect_status: https && Number.isInteger(status) ? status : null,
      expect_body: https ? blank(t.expectBody) : null,
    };
  });
}
