// Small helpers the 1.4 windows (history, settings, debug) share. Plain DOM, no component library.

export function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...children: (Node | string)[]): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

export function button(label: string, onClick: () => void, className = ""): HTMLButtonElement {
  const b = el("button", { type: "button", className }, label);
  b.addEventListener("click", onClick);
  return b;
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
