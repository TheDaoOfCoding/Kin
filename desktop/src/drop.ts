// The Home page (the drop window, which the tray's left click opens): your phone and its state, the Send menu (a typed text, or files from the file
// dialog), a target for files dragged from Explorer, the recent transfers with a Cancel button, the phone's media, Ring my phone and Pause. The backend
// (src-tauri/src/transfers.rs, `Core::drop_view`) owns the state and does the sending. This only draws it, and passes on what was dropped.
//
// Windows doesn't support dropping files on a tray icon, which is why this window exists. The paths of the dropped files come
// from Tauri's own drag-and-drop event (a web page is never told the path of a file it is given).
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { actionButton, el, errorText, flash, sensitiveWhilePausedText, size } from "./dom";
import { boxArt, icon, mediaArt, phoneArt } from "./icons";
import { mountShell } from "./shell";

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
    /** 2B.5: "Do Not Disturb" while the phone says it is in it (a short word, memory only). */
    dnd?: string | null;
  };
  ring: { can: boolean; active: boolean; text: string | null; reason: string | null };
  showPhone: boolean;
};
type View = { phone: string | null; connected: boolean; active: Transfer[]; recent: Transfer[]; folder: string; media: Media; presence: Presence; pcName: string; version: string };

function describe(t: Transfer): string {
  if (t.status === "active") {
    const speed = t.speedBps && t.speedBps > 0 ? ` · ${size(t.speedBps)}/s` : "";
    // A batch moves several files at once (up to 4): say how far it is and how many are going now.
    const files = t.files > 1 ? `${t.filesDone ?? 0} of ${t.files} files · ` : "";
    const parallel = t.filesActive && t.filesActive > 1 ? ` · ${t.filesActive} at once` : "";
    return `${files}${size(t.bytesDone)} of ${size(t.totalBytes)}${speed}${parallel}`;
  }
  return `${size(t.totalBytes)}${t.error && t.status !== "done" ? ` · ${errorText(t.error)}` : ""}`;
}

