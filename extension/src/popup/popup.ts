// The toolbar popup. It holds no logic of its own: the background does the work and the popup draws.
import type { HistoryEntry } from "../shared/history";
import type { DeviceInfo, StatusInfo } from "../shared/protocol";

type Ext = typeof chrome;
const g = globalThis as unknown as { browser?: Ext; chrome?: Ext };
const ext = (g.browser ?? g.chrome) as Ext;

interface View {
  ok: boolean;
  status?: StatusInfo;
  code?: string;
  message?: string;
}
interface Outcome {
  outcome: "sent" | "queued" | "failed";
  message: string;
  code?: string;
}
interface DownloadAlerts {
  enabled: boolean;
  longOnly: boolean;
  /** The browser has given the extension the `downloads` permission. */
  granted: boolean;
  /** The browser has a downloads API at all (Firefox for Android hasn't): when it hasn't, the whole section is hidden. */
  supported?: boolean;
}
interface State {
  view: View;
  selectedDeviceId?: string;
  history: HistoryEntry[];
  lastResult: { at: number; outcome: Outcome["outcome"]; message: string } | null;
  downloadAlerts?: DownloadAlerts;
}

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;
const send = <T>(msg: Record<string, unknown>): Promise<T> => ext.runtime.sendMessage(msg) as Promise<T>;

let selected: string | undefined;
let devices: DeviceInfo[] = [];

function setHidden(el: HTMLElement, hidden: boolean): void {
  el.hidden = hidden;
}

function ago(at: number): string {
  const s = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return new Date(at).toLocaleDateString();
}

function renderDevices(): void {
  const list = $("devices");
  list.replaceChildren();
  for (const d of devices) {
    const li = document.createElement("li");
    const dot = document.createElement("span");
    dot.className = "dot" + (d.online ? " online" : "");
    li.append(dot);
    const label = document.createElement("label");
    if (devices.length > 1) {
      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = "device";
      radio.value = d.id;
      radio.checked = d.id === selected;
      radio.addEventListener("change", () => {
        selected = d.id;
        void send({ kin: "select", deviceId: d.id });
      });
      label.append(radio);
    }
    const name = document.createElement("span");
    name.textContent = d.name;
    const sub = document.createElement("span");
    sub.className = "sub";
    const parts = [d.online ? "online" : "offline, will queue"];
    if (d.queuedCount > 0) parts.push(`${d.queuedCount} waiting`);
    sub.textContent = parts.join(" · ");
    label.append(name, sub);
    li.append(label);
    list.append(li);
  }
}

function renderHistory(history: HistoryEntry[]): void {
  const list = $("history");
  list.replaceChildren();
  for (const h of history) {
    const li = document.createElement("li");
    const what = document.createElement("span");
    what.className = "what";
    what.textContent = h.label;
    what.title = h.outcome === "failed" && h.message ? `${h.label}: ${h.message}` : h.label;
    const meta = document.createElement("span");
    meta.className = `meta badge-${h.outcome}`;
    const word = h.outcome === "sent" ? "sent" : h.outcome === "queued" ? "queued" : "failed";
    meta.textContent = `${word}${h.device ? " · " + h.device : ""} · ${ago(h.at)}`;
    li.append(what, meta);
    list.append(li);
  }
  setHidden($("history-empty"), history.length > 0);
  setHidden($("clear"), history.length === 0);
}

function showResult(o: { outcome: Outcome["outcome"]; message: string } | null): void {
  const el = $("result");
  if (!o) {
    setHidden(el, true);
    return;
  }
  el.className = `result ${o.outcome}`;
  el.textContent = o.message;
  setHidden(el, false);
}

function renderDownloads(d: DownloadAlerts | undefined): void {
  setHidden($("dl-section"), d?.supported === false);
  if (d?.supported === false) return;
  const on = $<HTMLInputElement>("dl-enabled");
  const long = $<HTMLInputElement>("dl-long");
  const note = $("dl-note");
  const active = !!d && d.enabled && d.granted;
  on.checked = active;
  long.checked = d?.longOnly ?? true;
  long.disabled = !active;
  note.classList.toggle("warn", !!d && d.enabled && !d.granted);
  if (d && d.enabled && !d.granted) note.textContent = "Needs permission: turn it on to let Kin see when downloads finish.";
  else if (active) note.textContent = "Kin sends only the file name, never the address, and nothing from private windows.";
  else note.textContent = "Off. Turning it on asks your browser for access to your downloads.";
}

