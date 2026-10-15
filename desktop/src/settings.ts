// The settings window: device name, automatic clipboard sync, images to the clipboard, the received folder, the hotkey (with conflict
// detection: if another program has the key, the old one stays and the window says so), start with Windows, and unpair.
// The backend (src-tauri/src/prefs.rs, app.rs) saves each change at once and applies it.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { button, el, flash } from "./dom";

type Device = { deviceId: string; name: string; platform: string; connected: boolean };
type View = {
  deviceName: string;
  customDeviceName: string | null;
  defaultDeviceName: string;
  autoClipboardSync: boolean;
  imagesToClipboard: boolean;
  receivedFolder: string;
  receivedFolderIsDefault: boolean;
  hotkey: { accelerator: string; registered: boolean; error: string | null };
  defaultHotkey: string;
  startWithWindows: boolean | null;
  startWithWindowsIsReal: boolean;
  devices: Device[];
  version: string;
};

/** The key part of an accelerator ("K", "F9", "Space") from a keyboard event, or null for a key that can't be a hotkey on its own. */
export function accelKey(code: string): string | null {
  let m = /^Key([A-Z])$/.exec(code);
  if (m) return m[1];
  m = /^Digit([0-9])$/.exec(code);
  if (m) return m[1];
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  const names: Record<string, string> = {
    Space: "Space", Enter: "Enter", Tab: "Tab", Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown", Insert: "Insert", Delete: "Delete",
    ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
  };
  return names[code] ?? null;
}

/** "Ctrl+Alt+Shift+K" from a keydown event, or null while only modifiers are down. */
export function accelerator(e: KeyboardEvent): string | null {
  const key = accelKey(e.code);
  if (!key) return null;
  const mods = [e.ctrlKey ? "Ctrl" : "", e.altKey ? "Alt" : "", e.shiftKey ? "Shift" : "", e.metaKey ? "Super" : ""].filter(Boolean);
  return [...mods, key].join("+");
}

function section(heading: string, ...children: (Node | string)[]): HTMLElement {
  return el("section", {}, el("h2", {}, heading), ...children);
}