export function startDropWindow(root: HTMLElement): void {
  const shell = mountShell(root, "drop");
  const page = document.documentElement;
  const note = el("p", { className: "flash" });
  let last: View | null = null;

  // ----- the page: the phone, the media, the recent transfers, and the drop zone
  const kicker = el("div", { className: "kicker" }, "Your phone");
  const phoneName = el("div", { className: "dname" });
  const pillText = el("span", {});
  const pill = el("span", { className: "spill" }, el("span", { className: "dot" }), pillText);
  const chips = el("div", { className: "chips" });
  const heroHint = el("div", { className: "hint small" });
  const hero = el("section", { className: "hero phero" }, el("div", { className: "txt" }, kicker, phoneName, pill, chips, heroHint), phoneArt());
  const media = el("section", { className: "card media" });
  const list = el("div", { className: "plist" });
  const seeAll = el("a", { href: "#" }, "See all");
  seeAll.addEventListener("click", (e) => {
    e.preventDefault();
    void invoke("open_window", { name: "history" });
  });
  const recent = el("section", { className: "card precent" }, el("div", { className: "sec-head" }, el("b", {}, "Recent transfers"), seeAll), list);
  const zoneTitle = el("h3", {}, "Drop files here");
  const zoneText = el("p", {});
  const zoneFolder = el("div", { className: "alt" });
  const choose = () => void invoke("pick_files");
  const zone = el(
    "div",
    { className: "drop", tabIndex: 0, role: "button" as string },
    boxArt(),
    zoneTitle,
    zoneText,
    el("span", { className: "btn soft" }, icon("file"), "Choose files…"),
    zoneFolder,
  );
  zone.addEventListener("click", choose);
  zone.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") choose();
  });
  shell.body.append(note, el("div", { className: "cols" }, el("div", { className: "lcol" }, hero, media, recent), zone));

  // ----- the actions: Ring phone, Pause, Resume and the Send menu
  const act = async (command: string, args: Record<string, unknown>) => {
    try {
      const line = await invoke<string | null>(command, args);
      if (line) flash(note, String(line));
      else note.textContent = "";
    } catch (e) {
      flash(note, String(e), true);
    }
    void refresh();
  };
  const ringButton = actionButton("ring", "Ring phone", () => void act("ring_phone", { start: !last?.presence.ring.active }));
  const resumeButton = actionButton("play", "Resume", () => void act("set_pause", { choice: "resume" }), "warn");
  resumeButton.querySelector("svg")?.classList.add("solid");
  const pauseButton = actionButton("pause", "Pause", () => toggleMenu(pauseWrap, "Pause Kin", [
    { icon: "pause", tint: "honey", label: "For 1 hour", run: () => void act("set_pause", { choice: "1h" }) },
    { icon: "pause", tint: "honey", label: "Until tomorrow", run: () => void act("set_pause", { choice: "tomorrow" }) },
    { icon: "pause", tint: "honey", label: "Until I resume", run: () => void act("set_pause", { choice: "resumed" }) },
  ]));
  const sendButton = actionButton("send", "Send", () => toggleMenu(sendWrap, last?.phone ? `Send to ${last.phone}` : "Send", [
    { icon: "chat", tint: "lilac", label: "Text", hint: "A note or a link", run: () => compose() },
    { icon: "file", tint: "sage", label: "File", hint: "Pick files on this PC", run: choose },
  ]), "primary");
  sendButton.append(icon("chev-d", "s16"));
  const pauseWrap = el("div", { className: "menuwrap" }, pauseButton);
  const sendWrap = el("div", { className: "menuwrap" }, sendButton);
  shell.acts.append(ringButton, pauseWrap, resumeButton, sendWrap);

  let openMenu: HTMLElement | null = null;
  const closeMenu = () => {
    openMenu?.remove();
    openMenu = null;
  };
  type MenuItem = { icon: string; tint: string; label: string; hint?: string; run: () => void };
  const toggleMenu = (wrap: HTMLElement, heading: string, items: MenuItem[]) => {
    const was = openMenu?.parentElement === wrap;
    closeMenu();
    if (was) return;
    const menu = el("div", { className: "menu", role: "menu" }, el("div", { className: "menu-h" }, heading));
    for (const item of items) {
      const row = el(
        "button",
        { type: "button", className: `mi ${item.tint}`, role: "menuitem" },
        el("span", { className: "slot" }, icon(item.icon)),
        el("span", {}, el("b", {}, item.label), ...(item.hint ? [el("small", {}, item.hint)] : [])),
      );
      row.addEventListener("click", () => {
        closeMenu();
        item.run();
      });
      menu.append(row);
    }
    wrap.append(menu);
    openMenu = menu;
  };
  document.addEventListener("pointerdown", (e) => {
    if (openMenu && !(e.target as HTMLElement).closest(".menuwrap")) closeMenu();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeMenu();
  });

  // ----- the Text box of the Send menu
  const compose = () => {
    const phone = last?.phone ?? null;
    const text = el("textarea", { rows: 5, placeholder: "Type a note or paste a link", ariaLabel: "Text to send" });
    const count = el("span", { className: "count" }, "0 characters");
    const problem = el("p", { className: "flash bad" });
    const cancel = actionButton(null, "Cancel", () => close(), "plain");
    const send = actionButton("send", "Send", () => void submit(), "primary");
    const close = () => {
      scrim.remove();
      document.removeEventListener("keydown", onKey);
    };
    const submit = async () => {
      if (!text.value.trim() || send.disabled) return;
      send.disabled = true;
      problem.textContent = "";
      try {
        const line = await invoke<string>("send_text", { text: text.value });
        close();
        flash(note, line);
        void refresh();
      } catch (e) {
        problem.textContent = String(e);
        send.disabled = false;
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) void submit();
    };
    text.addEventListener("input", () => {
      const n = [...text.value].length;
      count.textContent = `${n} character${n === 1 ? "" : "s"}`;
    });
    const dialog = el(
      "div",
      { className: "dialog", role: "dialog", ariaLabel: "Send a text" },
      el("div", { className: "sheet-head" }, el("div", {}, el("h3", {}, "Send a text"), el("div", { className: "to" }, el("span", { className: "dot" }), phone ? `To ${phone}` : "No phone is paired yet"))),
      el("div", { className: "field" }, text, count),
      problem,
      el("div", { className: "sheet-actions" }, cancel, send),
    );
    const scrim = el("div", { className: "scrim" }, dialog);
    scrim.addEventListener("pointerdown", (e) => {
      if (e.target === scrim) close();
    });
    document.addEventListener("keydown", onKey);
    document.body.append(scrim);
    text.focus();
  };

  // ----- drawing
  const chip = (...children: (Node | string)[]) => el("span", { className: "chip on-chip" }, ...children);

  const drawHero = (v: View) => {
    const p = v.presence;
    const name = p.phone.name ?? v.phone;
    phoneName.textContent = name ?? "No phone yet";
    const paused = p.pause.on;
    // The face of the phone and the dot follow one word, as in the design: connected, connecting (looking for the phone) or paused.
    page.dataset.state = paused ? "paused" : v.phone && v.connected ? "connected" : "connecting";
    pill.hidden = !name;
    pillText.textContent = paused ? `Paused ${p.pause.text ?? ""}`.trim() : v.connected ? "Connected" : "Looking for your phone…";
    const kids: HTMLElement[] = [];
    const bat = p.phone.connected ? p.phone.battery : p.phone.lastKnown;
    if (name && p.showPhone && bat) {
      const glyph = el("span", { className: "bglyph" }, el("i", {}));
      glyph.firstElementChild?.setAttribute("style", `width:${Math.max(4, Math.min(100, bat.level))}%`);
      kids.push(chip(glyph, `${bat.level}%${bat.charging ? " charging" : ""}${p.phone.connected ? "" : " (last known)"}`));
    }
    if (p.phone.connected && p.phone.paused) kids.push(chip(icon("pause"), `${name} paused ${p.phone.pausedText ?? ""}`.trim()));
    // 2B.5: "Phone: Do Not Disturb" while the connected phone says so (and "Show my phone's battery here" is on: the same switch shows what the phone shares).
    if (p.phone.connected && p.showPhone && p.phone.dnd) kids.push(chip(icon("bell"), `Phone: ${p.phone.dnd}`));
    chips.replaceChildren(...kids);
    const lines: string[] = [];
    if (!name) lines.push("No phone is paired yet. Use Pair a phone on the left.");
    else if (!p.ring.active && !p.ring.can && p.ring.reason) lines.push(p.ring.reason);
    if (paused && p.pause.quietArrivals) lines.push(`${p.pause.quietArrivals} received while paused`);
    if (paused && p.pause.quietSensitive) lines.push(sensitiveWhilePausedText(p.pause.quietSensitive));
    if (p.ring.active && p.ring.text) lines.push(p.ring.text);
    heroHint.replaceChildren(...lines.map((l) => el("div", {}, l)));
    heroHint.hidden = lines.length === 0;

    shell.sub.textContent = name ? `${name} and this PC, side by side.` : "Pair a phone to start sending.";
    ringButton.replaceChildren(icon("ring"), p.ring.active ? "Stop ringing" : "Ring phone");
    ringButton.disabled = !name || (!p.ring.active && !p.ring.can);
    ringButton.title = !p.ring.active && !p.ring.can ? (p.ring.reason ?? "") : "";
    ringButton.hidden = !name;
    pauseButton.hidden = !name || paused;
    resumeButton.hidden = !name || !paused;
    if (paused) closeMenu();
  };

  // ----- the media card
  const command = (action: string, value?: number) => void invoke("media_command", { action, value: value ?? null });
  const time = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
  let dragging = false;
  let mediaKey = "";
  let progress: HTMLInputElement | null = null;
  let clock: HTMLElement | null = null;
  let stateLine: HTMLElement | null = null;

  /** A round icon button of the media card, enabled only if the phone's player offers it. */
  const control = (name: string, label: string, onClick: () => void, enabled: boolean, className = ""): HTMLButtonElement => {
    const b = el("button", { type: "button", className: `iconbtn ${className}`, title: label, ariaLabel: label }, icon(name));
    b.disabled = !enabled;
    b.addEventListener("click", onClick);
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
      const cover = p.art ? el("img", { className: "cover", src: p.art, alt: "" }) : mediaArt();
      stateLine = el("small", { className: "hint" }, `${p.app} on ${p.from} · ${p.state}`);
      const controls = el(
        "div",
        { className: "ctrls" },
        control("prev", "Previous", () => command("prev"), can("prev")),
        control(p.state === "playing" ? "pause" : "play", p.state === "playing" ? "Pause" : "Play", () => command("toggle"), playPause, "pp"),
        control("next", "Next", () => command("next"), can("next")),
      );
      parts.push(el("div", { className: "mediacard" }, cover, el("div", { className: "text" }, el("b", {}, p.title), el("span", { className: "by" }, p.artist ?? ""), stateLine), controls));
      if (can("seek") && p.durationMs && p.positionMs !== null) {
        progress = slider(p.positionMs, p.durationMs, (v) => command("seek", v), "Position");
        clock = el("span", { className: "hint small" }, `${time(p.positionMs)} / ${time(p.durationMs)}`);
        parts.push(el("div", { className: "seekrow" }, progress, clock));
      }
      if (p.volume) parts.push(el("label", { className: "seekrow" }, el("span", { className: "hint small" }, "Phone volume"), slider(p.volume.level, 100, (v) => command("volume", v), "Phone volume")));
    }
    if (m.pc) parts.push(el("p", { className: "hint small" }, `PC media → phone: ${m.pc.title}${m.pc.artist ? ` · ${m.pc.artist}` : ""} (${m.pc.state})`));
    if (m.volume) parts.push(el("label", { className: "seekrow" }, el("span", { className: "hint small" }, "PC volume"), slider(m.volume.level, 100, (v) => command("pc-volume", v), "PC volume")));
    media.replaceChildren(...parts);
    media.hidden = parts.length === 0;
  };

  // ----- the recent transfers
  const row = (t: Transfer): HTMLElement => {
    const out = t.direction === "out";
    const what = t.files === 1 ? (t.names[0] ?? "1 file") : `${t.files} files`;
    const failed = t.status === "failed";
    const percent = t.totalBytes > 0 ? Math.min(100, Math.round((t.bytesDone / t.totalBytes) * 100)) : 0;
    const small = el("small", failed ? { className: "err-t" } : {}, icon(out ? "up" : "down"), `${out ? "To" : "From"} ${t.peer} · ${describe(t)}`);
    const grow = el("div", { className: "grow" }, el("b", { title: what }, what), small);
    const trail: Node[] = [];
    if (t.status === "active") {
      const bar = el("i", {});
      bar.style.width = `${percent}%`;
      grow.append(el("div", { className: "prog" }, bar));
      const ring = el("span", { className: "ring" });
      ring.style.setProperty("--p", `${percent}%`);
      trail.push(el("span", { className: "chip warn" }, ring, `${out ? "Sending" : "Receiving"} ${percent}%`));
      if (t.cancelable !== false) {
        const cancel = actionButton(null, "Cancel", () => {
          cancel.disabled = true;
          void invoke("cancel_transfer", { id: t.id });
        }, "plain sm");
        trail.push(cancel);
      }
    } else if (t.status === "done") trail.push(el("span", { className: "chip ok" }, icon("check"), out ? "Sent" : "Received"));
    else if (failed) trail.push(el("span", { className: "chip warn" }, "Failed"));
    else trail.push(el("span", { className: "chip neutral" }, "Canceled"));
    return el("div", { className: `prow ${t.status}` }, el("span", { className: `av ${failed ? "err" : "sage"}` }, icon("file")), grow, ...trail);
  };

  const draw = (view: View) => {
    last = view;
    shell.setPc(view.pcName, view.version);
    drawHero(view);
    zoneText.textContent = !view.phone
      ? "No phone is paired yet."
      : view.connected
        ? `They go to ${view.phone} over your Wi-Fi.`
        : `${view.phone} isn't connected. Files wait and go out when it is back.`;
    zoneFolder.textContent = `Files from your phone go to ${view.folder}`;
    const rows = [...view.active, ...view.recent.filter((t) => !view.active.some((a) => a.id === t.id)).slice(0, 5)];
    list.replaceChildren(...(rows.length ? rows.map(row) : [el("p", { className: "empty small" }, "Nothing sent or received yet.")]));
    drawMedia(view.media);
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
