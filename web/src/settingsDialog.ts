// The settings dialog (ADR-0035): shows the configured targets, keeps the edits in a
// draft (settings.ts), and sends them to the daemon, which validates, saves, and
// restarts. Only this file and main.ts touch the DOM and the network.

import { parseConfig } from "./api/parse";
import type { TargetKind } from "./model";
import { addTarget, type Draft, payload, removeTarget, toDraft, type TargetDraft, updateTarget } from "./settings";

const KINDS: readonly { readonly kind: TargetKind; readonly label: string; readonly hint: string }[] = [
  { kind: "icmp", label: "ICMP", hint: "address, e.g. 1.1.1.1" },
  { kind: "dns", label: "DNS", hint: "server address, or system" },
  { kind: "https", label: "HTTPS", hint: "https://host/path" },
];

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> = {},
): HTMLElementTagNameMap[K] {
  return Object.assign(document.createElement(tag), props);
}

function input(value: string, placeholder: string, onInput: (value: string) => void, className = ""): HTMLInputElement {
  const field = el("input", { value, placeholder, className });
  field.addEventListener("input", () => {
    onInput(field.value);
  });
  return field;
}

/** Opens the settings dialog. `onSaved` runs once the daemon has accepted the change
 *  and is restarting. */
export async function openSettings(onSaved: () => void): Promise<void> {
  const dialog = document.getElementById("settings-dialog");
  if (!(dialog instanceof HTMLDialogElement)) return;
  const response = await fetch("/api/config");
  const config = parseConfig(await response.json());
  let draft: Draft = toDraft(config);

  const status = el("p", { className: "hint" });
  const list = el("div", { className: "targets" });
  const save = el("button", { textContent: "Save and restart", className: "primary", disabled: !config.editable });

  const row = (t: TargetDraft): HTMLElement => {
    const change = (patch: Partial<Omit<TargetDraft, "key" | "id">>): void => {
      draft = updateTarget(draft, t.key, patch);
    };
    const kind = el("select");
    for (const k of KINDS)
      kind.append(el("option", { value: k.kind, textContent: k.label, selected: k.kind === t.kind }));
    kind.addEventListener("change", () => {
      const chosen = KINDS.find((k) => k.kind === kind.value);
      if (chosen !== undefined) change({ kind: chosen.kind });
      render();
    });
    const hint = KINDS.find((k) => k.kind === t.kind)?.hint ?? "";
    const remove = el("button", { textContent: "Remove", type: "button" });
    remove.addEventListener("click", () => {
      draft = removeTarget(draft, t.key);
      render();
    });
    const fields: HTMLElement[] = [
      el("span", { className: "target-id", textContent: t.id === null ? "new" : `#${String(t.id)}` }),
      kind,
      input(
        t.address,
        hint,
        (v) => {
          change({ address: v });
        },
        "wide",
      ),
      input(t.name, "name (optional)", (v) => {
        change({ name: v });
      }),
    ];
    if (t.kind === "https") {
      fields.push(
        input(
          t.expectStatus,
          "status",
          (v) => {
            change({ expectStatus: v });
          },
          "narrow",
        ),
        input(t.expectBody, "body contains (optional)", (v) => {
          change({ expectBody: v });
        }),
      );
    }
    fields.push(remove);
    const line = el("div", { className: "target-row" });
    line.append(...fields);
    return line;
  };

  function render(): void {
    list.replaceChildren(...draft.map(row));
  }
  render();

  const add = el("button", { textContent: "Add target", type: "button" });
  add.addEventListener("click", () => {
    draft = addTarget(draft, "icmp");
    render();
  });
  const close = el("button", { textContent: "Close", type: "button" });
  close.addEventListener("click", () => {
    dialog.close();
  });
  save.addEventListener("click", () => {
    save.disabled = true;
    status.textContent = "Saving…";
    void fetch("/api/config", {
      method: "PUT",
      headers: { "Content-Type": "application/json", "X-Netwatch": "1" },
      body: JSON.stringify(payload(draft)),
    })
      .then(async (r) => {
        if (r.ok) {
          status.textContent = "Saved. netwatch is restarting with the new targets…";
          onSaved();
          setTimeout(() => {
            dialog.close();
          }, 1500);
        } else {
          status.textContent = await r.text();
          status.className = "hint error";
          save.disabled = false;
        }
      })
      .catch((error: unknown) => {
        // The daemon couldn't be reached: say so, and let the user try again.
        status.textContent = `Couldn't reach netwatch: ${String(error)}`;
        status.className = "hint error";
        save.disabled = false;
      });
  });

  status.textContent = config.editable
    ? `Targets in ${config.path ?? "the configuration"}. Saving restarts netwatch; history is kept by target id.`
    : `Read-only: ${config.reason ?? "the configuration can't be edited"}.`;
  const actions = el("div", { className: "actions" });
  actions.append(add, el("span", { className: "spacer" }), close, save);

  // Flushing history (ADR-0038) can't be undone, so it asks first.
  const flushStatus = el("p", { className: "hint" });
  const flush = el("button", { textContent: "Flush all data…", type: "button", className: "danger" });
  flush.addEventListener("click", () => {
    if (
      !confirm("Delete all recorded history: samples, readings, rollups, incidents, and events? This can't be undone.")
    ) {
      return;
    }
    flush.disabled = true;
    flushStatus.textContent = "Flushing…";
    void fetch("/api/history", {
      method: "DELETE",
      headers: { "Content-Type": "application/json", "X-Netwatch": "1" },
    })
      .then(async (r) => {
        flush.disabled = false;
        flushStatus.textContent = r.ok ? "All history flushed. Graphs start over from now." : await r.text();
        flushStatus.className = r.ok ? "hint" : "hint error";
      })
      .catch((error: unknown) => {
        flush.disabled = false;
        flushStatus.textContent = `Couldn't reach netwatch: ${String(error)}`;
        flushStatus.className = "hint error";
      });
  });
  const danger = el("div", { className: "danger-zone" });
  danger.append(
    el("h2", { textContent: "History" }),
    el("p", {
      className: "hint",
      textContent: "Targets and settings are kept; everything recorded so far is deleted.",
    }),
    flush,
    flushStatus,
  );
  dialog.replaceChildren(el("h2", { textContent: "Targets" }), status, list, actions, danger);
  dialog.showModal();
}
