// The look of every window: the skin (Warm or Simple) and the theme (System, Light or Dark) become `data-skin` and `data-mode` on the page, and
// styles.css does the rest. The setting is kept by the backend (settings.json) and arrives when the window opens and whenever it is changed, in
// any window (`appearance-changed`). System follows the colour scheme of Windows, live.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Skin = "warm" | "simple";
export type Theme = "system" | "light" | "dark";
export type Appearance = { skin: Skin; theme: Theme };

const dark = window.matchMedia("(prefers-color-scheme: dark)");
let current: Appearance = { skin: "warm", theme: "system" };
// The last look any window showed: a new window paints it at once instead of flashing the default until `appearance_view` answers.
const CACHE_KEY = "kin.appearance";

/** The mode a theme shows now: Light and Dark are themselves, System is what Windows is set to. */
export function effectiveMode(theme: Theme): "light" | "dark" {
  return theme === "system" ? (dark.matches ? "dark" : "light") : theme;
}

export function applyAppearance(next: Partial<Appearance> | null | undefined): void {
  const skin: Skin = next?.skin === "simple" ? "simple" : "warm";
  const theme: Theme = next?.theme === "light" || next?.theme === "dark" ? next.theme : "system";
  current = { skin, theme };
  const root = document.documentElement;
  root.dataset.skin = skin;
  root.dataset.mode = effectiveMode(theme);
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify(current));
  } catch {
    // No storage: the window still gets the saved look from the backend, after a moment of the default.
  }
}

/** Shows the last known look at once (or the default), then the saved one as soon as the backend answers. */
export function startAppearance(): void {
  let cached: Partial<Appearance> | null = null;
  try {
    cached = JSON.parse(localStorage.getItem(CACHE_KEY) ?? "null");
  } catch {
    cached = null;
  }
  applyAppearance(cached ?? current);
  dark.addEventListener("change", () => applyAppearance(current));
  void invoke<Appearance>("appearance_view").then(applyAppearance, () => {});
  void listen<Appearance>("appearance-changed", (event) => applyAppearance(event.payload));
}
