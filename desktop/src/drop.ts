// The drop window (the tray's left click): a target for files dragged from Explorer, a button for the file dialog, and the
// progress of the transfers with a Cancel button. The backend (src-tauri/src/transfers.rs, `Core::drop_view`) owns the state and does
// the sending. This only draws it, and passes on what was dropped.
//
// Windows doesn't support dropping files on a tray icon, which is why this window exists. The paths of the dropped files come
// from Tauri's own drag-and-drop event (a web page is never told the path of a file it is given).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { button, sensitiveWhilePausedText } from "./dom";

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
/** The media strip (2B.3): the phone's session (with its title: this window was opened by the user), the PC's own session being shared (read-only) and the PC's volume. */
type PhoneMedia = {
  from: string;
  app: string;
  title: string;
  artist: string | null;
  state: "playing" | "paused" | "buffering" | "stopped";
  positionMs: number | null;
  durationMs: number | null;
  actions: string[];
  art: string | null;
  volume?: { level: number; muted: boolean };
};
type Media = {
  phone: PhoneMedia | null;
  pc: { title: string; artist: string | null; app: string; state: string } | null;
  volume: { level: number; muted: boolean } | null;
  testMode: boolean;
};
/** The phone strip (2B.4): the phone's battery, the pause of both devices, Ring my phone and Pause. (The same shape as the Settings window's `Presence`.) */
type Presence = {
  pause: { on: boolean; untilResumed: boolean; text: string | null; quietArrivals: number; quietSensitive: number };
  phone: {
    name: string | null;
    connected: boolean;
    battery: { level: number; charging: boolean } | null;
    lastKnown: { level: number; charging: boolean } | null;
    paused: boolean;
    pausedText: string | null;
  };
  ring: { can: boolean; active: boolean; text: string | null; reason: string | null };
  showPhone: boolean;
};
type View = { phone: string | null; connected: boolean; active: Transfer[]; recent: Transfer[]; folder: string; media: Media; presence: Presence };

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
  const media = el("div", { className: "media" });
  const strip = el("div", { className: "phonestrip" });
  root.append(el("h1", {}, "Send to phone"), strip, media, zone, status, list, folder);

  /** The phone strip: name and battery, "Paused until 14:30" chips for both devices, Ring my phone and Pause. Nothing runs while the window is closed. */
  const drawStrip = (p: Presence) => {
    const name = p.phone.name;
    if (!name) {
      strip.replaceChildren();
      strip.hidden = true;
      return;
    }
    strip.hidden = false;
    const bat = p.phone.connected ? p.phone.battery : p.phone.lastKnown;
    const batText = p.showPhone && bat ? ` · ${bat.level}%${bat.charging ? " ⚡" : ""}${p.phone.connected ? "" : " (last known)"}` : "";
    const chips: HTMLElement[] = [];
    if (p.pause.on) chips.push(el("span", { className: "chip" }, `This PC paused ${p.pause.text ?? ""}`));
    if (p.phone.connected && p.phone.paused) chips.push(el("span", { className: "chip" }, `${name} paused ${p.phone.pausedText ?? ""}`));
    const say = el("span", { className: "hint small" }, p.ring.active ? (p.ring.text ?? "") : p.ring.can ? "" : p.ring.reason ?? "");
    const act = async (command: string, args: Record<string, unknown>) => {
      try {
        say.textContent = String((await invoke<string | null>(command, args)) ?? "");
      } catch (e) {
        say.textContent = String(e);
      }
      void refresh();
    };
    const ring = p.ring.active
      ? button("Stop ringing phone", () => void act("ring_phone", { start: false }), "small")
      : (() => {
          const b = button("Ring my phone", () => void act("ring_phone", { start: true }), "secondary small");
          b.disabled = !p.ring.can;
          return b;
        })();
    const pauseButtons = p.pause.on
      ? [button("Resume Kin", () => void act("set_pause", { choice: "resume" }), "small")]
      : [
          button("Pause 1 h", () => void act("set_pause", { choice: "1h" }), "secondary small"),
          button("Until tomorrow", () => void act("set_pause", { choice: "tomorrow" }), "secondary small"),
          button("Until I resume", () => void act("set_pause", { choice: "resumed" }), "secondary small"),
        ];
    strip.replaceChildren(
      el("strong", {}, `${name}${batText}`),
      ...chips,
      el("div", { className: "row left" }, ring, ...pauseButtons),
      say,
      p.pause.on && p.pause.quietArrivals ? el("span", { className: "hint small" }, `${p.pause.quietArrivals} received while paused`) : "",
      p.pause.on && p.pause.quietSensitive ? el("span", { className: "hint small" }, sensitiveWhilePausedText(p.pause.quietSensitive)) : "",
    );
  };

  const command = (action: string, value?: number) => void invoke("media_command", { action, value: value ?? null });
  const time = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
  let dragging = false;
  let mediaKey = "";
  let progress: HTMLInputElement | null = null;
  let clock: HTMLElement | null = null;
  let stateLine: HTMLElement | null = null;

  /** A button of the media strip, enabled only if the phone's player offers it. */
  const control = (label: string, onClick: () => void, enabled: boolean): HTMLButtonElement => {
    const b = button(label, onClick, "secondary small");
    b.disabled = !enabled;
    return b;
  };

  const slider = (value: number, max: number, onChange: (v: number) => void, label: string): HTMLInputElement => {
    const r = el("input", { type: "range", min: "0", max: String(max), value: String(value), ariaLabel: label });
    r.addEventListener("pointerdown", () => (dragging = true));
    r.addEventListener("change", () => {
      dragging = false;
      onChange(Number(r.value));
    });
    r.addEventListener("input", () => onChange(Number(r.value)));
    return r;
  };

  /** The phone's session, the PC's session and the PC's volume. Rebuilt when something but the position changed; the position and the state are updated in place. */
  const drawMedia = (m: Media) => {
    const p = m.phone;
    const key = JSON.stringify([p && [p.from, p.app, p.title, p.artist, p.actions, p.art, p.durationMs, p.volume !== undefined], m.pc && [m.pc.title, m.pc.artist, m.pc.app], m.volume !== null]);
    if (key === mediaKey) {
      if (p && !dragging) {
        if (progress && p.positionMs !== null) progress.value = String(p.positionMs);
        if (clock && p.positionMs !== null && p.durationMs) clock.textContent = `${time(p.positionMs)} / ${time(p.durationMs)}`;
      }
      if (p && stateLine) stateLine.textContent = `${p.app} on ${p.from} · ${p.state}`;
      return;
    }
    mediaKey = key;
    progress = null;
    clock = null;
    stateLine = null;
    const parts: Node[] = [];
    if (p) {
      const can = (a: string) => p.actions.includes(a);
      const playPause = can("toggle") || can("play") || can("pause");
      const art = p.art ? [el("img", { className: "art", src: p.art, alt: "" })] : [];
      stateLine = el("span", { className: "hint small" }, `${p.app} on ${p.from} · ${p.state}`);
      const controls = el(
        "div",
        { className: "row left" },
        control("⏮", () => command("prev"), can("prev")),
        control(p.state === "playing" ? "⏸" : "▶", () => command("toggle"), playPause),
        control("⏭", () => command("next"), can("next")),
      );
      parts.push(
        el("div", { className: "mediacard" }, ...art, el("div", { className: "text" }, el("strong", {}, p.title), el("span", {}, p.artist ?? ""), stateLine), controls),
      );
      if (can("seek") && p.durationMs && p.positionMs !== null) {
        progress = slider(p.positionMs, p.durationMs, (v) => command("seek", v), "Position");
        clock = el("span", { className: "hint small" }, `${time(p.positionMs)} / ${time(p.durationMs)}`);
        parts.push(el("div", { className: "row left" }, progress, clock));
      }
      if (p.volume) parts.push(el("label", { className: "hint small" }, "Phone volume ", slider(p.volume.level, 100, (v) => command("volume", v), "Phone volume")));
    }
    if (m.pc) parts.push(el("p", { className: "hint small" }, `PC media → phone: ${m.pc.title}${m.pc.artist ? ` — ${m.pc.artist}` : ""} (${m.pc.state})`));
    if (m.volume) parts.push(el("label", { className: "hint small" }, "PC volume ", slider(m.volume.level, 100, (v) => command("pc-volume", v), "PC volume")));
    media.replaceChildren(...parts);
    media.hidden = parts.length === 0;
  };

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
    drawMedia(view.media);
    drawStrip(view.presence);
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
  void listen("media-changed", () => void refresh());
  // A transfer's numbers move all the time and a change of state is announced; this keeps the bars moving smoothly.
  window.setInterval(() => void refresh(), 700);
  void refresh();
}
