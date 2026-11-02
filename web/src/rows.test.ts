import { describe, expect, it } from "vitest";

import { parseSharedRows, parseStatuses } from "./api/parse";
import { membersOf, rowOf } from "./rows";

const shared = parseSharedRows([
  { key: "hop:1:1", members: ["hop:1:1", "hop:2:1", "hop:11:1"], status: "healthy" },
  { key: "hop:1:2", members: ["hop:1:2", "hop:11:2"], status: "degraded" },
]);
// Role keys, through the boundary parser.
const keys = [...parseStatuses({ "hop:11:1": "healthy", "hop:1:2": "healthy", nic: "healthy" }).keys()];
const key = (text: string) => {
  const found = keys.find((k) => k === text);
  if (found === undefined) throw new Error(`no key ${text}`);
  return found;
};

describe("rows", () => {
  it("draws a shared stream's hop roles in the owner's row", () => {
    expect(rowOf(shared, key("hop:11:1"))).toBe("hop:1:1");
    expect(membersOf(shared, key("hop:1:2"))).toEqual(["hop:1:2", "hop:11:2"]);
  });

  it("gives every other role a row of its own", () => {
    expect(rowOf(shared, key("nic"))).toBe("nic");
    expect(membersOf(shared, key("nic"))).toEqual(["nic"]);
  });
});
