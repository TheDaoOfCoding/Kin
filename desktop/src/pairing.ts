// The pairing window: the QR code, the PC name and short ID, the 5-minute countdown, and the "New device paired" notice with
// a button to remove the device. The backend (src-tauri/src/core.rs, `PairingView`) owns the state. This only draws it.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Device = { deviceId: string; name: string; platform: string };
type View = {
  status: "waiting" | "expired" | "paired" | "removed" | "no_network";
  svg: string | null;
  pcName: string;
  shortId: string;
  expiresAtMs: number;
  device: Device | null;
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...children: (Node | string)[]): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function button(label: string, onClick: () => void, secondary = false): HTMLButtonElement {
  const b = el("button", { type: "button", className: secondary ? "secondary" : "" }, label);
  b.addEventListener("click", onClick);
  return b;
}

// The QR code comes from our own backend as an SVG string. Parse it as SVG instead of assigning innerHTML.
function svgNode(svg: string): Node {
  const doc = new DOMParser().parseFromString(svg, "image/svg+xml");
  return document.importNode(doc.documentElement, true);
}

function countdown(ms: number): string {
  const s = Math.max(0, Math.ceil(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function startPairingWindow(root: HTMLElement): void {
  let timer: number | undefined;

  const draw = (view: View) => {
    window.clearInterval(timer);
    root.replaceChildren();

    if ((view.status === "paired" || view.status === "removed") && view.device) {
      const d = view.device;
      const removed = view.status === "removed";
      root.append(
        el("h1", {}, removed ? "Device removed" : "New device paired"),
        el("p", { className: "big" }, d.name),
        el("p", {}, removed ? "It can no longer connect to this PC." : `${d.platform === "android" ? "Android phone" : d.platform} paired with ${view.pcName}.`),
      );
      const done = button("Done", () => void invoke("close_window"));
      root.append(
        removed
          ? el("div", { className: "row" }, done)
          : el("p", { className: "hint" }, "Didn't you scan the code? Someone else may have. Remove the device."),
      );
      if (!removed) {
        // The backend emits "pairing-changed" after the removal, which redraws this as "Device removed".
        root.append(el("div", { className: "row" }, button("Remove this device", () => void invoke("remove_device", { deviceId: d.deviceId }), true), done));
      }
      return;
    }

    root.append(el("h1", {}, "Pair new phone"));
    if (view.status === "no_network") {
      root.append(
        el("p", {}, "No network address found. Connect this PC to Wi-Fi or Ethernet, then try again."),
        el("div", { className: "row" }, button("Try again", () => void renew())),
      );
      return;
    }
    if (view.status === "expired") {
      root.append(
        el("p", {}, "This code has expired."),
        el("div", { className: "row" }, button("Show a new code", () => void renew())),
      );
      return;
    }

    root.append(
      el("p", {}, "Open Kin on your phone and scan this code."),
      el("div", { className: "qr" }, view.svg ? svgNode(view.svg) : ""),
      el("p", { className: "id" }, el("strong", {}, view.pcName), ` · ${view.shortId}`),
    );
    const left = el("p", { className: "hint" });
    root.append(left);
    const tick = () => {
      const remaining = view.expiresAtMs - Date.now();
      left.textContent = `Valid for ${countdown(remaining)}`;
      if (remaining <= 0) void refresh();
    };
    tick();
    timer = window.setInterval(tick, 1000);
  };

  const refresh = async () => draw(await invoke<View>("pairing_view"));
  const renew = async () => draw(await invoke<View>("new_pairing"));

  void listen("pairing-changed", () => void refresh());
  void refresh();
}
