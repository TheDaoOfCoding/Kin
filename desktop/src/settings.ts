// The settings window: device name, automatic clipboard sync, images to the clipboard, the received folder, the hotkey (with conflict
// detection: if another program has the key, the old one stays and the window says so), start with Windows, Pause, the phone's battery, and Unlink (2B.4: one word,
// a confirmation that says what happens, and a "Lost your phone?" line with Ring).
// The backend (src-tauri/src/prefs.rs, app.rs) saves each change at once and applies it.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { effectiveMode, type Skin, type Theme } from "./appearance";
import { button, el, flash, sensitiveWhilePausedText } from "./dom";
import { icon } from "./icons";
import { mountShell } from "./shell";

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
type Battery = { level: number; charging: boolean } | null;
/** Presence, Pause and Ring (2B.4): what the Settings window shows. The window may show the phone's level; no log and no `status.json` ever has it. */
export type Presence = {
  pause: { on: boolean; untilResumed: boolean; text: string | null; quietArrivals: number; quietSensitive: number };
  phone: {
    name: string | null;
    deviceId: string | null;
    connected: boolean;
    battery: Battery;
    lastKnown: Battery;
    paused: boolean;
    pausedText: string | null;
    /** The phone's Kin lists `presence`: it can be told about a pause. */
    knowsPause: boolean;
    /** "Do Not Disturb" (2B.5): what the phone says about its own Do Not Disturb while it is on and shared. Memory only. */
    dnd: string | null;
  };
  /** Focus and Do Not Disturb (2B.5). `sessionsSupported` is null until Kin has read Windows (the switch is off, or no phone is connected). */
  focus: { share: boolean; alsoPresenting: boolean; holdToasts: boolean; sessionsSupported: boolean | null; on: boolean };
  ring: { can: boolean; active: boolean; text: string | null; reason: string | null };
  shareBattery: boolean;
  showPhone: boolean;
  lowBatteryNudge: boolean;
  hasBattery: boolean;
};
type View = {
  deviceName: string;
  customDeviceName: string | null;
  defaultDeviceName: string;
  autoClipboardSync: boolean;
  imagesToClipboard: boolean;
  openLinksAutomatically: boolean;
  skin: Skin;
  theme: Theme;
  mirror: Mirror;
  media: Media;
  presence: Presence;
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
  return el("section", {}, el("h3", { className: "slabel" }, heading), el("div", { className: "card rows" }, ...children));
}

/** "62% charging", "14%": the battery as a person reads it. */
export function batteryText(b: Battery): string {
  return b ? (b.charging ? `${b.level}% charging` : `${b.level}%`) : "";
}

/** The device id of `#unlink=<deviceId>` (the tray's "Unlink phone…" opens the window with it), or null. Device IDs are base64url, so nothing else is let through. */
export function unlinkFromHash(hash: string): string | null {
  const m = /^#unlink=([A-Za-z0-9_-]{1,64})$/.exec(hash);
  return m ? m[1] : null;
}

/** The text of the confirmation of Unlink: what happens, and for an offline phone when it finds out. */
export function unlinkText(name: string, connected: boolean): string {
  return (
    `Unlink ${name}? This phone stops working with Kin right away: nothing more can be sent, shown or rung, even if it is offline. You can pair it again later with a new code.` +
    (connected ? "" : " It will find out the next time it connects.")
  );
}

