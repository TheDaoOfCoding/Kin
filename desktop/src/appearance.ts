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
}

/** Shows the default look at once, then the saved one as soon as the backend answers. */
export function startAppearance(): void {
  applyAppearance(current);
  dark.addEventListener("change", () => applyAppearance(current));
  void invoke<Appearance>("appearance_view").then(applyAppearance, () => {});
  void listen<Appearance>("appearance-changed", (event) => applyAppearance(event.payload));
}