export function startSettingsWindow(root: HTMLElement): void {
  root.className = "list settings";
  const note = el("p", { className: "flash" });
  const body = el("div", {});
  root.append(el("h1", {}, "Settings"), body, note);

  const call = async (command: string, args: Record<string, unknown> = {}): Promise<boolean> => {
    try {
      await invoke(command, args);
      return true;
    } catch (e) {
      flash(note, String(e), true);
      return false;
    }
  };

  const toggle = (label: string, hint: string, value: boolean, onChange: (on: boolean) => void): HTMLElement => {
    const box = el("input", { type: "checkbox", checked: value });
    box.addEventListener("change", () => onChange(box.checked));
    return el("label", { className: "toggle" }, box, el("span", {}, el("strong", {}, label), el("small", {}, hint)));
  };

  const draw = (v: View) => {
    // device name
    const name = el("input", { type: "text", value: v.customDeviceName ?? "", placeholder: v.defaultDeviceName, maxLength: 64 });
    const saveName = async () => {
      if (name.value.trim() === (v.customDeviceName ?? "")) return;
      const ok = await call("set_device_name", { name: name.value });
      if (ok) flash(note, "Name saved. The phone is told now.");
      void refresh();
    };
    name.addEventListener("change", () => void saveName());
    name.addEventListener("keydown", (e) => {
      if (e.key === "Enter") void saveName();
    });

    // hotkey
    const key = el("input", { type: "text", readOnly: true, value: v.hotkey.accelerator || "(off)", className: "key" });
    let pending: string | null = null;
    const hotkeyMsg = el("p", { className: `hint ${v.hotkey.error ? "bad" : ""}` }, v.hotkey.error ?? (v.hotkey.registered ? "Registered. Click the box and press a new combination." : v.hotkey.accelerator ? "Not registered." : "Turned off."));
    key.addEventListener("keydown", (e) => {
      if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
      e.preventDefault();
      const combo = accelerator(e);
      if (!combo) {
        hotkeyMsg.textContent = "That key can't be a hotkey. Use a letter, a digit, an F key or Space, with Ctrl, Alt or Shift.";
        return;
      }
      pending = combo;
      key.value = combo;
      hotkeyMsg.textContent = "Press Set to use it.";
    });
    const setHotkey = async (combo: string) => {
      try {
        await invoke("set_hotkey", { accelerator: combo });
        flash(note, combo ? `Hotkey set to ${combo}.` : "Hotkey turned off.");
      } catch (e) {
        flash(note, String(e), true);
        hotkeyMsg.className = "hint bad";
        hotkeyMsg.textContent = String(e);
        key.value = v.hotkey.accelerator || "(off)";
        return;
      }
      void refresh();
    };

    const devices = v.devices.length
      ? v.devices.map((d) => {
          const unpair = button("Unpair", () => {
            unpair.replaceWith(
              el("span", { className: "confirm" }, `Unpair ${d.name}? `, button("Yes, unpair", () => void call("remove_device", { deviceId: d.deviceId }).then(() => refresh()), "small"), button("No", () => void refresh(), "secondary small")),
            );
          }, "secondary small");
          return el("div", { className: "device" }, el("span", {}, `${d.name} `, el("small", {}, d.connected ? "connected" : "offline")), unpair);
        })
      : [el("p", { className: "hint" }, "No phone is paired.")];

    const folder = el("code", { className: "path" }, v.receivedFolder);
    const startHint = v.startWithWindowsIsReal ? "Kin starts when you sign in and waits in the tray." : "Developer run: a file stands in for Windows' startup list, which is never changed.";

    body.replaceChildren(
      section("This PC", el("label", { className: "field" }, "Device name", name), el("p", { className: "hint" }, `Shown on your phone, and sent when it connects. Empty means ${v.defaultDeviceName}.`)),
      section(
        "Clipboard and photos",
        toggle("Send the clipboard to the phone automatically", "Text you copy goes to the phone. Passwords from a password manager never do.", v.autoClipboardSync, (on) => void call("set_auto_clipboard_sync", { on })),
        toggle("Put photos from the phone on the clipboard", "A single image also goes to the clipboard, so Ctrl+V pastes it.", v.imagesToClipboard, (on) => void call("set_images_to_clipboard", { on })),
      ),
      section(
        "Received files",
        folder,
        el("div", { className: "row left" }, button("Choose folder…", () => void call("choose_received_folder").then(() => refresh()), "small"), v.receivedFolderIsDefault ? "" : button("Use the default", () => void call("reset_received_folder").then(() => refresh()), "secondary small")),
      ),
      section(
        "Hotkey",
        el("p", { className: "hint" }, "Sends the text on the clipboard to the phone."),
        key,
        hotkeyMsg,
        el("div", { className: "row left" }, button("Set", () => void setHotkey(pending ?? v.hotkey.accelerator), "small"), button("Turn off", () => void setHotkey(""), "secondary small"), button(`Default (${v.defaultHotkey})`, () => void setHotkey(v.defaultHotkey), "secondary small")),
      ),
      section("Start with Windows", toggle("Start Kin when I sign in", startHint, v.startWithWindows === true, (on) => void call("set_start_with_windows", { on }).then(() => refresh()))),
      section("Phone", ...devices),
      el("div", { className: "row left" }, button("Debug…", () => void call("open_window", { name: "debug" }), "secondary small"), el("span", { className: "hint" }, `Kin ${v.version}`)),
    );
  };

  let busy = false;
  const refresh = async () => {
    if (busy) return;
    busy = true;
    try {
      draw(await invoke<View>("settings_view"));
    } finally {
      busy = false;
    }
  };
  void listen("settings-changed", () => {
    // Don't redraw while a field has the focus: that would throw away what is being typed.
    if (!document.activeElement || document.activeElement === document.body) void refresh();
  });
  void refresh();
}
