// The history window: the recent transfers in both directions (text and files). A click on a finished item opens it (a file with its
// default program if its type is on the allow-list, else shown in its folder; a text is copied). Retry on a failed send, Cancel on a
// queued one. The backend (src-tauri/src/history.rs) owns the list and never has a sensitive text or an automatic clipboard sync in it.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { button, el, flash, historyErrorText, size, when } from "./dom";

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
type View = { items: Item[]; cap: number; phone: string | null; connected: boolean; queued: number };

const STATUS: Record<Item["status"], string> = { queued: "Queued", active: "In progress", done: "Done", failed: "Failed", canceled: "Canceled" };

function title(i: Item): string {
  if (i.kind === "text" && i.isAlert) return `Alert: ${i.title ? `${i.title} — ` : ""}${i.text ?? "(alert)"}`;
  if (i.kind === "text") return i.isLink && i.title ? `${i.title} — ${i.text ?? ""}` : (i.text ?? "(text)");
  if (i.files <= 1) return i.names[0] ?? "1 file";
  return `${i.files} files`;
}

export function startHistoryWindow(root: HTMLElement): void {
  root.className = "list";
  const head = el("p", { className: "hint" });
  const note = el("p", { className: "flash" });
  const list = el("div", { className: "items" });
  root.append(el("h1", {}, "History"), head, list, note);

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
    const arrow = el("span", { className: "dir", title: i.direction === "out" ? "Sent to the phone" : "Received from the phone" }, i.direction === "out" ? "↑" : "↓");
    const name = el("span", { className: `name ${i.kind === "text" ? "text" : ""}` }, title(i));
    const bits = [when(i.createdMs), STATUS[i.status], size(i.totalBytes)];
    if (i.error) bits.push(historyErrorText(i.error));
    if (i.retryOf) bits.push("retry");
    const meta = el("div", { className: "meta" }, ...bits.map((b, n) => el("span", { className: n === 1 ? `chip ${i.status}` : "" }, b)));
    const buttons = el("div", { className: "buttons" });
    if (i.canRetry) buttons.append(once(button("Retry", () => {}, "small"), `retry:${i.id}`, () => invoke<string>("history_retry", { id: i.id }), "Queued again."));
    if (i.canCancel) buttons.append(once(button("Cancel", () => {}, "secondary small"), `cancel:${i.id}`, () => invoke("history_cancel", { id: i.id })));
    const line = el("div", { className: `item ${i.status}${i.canOpen ? " clickable" : ""}` }, arrow, el("div", { className: "body" }, name, meta), buttons);
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
    head.textContent = `${who}${view.queued ? ` ${view.queued} waiting to be sent.` : ""} The newest ${view.cap} items are kept in this list.`;
    if (view.items.length === 0) list.replaceChildren(el("p", { className: "empty" }, "Nothing has been sent or received yet."));
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
  void listen("history-changed", () => void refresh());
  window.setInterval(() => void refresh(), 3000);
  void refresh();
}