function render(s: State): void {
  const stateEl = $("state");
  const problem = $("problem");
  const text = $("problem-text");
  const startBtn = $<HTMLButtonElement>("start");
  const openBtn = $<HTMLButtonElement>("open");
  const sendBtn = $<HTMLButtonElement>("send-tab");
  selected = s.selectedDeviceId;
  devices = s.view.status?.devices ?? [];
  if (!selected || !devices.some((d) => d.id === selected)) selected = s.view.status?.defaultDeviceId ?? devices[0]?.id;

  setHidden(problem, true);
  setHidden(startBtn, true);
  setHidden(openBtn, true);
  setHidden($("devices-section"), true);
  sendBtn.disabled = false;
  stateEl.className = "state";

  if (!s.view.ok) {
    stateEl.textContent = "Kin unavailable";
    stateEl.classList.add("bad");
    text.textContent = s.view.message ?? "Can't reach Kin.";
    setHidden(problem, false);
    // Only a host that exists but can't find Kin gets the button.
    setHidden(startBtn, s.view.code !== "KIN_NOT_RUNNING");
    sendBtn.disabled = s.view.code === "HOST_NOT_INSTALLED" || s.view.code === "HOST_FORBIDDEN";
  } else if (!s.view.status?.running) {
    stateEl.textContent = "Kin isn't running";
    stateEl.classList.add("bad");
    text.textContent = "Kin isn't running. Start Kin to send things to your phone.";
    setHidden(problem, false);
    setHidden(startBtn, false);
    // "Send this tab" still works: the helper starts Kin and sends.
  } else if (devices.length === 0) {
    stateEl.textContent = "No phone paired";
    stateEl.classList.add("bad");
    text.textContent = "Pair your phone with Kin first.";
    setHidden(problem, false);
    setHidden(openBtn, false);
    sendBtn.disabled = true;
  } else {
    stateEl.textContent = s.view.status.paused ? "Kin is paused" : "Kin is running";
    stateEl.classList.add("ok");
    setHidden($("devices-section"), false);
    renderDevices();
  }
  renderHistory(s.history);
  renderDownloads(s.downloadAlerts);
  showResult(s.lastResult && Date.now() - s.lastResult.at < 60_000 ? s.lastResult : null);
}

async function load(): Promise<void> {
  try {
    render(await send<State>({ kin: "state" }));
  } catch {
    $("state").textContent = "Kin unavailable";
  }
}

$("send-tab").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("send-tab");
  btn.disabled = true;
  btn.textContent = "Sending…";
  try {
    const r = await send<{ outcome: Outcome }>({ kin: "sendTab", ...(selected ? { deviceId: selected } : {}) });
    await load();
    showResult(r.outcome);
  } finally {
    btn.textContent = "Send this tab";
    btn.disabled = false;
  }
});

$("start").addEventListener("click", async () => {
  $("start").setAttribute("disabled", "");
  $("state").textContent = "Starting Kin…";
  await send({ kin: "start" });
  $("start").removeAttribute("disabled");
  await load();
});

$("open").addEventListener("click", async () => {
  await send({ kin: "open", target: "pairing" });
});

$("dl-enabled").addEventListener("change", () => {
  if ($<HTMLInputElement>("dl-enabled").checked) {
    // The request has to start inside the click, before any await (Firefox insists on it).
    const asked = ext.permissions.request({ permissions: ["downloads"] });
    // The wish is noted at once: if the browser's prompt takes the popup down, the permission arriving still turns the alerts on.
    const wish = send({ kin: "downloadAlerts", enabled: true });
    void asked
      .catch(() => false)
      .then(async (granted) => {
        await wish.catch(() => undefined);
        if (!granted) await send({ kin: "downloadAlerts", enabled: false });
        await load();
      });
  } else {
    void send({ kin: "downloadAlerts", enabled: false }).then(load);
  }
});

$("dl-long").addEventListener("change", () => {
  void send({ kin: "downloadAlerts", longOnly: $<HTMLInputElement>("dl-long").checked }).then(load);
});

$("clear").addEventListener("click", async () => {
  await send({ kin: "clear" });
  await load();
});

void load();
