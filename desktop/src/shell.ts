// The frame of the main window: Home (`drop.ts`), History and Settings are three pages of it. A sidebar with the logo, the three pages and "Pair a phone",
// and a main area with a heading, its actions and the page. A click on a page swaps it in place (no new window, no reload): a page is built the first time it is
// shown and then only hidden and shown, so it keeps what it loaded, and a hidden page stops its timers (`PageHost.every`). Pairing and Debug stay separate
// windows (`open_window`, src-tauri/src/ui.rs).
import { invoke } from "@tauri-apps/api/core";
import { el } from "./dom";
import { icon, logo } from "./icons";

export type Page = "home" | "history" | "settings";

/** What a page gets from the shell: its own heading line, buttons and content area, and the page's lifecycle. */
export type PageHost = {
  /** This page's buttons on the right of the heading (swapped with the page). */
  acts: HTMLElement;
  /** This page's line under the heading. */
  sub: HTMLElement;
  /** This page's content. */
  body: HTMLElement;
  /** The computer's name and the version of Kin, shown in the sidebar once the page knows them. */
  setPc: (pcName: string, version: string) => void;
  visible: () => boolean;
  /** Show another page (Home's "See all" uses it). */
  go: (page: Page) => void;
  /** Called each time the page is shown, also the first time. */
  onShow: (run: () => void) => void;
  onHide: (run: () => void) => void;
  /** A timer that only runs while the page is visible. */
  every: (ms: number, run: () => void) => void;
};

export type PageStarters = Record<Page, (host: PageHost) => void>;
export type Shell = { go: (page: Page) => void };

const PAGES: { page: Page; icon: string; label: string }[] = [
  { page: "home", icon: "home", label: "Home" },
  { page: "history", icon: "history", label: "History" },
  { page: "settings", icon: "settings", label: "Settings" },
];

/** The page a name stands for (the `page=` of the window's URL, the payload of the `navigate` event), or null. */
export function parsePage(name: unknown): Page | null {
  return PAGES.find((p) => p.page === name)?.page ?? null;
}

type Slot = {
  host: PageHost;
  sub: HTMLElement;
  acts: HTMLElement;
  body: HTMLElement;
  nav: HTMLElement;
  started: boolean;
  scroll: number;
  shows: (() => void)[];
  hides: (() => void)[];
  timers: { ms: number; run: () => void; id: number | null }[];
};

/** Builds the sidebar and the three page areas, starts Home (it holds the drop target, so it exists from the start) and shows `first`. */
export function mountShell(root: HTMLElement, starters: PageStarters, first: Page): Shell {
  root.className = "shell";
  const pc = el("small", {}, "");
  const ver = el("span", { className: "ver" }, "Stays on your Wi-Fi");
  let current: Page | null = null;

  const nav = PAGES.map((p) => {
    const item = el("button", { type: "button", className: "nv" }, icon(p.icon), p.label);
    item.addEventListener("click", () => go(p.page));
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

  const title = el("h2", {}, "");
  const subs = el("div", {}, title);
  const mh = el("div", { className: "mh" }, subs);
  const main = el("div", { className: "main" }, mh);

  const slots = {} as Record<Page, Slot>;
  PAGES.forEach((p, n) => {
    const sub = el("p", {});
    const acts = el("div", { className: "acts" });
    const body = el("div", { className: "mbody" });
    for (const part of [sub, acts, body]) part.hidden = true;
    subs.append(sub);
    mh.append(acts);
    main.append(body);
    const slot: Slot = {
      sub,
      acts,
      body,
      nav: nav[n],
      started: false,
      scroll: 0,
      shows: [],
      hides: [],
      timers: [],
      host: {
        acts,
        sub,
        body,
        setPc: (pcName, version) => {
          pc.textContent = pcName ? `on ${pcName}` : "";
          ver.textContent = `Version ${version} · stays on your Wi-Fi`;
        },
        visible: () => current === p.page,
        go: (page) => go(page),
        onShow: (run) => slot.shows.push(run),
        onHide: (run) => slot.hides.push(run),
        every: (ms, run) => {
          const timer = { ms, run, id: null as number | null };
          slot.timers.push(timer);
          // A timer added while the page is already showing starts at once.
          if (current === p.page) timer.id = window.setInterval(run, ms);
        },
      },
    };
    slots[p.page] = slot;
  });
  root.replaceChildren(el("div", { className: "wbody" }, side, main));

  const start = (page: Page) => {
    const slot = slots[page];
    if (slot.started) return;
    slot.started = true;
    starters[page](slot.host);
  };

  function go(page: Page): void {
    if (current === page) return;
    const was = current ? slots[current] : null;
    if (was) {
      was.scroll = was.body.scrollTop;
      for (const t of was.timers) {
        if (t.id !== null) window.clearInterval(t.id);
        t.id = null;
      }
    }
    current = page;
    const slot = slots[page];
    start(page);
    for (const p of PAGES) {
      const s = slots[p.page];
      const on = p.page === page;
      s.sub.hidden = s.acts.hidden = s.body.hidden = !on;
      s.nav.classList.toggle("on", on);
      if (on) s.nav.setAttribute("aria-current", "page");
      else s.nav.removeAttribute("aria-current");
    }
    title.textContent = PAGES.find((p) => p.page === page)?.label ?? "";
    slot.body.scrollTop = slot.scroll;
    if (was) for (const run of was.hides) run();
    for (const t of slot.timers) if (t.id === null) t.id = window.setInterval(t.run, t.ms);
    for (const run of slot.shows) run();
  }

  // Home holds the drop target, so it exists from the start, even when the window opens on another page.
  start("home");
  go(first);
  return { go };
}
