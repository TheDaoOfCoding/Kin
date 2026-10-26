// Small helpers the windows share. Plain DOM, no component library.
import { icon } from "./icons";

export function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...children: (Node | string)[]): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

/** A text button. `className` says what kind: "secondary" (plain instead of the main look) and "small". */
export function button(label: string, onClick: () => void, className = ""): HTMLButtonElement {
  const words = className.split(" ");
  const b = el("button", { type: "button", className: `btn ${words.includes("secondary") ? "plain" : "primary"}${words.includes("small") ? " sm" : ""}` }, label);
  b.addEventListener("click", onClick);
  return b;
}

/** A button with an icon of the sprite in front of its label. `className` is the look: "primary", "soft", "plain", "warn", with "sm" for a small one. */
export function actionButton(iconName: string | null, label: string, onClick: () => void, className = "plain"): HTMLButtonElement {
  const b = el("button", { type: "button", className: `btn ${className}` }, ...(iconName ? [icon(iconName)] : []), label);
  b.addEventListener("click", onClick);
  return b;
}

let helpCount = 0;
let closeOpenHelp: (() => void) | null = null;

/** A small "?" button that opens a popover with `text`. A click opens it; a click outside, Esc or tabbing away closes it. One is open at a time. `about` names the setting for screen readers. */
export function helpButton(text: string, about: string): HTMLElement {
  const id = `help-${++helpCount}`;
  const pop = el("div", { className: "pop", id, tabIndex: -1, hidden: true }, text);
  pop.setAttribute("role", "note");
  const btn = el("button", { type: "button", className: "helpbtn" }, "?");
  btn.setAttribute("aria-label", `About ${about}`);
  btn.setAttribute("aria-expanded", "false");
  btn.setAttribute("aria-controls", id);
  const wrap = el("span", { className: "help" }, btn, pop);

  const away = (e: Event) => {
    if (!wrap.isConnected || !wrap.contains(e.target as Node)) close();
  };
  const esc = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    close();
    btn.focus();
  };
  const close = () => {
    pop.hidden = true;
    btn.setAttribute("aria-expanded", "false");
    document.removeEventListener("pointerdown", away, true);
    document.removeEventListener("keydown", esc, true);
    if (closeOpenHelp === close) closeOpenHelp = null;
  };
  const open = () => {
    closeOpenHelp?.();
    pop.classList.remove("end", "up");
    pop.hidden = false;
    // Keep the popover inside the window: open to the left or above when it would run out.
    const r = pop.getBoundingClientRect();
    if (r.right > window.innerWidth - 8) pop.classList.add("end");
    if (r.bottom > window.innerHeight - 8 && btn.getBoundingClientRect().top > r.height + 16) pop.classList.add("up");
    btn.setAttribute("aria-expanded", "true");
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("keydown", esc, true);
    closeOpenHelp = close;
  };
  btn.addEventListener("click", () => (pop.hidden ? open() : close()));
  wrap.addEventListener("focusout", (e) => {
    if (!pop.hidden && !wrap.contains(e.relatedTarget as Node | null)) close();
  });
  return wrap;
}

export function size(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

const two = (n: number) => String(n).padStart(2, "0");

/** "14:32" for today, "12 Oct 14:32" for another day of this year, "12 Oct 2025 14:32" for an older one. */
export function when(ms: number | null | undefined, now = Date.now()): string {
  if (ms === null || ms === undefined) return "";
  const d = new Date(ms);
  const n = new Date(now);
  const clock = `${two(d.getHours())}:${two(d.getMinutes())}`;
  if (d.toDateString() === n.toDateString()) return clock;
  const month = d.toLocaleString("en-GB", { month: "short" });
  return d.getFullYear() === n.getFullYear() ? `${d.getDate()} ${month} ${clock}` : `${d.getDate()} ${month} ${d.getFullYear()} ${clock}`;
}

/** The name of an error code the way a person reads it: "expired" stays "expired", "disk_full" becomes "disk full". */
export function errorText(code: string | null | undefined): string {
  return code ? code.replace(/_/g, " ") : "";
}

/** What the history shows for a failed item: a code that is advice for the user ("peer_unsupported") is said as the advice, the rest as `errorText`. */
export function historyErrorText(code: string | null | undefined): string {
  return code === "peer_unsupported" ? "Update Kin on your phone" : errorText(code);
}

/** A message line that fades after a few seconds, for the answer to a click. */
export function flash(target: HTMLElement, text: string, bad = false): void {
  target.textContent = text;
  target.className = `flash ${bad ? "bad" : "ok"}`;
  window.setTimeout(() => {
    if (target.textContent === text) target.textContent = "";
  }, 5000);
}

/** "1 sensitive text arrived while paused (not kept)": a count only, never any content. */
export function sensitiveWhilePausedText(n: number): string {
  return n === 1 ? "1 sensitive text arrived while paused (not kept)" : `${n} sensitive texts arrived while paused (not kept)`;
}
