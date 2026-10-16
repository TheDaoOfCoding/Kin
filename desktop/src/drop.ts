// The drop window (the tray's left click): a target for files dragged from Explorer, a button for the file dialog, and the
// progress of the transfers with a Cancel button. The backend (src-tauri/src/transfers.rs, `Core::drop_view`) owns the state and does
// the sending. This only draws it, and passes on what was dropped.
//
// Windows doesn't support dropping files on a tray icon, which is why this window exists. The paths of the dropped files come
// from Tauri's own drag-and-drop event (a web page is never told the path of a file it is given).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";

type Transfer = {
  id: string;
  direction: "in" | "out";
  peer: string;
  status: "active" | "done" | "failed" | "canceled";
  files: number;
  filesDone?: number;
  filesActive?: number;
  bytesDone: number;
  totalBytes: number;
  speedBps?: number;
  error?: string | null;
  names: string[];
  cancelable?: boolean;
};
type View = { phone: string | null; connected: boolean; active: Transfer[]; recent: Transfer[]; folder: string };

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...children: (Node | string)[]): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

function title(t: Transfer): string {
  const what = t.files === 1 ? (t.names[0] ?? "1 file") : `${t.files} files`;
  return `${t.direction === "out" ? "To" : "From"} ${t.peer}: ${what}`;
}

function describe(t: Transfer): string {
  if (t.status === "active") {
    const speed = t.speedBps && t.speedBps > 0 ? ` · ${size(t.speedBps)}/s` : "";
    // A batch moves several files at once (up to 4): say how far it is and how many are going now.
    const files = t.files > 1 ? `${t.filesDone ?? 0} of ${t.files} files · ` : "";
    const parallel = t.filesActive && t.filesActive > 1 ? ` · ${t.filesActive} at once` : "";
    return `${files}${size(t.bytesDone)} of ${size(t.totalBytes)}${speed}${parallel}`;
  }
  const names: Record<string, string> = { done: "Done", failed: "Failed", canceled: "Canceled" };
  return `${names[t.status] ?? t.status} · ${size(t.totalBytes)}${t.error && t.status !== "done" ? ` (${t.error})` : ""}`;
}

export function startDropWindow(root: HTMLElement): void {
  root.className = "drop";
  const zone = el("div", { className: "zone", tabIndex: 0, role: "button" as string });
  const status = el("p", { className: "hint" });
  const list = el("div", { className: "transfers" });
  const folder = el("p", { className: "hint small" });
  root.append(el("h1", {}, "Send to phone"), zone, status, list, folder);

  const choose = () => void invoke("pick_files");
  zone.addEventListener("click", choose);
  zone.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") choose();
  });

  const row = (t: Transfer): HTMLElement => {
    const bar = el("progress", { max: Math.max(t.totalBytes, 1), value: t.status === "done" ? Math.max(t.totalBytes, 1) : t.bytesDone });
    const line = el("div", { className: "line" }, el("span", { className: "name" }, title(t)));
    if (t.status === "active" && t.cancelable !== false) {
      const cancel = el("button", { type: "button", className: "secondary small" }, "Cancel");
      cancel.addEventListener("click", () => {
        cancel.disabled = true;
        void invoke("cancel_transfer", { id: t.id });
      });
      line.append(cancel);
    }
    return el("div", { className: `transfer ${t.status}` }, line, t.status === "active" ? bar : "", el("div", { className: "hint small" }, describe(t)));
  };

  const draw = (view: View) => {
    zone.replaceChildren(
      el("strong", {}, "Drop files here"),
      el("span", {}, view.phone ? `or click to choose files to send to ${view.phone}` : "or click to choose files (no phone is paired yet)"),
    );
    status.textContent = view.phone ? (view.connected ? `${view.phone} is connected.` : `${view.phone} is not connected. Files can't be sent now.`) : "No phone is paired.";
    list.replaceChildren(...view.active.map(row), ...view.recent.filter((t) => !view.active.some((a) => a.id === t.id)).slice(0, 5).map(row));
    folder.textContent = `Received files go to ${view.folder}`;
  };

  let busy = false;
  const refresh = async () => {
    if (busy) return;
    busy = true;
    try {
      draw(await invoke<View>("drop_view"));
    } finally {
      busy = false;
    }
  };

  // Files dragged from Explorer onto the window. The page gets the paths from Tauri's event, never from the DOM.
  void getCurrentWebview().onDragDropEvent((event) => {
    const type = event.payload.type;
    if (type === "enter" || type === "over") zone.classList.add("over");
    else zone.classList.remove("over");
    if (type === "drop" && event.payload.paths.length > 0) void invoke("send_files", { paths: event.payload.paths });
  });

  void listen("transfers-changed", () => void refresh());
  // A transfer's numbers move all the time and a change of state is announced; this keeps the bars moving smoothly.
  window.setInterval(() => void refresh(), 700);
  void refresh();
}
