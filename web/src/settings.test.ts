import fc from "fast-check";
import { describe, expect, it } from "vitest";

import { parseConfig } from "./api/parse";
import { addTarget, payload, removeTarget, toDraft, updateTarget } from "./settings";

const config = parseConfig({
  path: "/etc/netwatch/netwatch.toml",
  editable: true,
  reason: null,
  targets: [
    { id: 1, name: null, kind: "icmp", address: "1.1.1.1", expect_status: null, expect_body: null },
    { id: 21, name: "Google", kind: "https", address: "https://x/", expect_status: 204, expect_body: null },
  ],
});

describe("the settings draft", () => {
  it("round-trips the configured targets unchanged", () => {
    expect(payload(toDraft(config))).toEqual([
      { id: 1, name: null, kind: "icmp", address: "1.1.1.1", expect_status: null, expect_body: null },
      { id: 21, name: "Google", kind: "https", address: "https://x/", expect_status: 204, expect_body: null },
    ]);
  });

  it("adds new targets without an id, for the daemon to number", () => {
    const [added] = payload(addTarget(toDraft(config), "dns")).slice(-1);
    expect(added).toEqual({ id: null, name: null, kind: "dns", address: "", expect_status: null, expect_body: null });
  });

  it("sends https fields only for https targets, and blank text as null", () => {
    let draft = toDraft(config);
    const google = draft[1];
    if (google === undefined) throw new Error("no second target");
    draft = updateTarget(draft, google.key, { kind: "icmp", address: " 8.8.8.8 ", name: "  " });
    expect(payload(draft)[1]).toEqual({
      id: 21,
      name: null,
      kind: "icmp",
      address: "8.8.8.8",
      expect_status: null,
      expect_body: null,
    });
  });

  it("removing what was added gives back the original", () => {
    fc.assert(
      fc.property(fc.constantFrom("icmp", "dns", "https"), (kind) => {
        const draft = toDraft(config);
        const added = addTarget(draft, kind);
        const last = added.at(-1);
        if (last === undefined) throw new Error("nothing added");
        expect(payload(removeTarget(added, last.key))).toEqual(payload(draft));
      }),
    );
  });
});
