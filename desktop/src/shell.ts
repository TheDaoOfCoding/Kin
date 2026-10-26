// The frame of the three windows that are one place to the person: Home (the drop window), History and Settings. A sidebar with the logo, the three
// pages and "Pair a phone", and a main area with a heading, its actions and the page. A click on a page opens that window in the same place and
// size and closes this one (`open_window`, src-tauri/src/ui.rs), so it feels like one window changing its page.
import { invoke } from "@tauri-apps/api/core";
import { el } from "./dom";
import { icon, logo } from "./icons";

export type Page = "drop" | "history" | "settings";
export type Shell = {
  /** The heading's right side: the buttons of the page. */
  acts: HTMLElement;
  /** The line under the heading. */
  sub: HTMLElement;
  /** The page itself. */
  body: HTMLElement;
  /** The computer's name and the version of Kin, shown in the sidebar once the page knows them. */
  setPc: (pcName: string, version: string) => void;
};

const PAGES: { page: Page; icon: string; label: string }[] = [
  { page: "drop", icon: "home", label: "Home" },
  { page: "history", icon: "history", label: "History" },
  { page: "settings", icon: "settings", label: "Settings" },
];

export function mountShell(root: HTMLElement, active: Page): Shell {
  root.className = "shell";
  const pc = el("small", {}, "");
  const ver = el("span", { className: "ver" }, "Stays on your Wi-Fi");

  const nav = PAGES.map((p) => {
    const item = el("button", { type: "button", className: `nv${p.page === active ? " on" : ""}` }, icon(p.icon), p.label);
    if (p.page === active) item.setAttribute("aria-current", "page");
    else item.addEventListener("click", () => void invoke("open_window", { name: p.page }));
    return item;
  });
  const pair = el("button", { type: "button", className: "btn plain sm" }, icon("plus"), "Pair a phone");
  pair.addEventListener("click", () => void invoke("open_window", { name: "pairing" }));

  const side = el(
    "aside",
    { className: "side" },
    el("div", { className: "side-brand" }, logo(), el("div", {}, el("b", {}, "Kin"), pc)),
    el("nav", {}, ...nav),
    el(
      "div",
      { className: "side-foot" },
      el("div", { className: "paircard tinted lilac" }, el("b", {}, "Got another phone?"), el("small", {}, "Scan a code and it joins the family."), pair),
      ver,
    ),
  );

  const title = el("h2", {}, PAGES.find((p) => p.page === active)?.label ?? "");
  const sub = el("p", {});
  const acts = el("div", { className: "acts" });
  const body = el("div", { className: "mbody" });
  root.replaceChildren(el("div", { className: "wbody" }, side, el("div", { className: "main" }, el("div", { className: "mh" }, el("div", {}, title, sub), acts), body)));

  return {
    acts,
    sub,
    body,
    setPc: (pcName, version) => {
      pc.textContent = pcName ? `on ${pcName}` : "";
      ver.textContent = `Version ${version} · stays on your Wi-Fi`;
    },
  };
}
