// The debug window: the first-try success rate of sends (targets.md #1), recent errors with their codes, latency (ping round trip,
// tap to ack), the connection history, the network profile of this PC (Private or Public), the queue, and "Export logs" (a zip of the log
// files and a redacted status.json, to a place the user chooses). The numbers come from src-tauri/src/metrics.rs.
import { invoke } from "@tauri-apps/api/core";
import { button, el, errorText, flash, when } from "./dom";

type Rate = { settled: number; firstTryOk: number; percent: number | null };
type Latency = { n: number; lastMs: number; medianMs: number; p95Ms: number } | null;
type View = {
  windowDays: number;
  successRate: { firstTry: Rate; firstTryWhileConnected: Rate; byKind: Record<string, Rate>; target: number };
  latency: { tapToAck: Latency; ping: Latency };
  errors: { at: number; kind: string; code: string | null; id: string; result: string; attempts: number }[];
  connections: { at: number; event: string; detail: string }[];
  network: { networks: { name: string; category: string }[]; error?: string };
  queue: { count: number; items: { id: string; kind: string; createdMs: number; expiresMs: number | null; sentMs: number | null }[] };
  state: string;
  version: string;
  port: number;
};

const KIND: Record<string, string> = { text: "Text", clipboard: "Clipboard", offer: "Files to phone", upload: "Files from phone" };

function percent(r: Rate): string {
  return r.percent === null ? "no sends yet" : `${r.percent}%`;
}

function latency(label: string, l: Latency): HTMLElement {
  return el("tr", {}, el("th", {}, label), ...(l ? [`${l.lastMs} ms`, `${l.medianMs} ms`, `${l.p95Ms} ms`, String(l.n)] : ["", "", "", "no data"]).map((t) => el("td", {}, t)));
}

export function startDebugWindow(root: HTMLElement): void {
  root.className = "debug";
  const note = el("p", { className: "flash" });
  const body = el("div", {});
  root.append(el("h1", {}, "Debug"), body, note);

  const draw = (v: View) => {
    const rate = v.successRate.firstTry;
    const meets = rate.percent !== null && rate.percent >= v.successRate.target;
    const kinds = Object.entries(v.successRate.byKind);
    const profile = v.network.networks.length
      ? v.network.networks.map((n) => el("li", {}, `${n.name || "(unnamed)"}: `, el("strong", { className: n.category === "Public" ? "warn" : "" }, n.category)))
      : [el("li", {}, v.network.error ? `Not available (${v.network.error})` : "No connected network found")];
    const publicNet = v.network.networks.some((n) => n.category === "Public");

    const ping = el("span", { className: "hint" });
    body.replaceChildren(
      el("section", {},
        el("h2", {}, "Sends that worked at the first try"),
        el("p", { className: `big ${rate.percent === null ? "" : meets ? "good" : "warn"}` }, percent(rate)),
        el("p", { className: "hint" }, `${rate.firstTryOk} of ${rate.settled} sends in the last ${v.windowDays} days. Target: at least ${v.successRate.target}%. While the phone was connected: ${percent(v.successRate.firstTryWhileConnected)} (${v.successRate.firstTryWhileConnected.firstTryOk} of ${v.successRate.firstTryWhileConnected.settled}).`),
        kinds.length
          ? el("table", {}, el("tbody", {}, ...kinds.map(([k, r]) => el("tr", {}, el("th", {}, KIND[k] ?? k), el("td", {}, percent(r)), el("td", { className: "hint" }, `${r.firstTryOk} of ${r.settled}`)))))
          : "",
      ),
      el("section", {},
        el("h2", {}, "Latency"),
        el("table", {}, el("thead", {}, el("tr", {}, el("th", {}, ""), el("th", {}, "Last"), el("th", {}, "Median"), el("th", {}, "95th"), el("th", {}, "Count"))), el("tbody", {}, latency("Tap to ack", v.latency.tapToAck), latency("Ping round trip", v.latency.ping))),
        el("div", { className: "row left" }, button("Ping the phone", () => {
          ping.textContent = "Pinging…";
          void invoke<string>("debug_ping").then((t) => { ping.textContent = t; }).catch((e) => { ping.textContent = String(e); });
        }, "small"), ping),
      ),
      el("section", {},
        el("h2", {}, "Connection"),
        el("p", {}, `State: ${v.state.replace(/_/g, " ")}. Listening on port ${v.port}. ${v.queue.count ? `${v.queue.count} send${v.queue.count === 1 ? "" : "s"} waiting in the queue.` : "Nothing is waiting in the queue."}`),
        el("p", { className: "hint" }, "Network profile of this PC:"),
        el("ul", {}, ...profile),
        publicNet ? el("p", { className: "hint warn" }, "A Public network is the usual reason a phone can't connect: Windows may be blocking Kin on it. The installer's firewall rule allows Private and Public.") : "",
      ),
      el("section", {},
        el("h2", {}, "Recent errors"),
        v.errors.length
          ? el("ul", { className: "log" }, ...v.errors.map((e) => el("li", {}, `${when(e.at)} · ${KIND[e.kind] ?? e.kind} · `, el("code", {}, errorText(e.code) || "error"), ` · ${e.result}, ${e.attempts} attempt${e.attempts === 1 ? "" : "s"} · ${e.id}`)))
          : el("p", { className: "hint" }, "No errors."),
      ),
      el("section", {},
        el("h2", {}, "Connection history"),
        v.connections.length
          ? el("ul", { className: "log" }, ...v.connections.map((c) => el("li", {}, `${when(c.at)} · `, el("strong", {}, c.event), ` · ${c.detail}`)))
          : el("p", { className: "hint" }, "Nothing yet."),
      ),
      el("div", { className: "row left" }, button("Export logs…", () => {
        void invoke<string | null>("export_logs").then((path) => { if (path) flash(note, `Saved ${path}`); }).catch((e) => flash(note, String(e), true));
      }), el("span", { className: "hint" }, `Kin ${v.version}. The zip has the log files and a status file with your paths, addresses and names taken out.`)),
    );
  };

  let busy = false;
  const refresh = async () => {
    if (busy) return;
    busy = true;
    try {
      draw(await invoke<View>("debug_view"));
    } catch (e) {
      flash(note, String(e), true);
    } finally {
      busy = false;
    }
  };
  // Keep the page still while a button is being used: a redraw every few seconds would replace the ping text.
  window.setInterval(() => {
    if (document.activeElement === document.body || !document.activeElement) void refresh();
  }, 4000);
  void refresh();
}
