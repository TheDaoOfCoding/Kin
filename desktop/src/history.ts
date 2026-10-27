// The history window: the recent transfers in both directions (text and files). A click on a finished item opens it (a file with its
// default program if its type is on the allow-list, else shown in its folder; a text is copied). Retry on a failed send, Cancel on a
// queued one. The backend (src-tauri/src/history.rs) owns the list and never has a sensitive text or an automatic clipboard sync in it.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { actionButton, el, flash, historyErrorText, size, when } from "./dom";
import { emptyArt, icon } from "./icons";
import type { PageHost } from "./shell";

type Item = {
  id: string;
  direction: "in" | "out";
  kind: "file" | "text";
  status: "queued" | "active" | "done" | "failed" | "canceled";
  createdMs: number;
  finishedMs: number | null;
  totalBytes: number;
  error: string | null;
  files: number;
  names: string[];
  text: string | null;
  isUrl: boolean;
  isLink: boolean;
  /** 2B.5: a text of the user's own script that went to the phone as an alert (`Kin.exe --notify`, the browser's downloads). */
  isAlert?: boolean;
  title: string | null;
  retryOf: string | null;
  retried: boolean;
  canOpen: boolean;
  opensText: boolean;
  canRetry: boolean;
  canCancel: boolean;
};
type View = { items: Item[]; cap: number; phone: string | null; connected: boolean; queued: number; pcName: string; version: string };

/** The word on an item's chip: what happened to it, in the direction it went. */
function statusWord(i: Item): string {
  if (i.status === "done") return i.direction === "out" ? "Sent" : "Received";
  return { queued: "Waiting", active: "In progress", failed: "Failed", canceled: "Canceled" }[i.status];
}

function title(i: Item): string {
  if (i.kind === "text" && i.isAlert) return `Alert: ${i.title ? `${i.title}: ` : ""}${i.text ?? "(alert)"}`;
  if (i.kind === "text") return i.isLink && i.title ? `${i.title}: ${i.text ?? ""}` : (i.text ?? "(text)");
  if (i.files <= 1) return i.names[0] ?? "1 file";
  return `${i.files} files`;
}

export function startHistoryPage(host: PageHost): void {
  const shell = host;
  const note = el("p", { className: "flash" });
  const list = el("div", { className: "items card" });
  shell.body.append(list, note);

  const act = async (p: Promise<unknown>, done?: string) => {
    try {
      const r = await p;
      if (typeof r === "string" && r) flash(note, done ?? r);
    } catch (e) {
      flash(note, String(e), true);
    }
    void refresh();
  };

  // A Retry or Cancel that is being carried out: its button stays disabled until the call is over, also across a redraw of the list (the list
  // is redrawn every few seconds and whenever the history changes). A double click can't make two calls.
  const inFlight = new Set<string>();
  const once = (b: HTMLButtonElement, key: string, run: () => Promise<unknown>, done?: string) => {
    b.disabled = inFlight.has(key);
    b.addEventListener("click", () => {
      if (inFlight.has(key)) return;
      inFlight.add(key);
      b.disabled = true;
      void act(run().finally(() => inFlight.delete(key)), done);
    });
    return b;
  };

  const row = (i: Item): HTMLElement => {
    const out = i.direction === "out";
    const failed = i.status === "failed";
    const lead = el("span", { className: `av ${failed ? "err" : i.kind === "text" ? "lilac" : "sage"}` }, icon(i.kind === "text" ? "chat" : "file"));
    const name = el("b", { className: i.kind === "text" ? "text" : "", title: title(i) }, title(i));
    const bits = [when(i.createdMs), size(i.totalBytes)];
    if (i.error) bits.push(historyErrorText(i.error));
    if (i.retryOf) bits.push("retry");
    const meta = el("small", failed ? { className: "err-t" } : {}, icon(out ? "up" : "down"), bits.join(" · "));
    const chip =
      i.status === "done"
        ? el("span", { className: "chip ok" }, icon("check"), statusWord(i))
        : el("span", { className: `chip ${i.status === "canceled" || i.status === "queued" ? "neutral" : "warn"}` }, statusWord(i));
    const buttons = el("div", { className: "buttons" });
    if (i.canRetry) buttons.append(once(actionButton("retry", "Retry", () => {}, "soft sm"), `retry:${i.id}`, () => invoke<string>("history_retry", { id: i.id }), "Queued again."));
    if (i.canCancel) buttons.append(once(actionButton(null, "Cancel", () => {}, "plain sm"), `cancel:${i.id}`, () => invoke("history_cancel", { id: i.id })));
    const line = el("div", { className: `prow ${i.status}${i.canOpen ? " clickable" : ""}` }, lead, el("div", { className: "grow" }, name, meta), chip, buttons);
    if (i.canOpen) {
      line.title = i.opensText ? (i.isLink ? "Click to copy this link" : "Click to copy this text") : "Click to open";
      line.addEventListener("click", (e) => {
        if ((e.target as HTMLElement).closest("button")) return;
        void act(invoke<string>("history_open", { id: i.id }));
      });
    }
    return line;
  };

  const draw = (view: View) => {
    const who = view.phone ? (view.connected ? `${view.phone} is connected.` : `${view.phone} is not connected.`) : "No phone is paired.";
    shell.setPc(view.pcName, view.version);
    shell.sub.textContent = `${who}${view.queued ? ` ${view.queued} waiting to be sent.` : ""} The newest ${view.cap} items are kept in this list.`;
    list.classList.toggle("is-empty", view.items.length === 0);
    if (view.items.length === 0) list.replaceChildren(el("div", { className: "empty" }, emptyArt("history"), el("b", {}, "Nothing here yet"), el("p", {}, "Things you send and receive will show up here, newest first.")));
    else list.replaceChildren(...view.items.map(row));
  };

  let busy = false;
  const refresh = async () => {
    if (busy) return;
    busy = true;
    try {
      draw(await invoke<View>("history_view"));
    } finally {
      busy = false;
    }
  };
  void listen("history-changed", () => host.visible() && void refresh());
  host.every(3000, () => void refresh());
  host.onShow(() => void refresh());
}
