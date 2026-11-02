// Which row each role is drawn in (ADR-0045). The daemon says which hop roles share a
// row, and with what status; this only looks that up. No I/O.

import type { RoleKey, SharedRow } from "./model";

/** The key of the row `role` is drawn in: its shared row's, or its own. */
export function rowOf(shared: readonly SharedRow[], role: RoleKey): RoleKey {
  return shared.find((r) => r.members.includes(role))?.key ?? role;
}

/** The roles row `key` shows: a shared row's members, or the role itself. */
export function membersOf(shared: readonly SharedRow[], key: RoleKey): readonly RoleKey[] {
  return shared.find((r) => r.key === key)?.members ?? [key];
}