export function startSettingsWindow(root: HTMLElement): void {
  const shell = mountShell(root, "settings");
  shell.sub.textContent = "Make Kin look and work the way you like.";
  const note = el("p", { className: "flash" });
  const body = el("div", { className: "settings" });
  shell.body.append(body, note);
  // The device whose "Unlink" confirmation is open. The tray's "Unlink phone…" opens the window with `#unlink=<deviceId>`, or tells an open one with an event.
  let unlinkOpen: string | null = unlinkFromHash(window.location.hash);

  const call = async (command: string, args: Record<string, unknown> = {}): Promise<boolean> => {
    try {
      await invoke(command, args);
      return true;
    } catch (e) {
      flash(note, String(e), true);
      return false;
    }
  };

  const toggle = (label: string, hint: string, value: boolean, onChange: (on: boolean) => void, disabled = false): HTMLElement => {
    const box = el("input", { type: "checkbox", checked: value, disabled });
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

  /** The Appearance card: the skin (two pictures to pick from) and the theme. The change is saved and shown in every open window at once. */
  const appearanceSection = (v: View): HTMLElement => {
    let skin = v.skin;
    let theme = v.theme;
    const skinOptions: { skin: Skin; name: string; hint: string; preview: HTMLElement }[] = (["warm", "simple"] as const).map((k) => {
      const preview = el("span", { className: "sp" }, el("span", { className: "sp-hero" }, el("span", { className: "dot" })), el("span", { className: "sp-cta" }), el("span", { className: "sp-tints" }, ...["lilac", "honey", "sage", "peach"].map((t) => el("i", { className: t }))));
      preview.dataset.skin = k;
      return { skin: k, name: k === "warm" ? "Warm" : "Simple", hint: k === "warm" ? "Friendly, the default" : "Plain and quiet", preview };
    });
    const themeOptions: { theme: Theme; name: string; icon: string }[] = [
      { theme: "system", name: "System", icon: "auto" },
      { theme: "light", name: "Light", icon: "sun" },
      { theme: "dark", name: "Dark", icon: "moon" },
    ];
    const skinButtons = skinOptions.map((o) => {
      const b = el("button", { type: "button", className: "skin-opt", role: "radio" as string }, o.preview, el("span", { className: "lab" }, el("span", { className: "radio" }), el("span", {}, el("b", {}, o.name), el("small", {}, o.hint))));
      b.addEventListener("click", () => choose(o.skin, theme));
      return b;
    });
    const themeButtons = themeOptions.map((o) => {
      const b = el("button", { type: "button", role: "radio" as string }, icon(o.icon), o.name);
      b.addEventListener("click", () => choose(skin, o.theme));
      return b;
    });
    // The little pictures of the skins show the mode the windows are in now.
    const mark = () => {
      skinButtons.forEach((b, n) => b.setAttribute("aria-checked", String(skinOptions[n].skin === skin)));
      themeButtons.forEach((b, n) => b.setAttribute("aria-checked", String(themeOptions[n].theme === theme)));
      skinOptions.forEach((o) => (o.preview.dataset.mode = effectiveMode(theme)));
    };
    const choose = (nextSkin: Skin, nextTheme: Theme) => {
      skin = nextSkin;
      theme = nextTheme;
      mark();
      void call("set_appearance", { skin, theme });
    };
    mark();
    return el(
      "section",
      {},
      el("h3", { className: "slabel" }, "Appearance"),
      el(
        "div",
        { className: "card appear" },
        el("div", { className: "lbl" }, "Skin"),
        el("div", { className: "skins", role: "radiogroup", ariaLabel: "Skin" }, ...skinButtons),
        el("div", { className: "lbl" }, "Theme"),
        el("div", { className: "themes", role: "radiogroup", ariaLabel: "Theme" }, ...themeButtons),
      ),
    );
  };

  const draw = (v: View) => {
    shell.setPc(v.deviceName, v.version);
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

    const p = v.presence;
    const ringNow = async (start: boolean) => {
      try {
        flash(note, await invoke<string>("ring_phone", { start }));
      } catch (e) {
        flash(note, String(e), true);
      }
      void refresh();
    };
    const ringButton = (): HTMLElement =>
      p.ring.active
        ? button("Stop ringing phone", () => void ringNow(false), "small")
        : el("span", {}, button("Ring my phone", () => void ringNow(true), "small"), ...(p.ring.can ? [] : [el("small", { className: "hint" }, ` ${p.ring.reason ?? "Ringing isn't available."}`)]));
    const devices = v.devices.length
      ? v.devices.map((d) => {
          const here = p.phone.deviceId === d.deviceId;
          const details: HTMLElement[] = [];
          if (here) {
            const bat = d.connected ? p.phone.battery : p.phone.lastKnown;
            if (p.showPhone && bat) details.push(el("small", {}, ` ${d.connected ? "" : "last known "}${batteryText(bat)}`));
            if (d.connected && p.phone.paused) details.push(el("small", {}, ` Kin paused on the phone ${p.phone.pausedText ?? ""}`));
          }
          const unlink = button("Unlink", () => {
            unlinkOpen = d.deviceId;
            void refresh();
          }, "secondary small");
          const confirm = el(
            "span",
            { className: "confirm" },
            unlinkText(d.name, d.connected),
            " ",
            button("Yes, unlink", () => void call("remove_device", { deviceId: d.deviceId }).then(() => { unlinkOpen = null; return refresh(); }), "small"),
            button("No", () => { unlinkOpen = null; void refresh(); }, "secondary small"),
          );
          const lost = el(
            "p",
            { className: "hint" },
            "Lost your phone? Unlink it here: it stops working at once, even if it is offline. If it is connected you can ring it first. ",
            here ? ringButton() : "",
          );
          return el("div", { className: "device" }, el("span", {}, `${d.name} `, el("small", {}, d.connected ? "connected" : "offline"), ...details), unlinkOpen === d.deviceId ? confirm : unlink, lost);
        })
      : [el("p", { className: "hint" }, "No phone is paired.")];

    const choice = (label: string, which: string) => button(label, () => void call("set_pause", { choice: which }).then(() => refresh()), "small");
    const pauseSection = section(
      "Pause",
      el("p", { className: "hint" }, "While paused Kin sends and shows nothing by itself: no automatic clipboard, phone notifications, media or screenshots. Things you send by hand still arrive, quietly, in the history. Ringing your phone still works."),
      p.pause.on
        ? el("p", {}, el("strong", {}, `Paused ${p.pause.text ?? ""}. `), button("Resume Kin", () => void call("set_pause", { choice: "resume" }).then(() => refresh()), "small"), p.pause.quietArrivals ? el("small", { className: "hint" }, ` ${p.pause.quietArrivals} received while paused`) : "", p.pause.quietSensitive ? el("small", { className: "hint" }, ` ${sensitiveWhilePausedText(p.pause.quietSensitive)}`) : "")
        : el("div", { className: "row left" }, choice("Pause for 1 hour", "1h"), choice("Until tomorrow", "tomorrow"), choice("Until I resume", "resumed")),
      ...(v.devices.length && !p.phone.knowsPause && p.phone.connected ? [el("p", { className: "hint bad" }, `${p.phone.name ?? "The phone"} doesn't know about pausing: update Kin there.`)] : []),
    );
    const shareBox = el("input", { type: "checkbox", checked: p.shareBattery && p.hasBattery, disabled: !p.hasBattery });
    shareBox.addEventListener("change", () => void call("set_presence_option", { name: "shareBattery", on: shareBox.checked }));
    const presenceSection = section(
      "Phone battery",
      toggle("Show my phone's battery here", "In the tray icon's tooltip and the panel. When the phone is offline the last level it told is shown with the time.", p.showPhone, (on) => void call("set_presence_option", { name: "showPhone", on })),
      toggle("Warn me when the phone's battery is low", "One notice when it drops to 15 % or less and is not charging.", p.lowBatteryNudge, (on) => void call("set_presence_option", { name: "lowBatteryNudge", on })),
      el("label", { className: "toggle" }, shareBox, el("span", {}, el("strong", {}, "Share this PC's battery with the phone"), el("small", {}, p.hasBattery ? "The phone's notification and home screen show it." : "No battery: this PC is not a laptop."))),
    );

    const f = p.focus;
    const focusNotes: HTMLElement[] = [];
    if (f.sessionsSupported === false) focusNotes.push(el("p", { className: "hint" }, "Windows 10 only reports Do Not Disturb."));
    // A phone whose Kin doesn't list `presence` can't be told about Focus: the switch can't be turned on (a switch that is already on can still be turned off).
    const phoneLacksPresence = v.devices.length > 0 && p.phone.connected && !p.phone.knowsPause;
    if (phoneLacksPresence) focusNotes.push(el("p", { className: "hint bad" }, `${p.phone.name ?? "The phone"} needs Kin 0.2.0 for this: update Kin on your phone.`));
    if (f.share && f.on) focusNotes.push(el("p", { className: "hint" }, "This PC is in Focus now: your phone has been told."));
    if (p.phone.dnd) focusNotes.push(el("p", { className: "hint" }, `Phone: ${p.phone.dnd}.`));
    const focusSection = section(
      "Focus and Do Not Disturb",
      toggle("Silence my phone while this PC is in Focus", "Kin reads whether Windows Focus or Do Not Disturb is on. It does not change them. Your phone then turns on its own Do Not Disturb if you allowed that in Kin there, and turns it off again when your PC leaves Focus. Priority only by default.", f.share, (on) => void call("set_focus_option", { name: "focusShare", on }), phoneLacksPresence && !f.share),
      toggle("Also when I'm presenting or playing full screen", "A full-screen program or a presentation counts as Focus. Off by default: a game is not always \"don't disturb me\" for the phone.", f.alsoPresenting, (on) => void call("set_focus_option", { name: "focusAlsoPresenting", on })),
      toggle("Hold Kin's own notices while my phone is in Do Not Disturb", "The low-battery notice and similar notices from Kin wait. Phone notifications shown here and the answers to what you do are never held.", f.holdToasts, (on) => void call("set_focus_option", { name: "holdToasts", on })),
      ...focusNotes,
      el("p", { className: "hint" }, "Tell me when it's done: from a script run Kin.exe --notify \"Build finished\" (add --title, --ttl 2h or --device). Your phone shows it as a notification from this PC."),
    );

    const folder = el("code", { className: "path" }, v.receivedFolder);
    const startHint = v.startWithWindowsIsReal ? "Kin starts when you sign in and waits in the tray." : "Developer run: a file stands in for Windows' startup list, which is never changed.";

    body.replaceChildren(
      appearanceSection(v),
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
      pauseSection,
      presenceSection,
      focusSection,
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
  void listen<string>("unlink-requested", (event) => {
    unlinkOpen = /^[A-Za-z0-9_-]{1,64}$/.test(event.payload) ? event.payload : null;
    void refresh();
  });
  void listen("settings-changed", () => {
    // Don't redraw while a field has the focus: that would throw away what is being typed.
    if (!document.activeElement || document.activeElement === document.body) void refresh();
  });
  // The look was changed (here, or by a script): the pictures and the choices follow.
  void listen("appearance-changed", () => void refresh());
  void refresh();
}
