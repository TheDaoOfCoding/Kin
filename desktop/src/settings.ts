// The settings window: device name, automatic clipboard sync, images to the clipboard, the received folder, the hotkey (with conflict
// detection: if another program has the key, the old one stays and the window says so), start with Windows, and unpair.
// The backend (src-tauri/src/prefs.rs, app.rs) saves each change at once and applies it.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { button, el, flash } from "./dom";

type Device = { deviceId: string; name: string; platform: string; connected: boolean };
type Mirror = {
  enabled: boolean;
  sound: boolean;
  hideContent: boolean;
  dismissToPhone: boolean;
  textOnly: boolean;
  /** The toast layer is running (the switch is on and Windows accepted it). */
  running: boolean;
  /** The COM class that answers clicks is registered: the reply box and the buttons work. */
  activator: boolean;
  /** Why phone notifications can't run in this copy of Kin, if they can't. */
  unavailable: string | null;
  phones: { name: string; status: string }[];
};
type Media = {
  /** This PC's media goes to the phone. */
  share: boolean;
  /** The phone's media shows on this PC. */
  show: boolean;
  /** "installed", or "test" (a copy beside the installed Kin, a debug build, a build folder: it never touches your real media or volume). */
  mode: string;
  reading: boolean;
  flyout: boolean;
  readerUnavailable: string | null;
  flyoutUnavailable: string | null;
  phoneMedia: string;
  pcMedia: string;
  phones: { name: string; status: string }[];
};
type View = {
  deviceName: string;
  customDeviceName: string | null;
  defaultDeviceName: string;
  autoClipboardSync: boolean;
  imagesToClipboard: boolean;
  openLinksAutomatically: boolean;
  mirror: Mirror;
  media: Media;
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

  const mirrorOption = (name: string) => (on: boolean) => void call("set_mirror_option", { name, on }).then(() => refresh());

  /** What belongs under the switch: the status of the phones, why it can't run, and the four options. */
  const mirrorDetails = (m: Mirror): HTMLElement[] => {
    const out: HTMLElement[] = [];
    if (m.unavailable) out.push(el("p", { className: "hint bad" }, `Not available in this copy of Kin: ${m.unavailable}.`));
    for (const p of m.phones) out.push(el("p", { className: "hint" }, `${p.name}: ${p.status}`));
    if (m.enabled && m.running && !m.activator && !m.textOnly) out.push(el("p", { className: "hint bad" }, "Replying and buttons need a part of Windows that did not start here, so notifications show text only."));
    if (!m.enabled) return out;
    out.push(
      toggle("Closing a phone notification here also dismisses it on the phone", "Only pressing the X of a notification does this, never a notification that timed out or was replaced.", m.dismissToPhone, mirrorOption("dismissToPhone")),
      toggle("Hide notification content", "Every phone notification says \"Private notification\". Notifications of banking, authenticator and password apps always do.", m.hideContent, mirrorOption("hideContent")),
      toggle("Play a sound for phone notifications", "Off: they appear quietly, because your phone has made its own sound.", m.sound, mirrorOption("sound")),
      toggle("Text only (no reply box, no buttons)", "For a Windows where the reply box and the buttons do not work.", m.textOnly, mirrorOption("textOnly")),
    );
    return out;
  };

  /** What belongs under the two media switches: where it can't run, and what each phone says. */
  const mediaDetails = (m: Media): HTMLElement[] => {
    const out: HTMLElement[] = [];
    if (m.mode === "test") out.push(el("p", { className: "hint" }, "This copy of Kin is a test run: it reads and controls no media of yours and never changes the PC's volume, and it never shows Playing."));
    if (m.share && m.readerUnavailable) out.push(el("p", { className: "hint bad" }, `The PC's media can't be read here: ${m.readerUnavailable}.`));
    if (m.show && m.flyoutUnavailable) out.push(el("p", { className: "hint bad" }, `${m.flyoutUnavailable}. The tray menu and the panel of the window you open with a click on the tray icon still work.`));
    for (const p of m.phones) out.push(el("p", { className: "hint" }, `${p.name}: ${p.status}`));
    return out;
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
        "Links",
        toggle("Open links from the phone automatically", "A web page you share from the phone opens in your default browser. Off: a notice with Open and Copy buttons. In a fullscreen game or a presentation a link is never opened by itself.", v.openLinksAutomatically, (on) => void call("set_open_links_automatically", { on })),
      ),
      section(
        "Phone notifications",
        toggle("Show phone notifications on this PC", "Notifications of the apps you chose in Kin on your phone appear here as Windows notifications, with the reply box and buttons the app offers. The text stays in Windows' Action Center until it expires or you clear it. Needs Kin 0.2.0 on your phone.", v.mirror.enabled, (on) => void invoke("set_mirror_notifications", { on }).then(() => refresh(), (e) => flash(note, String(e), true))),
        ...mirrorDetails(v.mirror),
      ),
      section(
        "Media",
        toggle("Show the phone's media on this PC", "What plays on your phone appears in the Windows media flyout (volume keys menu) with its buttons, and in the panel of the tray icon. The title is only shown there. Needs Kin 0.2.0 on your phone, with sharing turned on.", v.media.show, (on) => void invoke("set_media_option", { name: "show", on }).then(() => refresh(), (e) => flash(note, String(e), true))),
        toggle("Share this PC's media with the phone", "What plays on this PC appears in your phone's media controls, and the phone's buttons can pause, skip and change the PC's volume. Nothing is stored.", v.media.share, (on) => void invoke("set_media_option", { name: "share", on }).then(() => refresh(), (e) => flash(note, String(e), true))),
        ...mediaDetails(v.media),
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
