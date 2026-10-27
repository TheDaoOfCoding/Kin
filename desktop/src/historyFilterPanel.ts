// The History page's Filter: a button for the heading (`host.acts`), the popover it opens (search, type, sort, direction) and the row of chips
// above the list. The rules are in historyFilter.ts. The chips and the popover live outside the list that history.ts redraws every few
// seconds, so typing in the search box never loses focus. The choices stay in memory for as long as the page does (never saved to disk).
import { el } from "./dom";
import { emptyArt, icon } from "./icons";
import {
  DIRECTION_OPTIONS,
  type DirectionId,
  type Query,
  SORT_OPTIONS,
  type SortId,
  TYPE_OPTIONS,
  chips,
  defaultQuery,
  toggleType,
  without,
} from "./historyFilter";

export type HistoryFilter = {
  /** The choices now (a copy: change them with the panel). */
  query: () => Query;
  /** Resets every choice. */
  clear: () => void;
  /** Closes the popover (a page that is hidden calls it). */
  close: () => void;
  /** The "Nothing matches." box for a list that the choices emptied, with its "Clear filters" button. */
  nothingMatches: () => HTMLElement;
};

let panelCount = 0;

/**
 * Puts the Filter button in `acts` and the chips row in `above`; `onChange` is called after every change of the choices (draw the list again).
 * Call it once per page.
 */
export function mountHistoryFilter(acts: HTMLElement, above: HTMLElement, onChange: () => void): HistoryFilter {
  const id = `hfilter-${++panelCount}`;
  let q = defaultQuery();

  const count = el("span", { className: "fcount", hidden: true });
  const btn = el("button", { type: "button", className: "btn soft sm fbtn" }, icon("filter"), "Filter", count);
  btn.setAttribute("aria-haspopup", "dialog");
  btn.setAttribute("aria-expanded", "false");
  btn.setAttribute("aria-controls", id);

  const search = el("input", { type: "text", autocomplete: "off", spellcheck: false, placeholder: "Search names and text" });
  search.setAttribute("aria-label", "Search");
  const clearBtn = el("button", { type: "button", className: "flink" }, "Clear");

  const group = <T extends string>(label: string, options: readonly { id: T; label: string }[], radio: boolean, pick: (id: T) => void) => {
    const buttons = new Map<T, HTMLButtonElement>();
    const box = el("div", { className: "fopts" });
    box.setAttribute("role", radio ? "radiogroup" : "group");
    box.setAttribute("aria-label", label);
    for (const o of options) {
      const b = el("button", { type: "button", className: "fopt" }, o.label);
      b.setAttribute("role", radio ? "radio" : "button");
      b.addEventListener("click", () => pick(o.id));
      buttons.set(o.id, b);
      box.append(b);
    }
    const set = (on: (id: T) => boolean) => {
      for (const [k, b] of buttons) {
        b.setAttribute(radio ? "aria-checked" : "aria-pressed", String(on(k)));
        b.classList.toggle("on", on(k));
      }
    };
    return { section: el("div", { className: "fsec" }, el("div", { className: "flabel" }, label), box), set };
  };

  const types = group("Type", TYPE_OPTIONS, false, (t) => change(toggleType(q, t)));
  const sorts = group("Sort", SORT_OPTIONS, true, (s: SortId) => change({ ...q, sort: s }));
  const dirs = group("Direction", DIRECTION_OPTIONS, true, (d: DirectionId) => change({ ...q, direction: d }));

  const pop = el(
    "div",
    { className: "fpop", id, tabIndex: -1, hidden: true },
    el("div", { className: "fhead" }, el("b", {}, "Filter"), clearBtn),
    el("label", { className: "fsearch" }, icon("search"), search),
    types.section,
    sorts.section,
    dirs.section,
  );
  pop.setAttribute("role", "dialog");
  pop.setAttribute("aria-label", "Filter the history");
  const wrap = el("div", { className: "menuwrap fwrap" }, btn, pop);
  acts.append(wrap);

  const row = el("div", { className: "fchips", hidden: true });
  above.append(row);

  /** Shows `q` everywhere without rebuilding anything the person is using (the search box keeps its focus and caret). */
  const show = () => {
    const list = chips(q);
    types.set((t) => q.types.includes(t));
    sorts.set((s) => q.sort === s);
    dirs.set((d) => q.direction === d);
    if (search.value !== q.search) search.value = q.search;
    clearBtn.hidden = list.length === 0;
    count.hidden = list.length === 0;
    count.textContent = String(list.length);
    btn.setAttribute("aria-label", list.length === 0 ? "Filter" : `Filter, ${list.length} active`);
    const items = list.map((c) => {
      const x = el("button", { type: "button", className: "fx" }, icon("x"));
      x.setAttribute("aria-label", `Remove ${c.label}`);
      x.addEventListener("click", () => change(without(q, c.id)));
      return el("span", { className: "chip fchip" }, el("span", { className: "fchip-t" }, c.label), x);
    });
    if (list.length >= 2) {
      const all = el("button", { type: "button", className: "chip fchip fclear" }, "Clear");
      all.addEventListener("click", () => change(defaultQuery()));
      items.push(all);
    }
    row.hidden = items.length === 0;
    row.replaceChildren(...items);
  };

  const change = (next: Query) => {
    q = next;
    show();
    onChange();
  };

  search.addEventListener("input", () => change({ ...q, search: search.value }));
  clearBtn.addEventListener("click", () => change(defaultQuery()));

  // The popover opens on a click of the button; a click outside, Esc or tabbing away closes it (the pattern of dom.ts `helpButton`).
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
  };
  const open = () => {
    pop.hidden = false;
    btn.setAttribute("aria-expanded", "true");
    document.addEventListener("pointerdown", away, true);
    document.addEventListener("keydown", esc, true);
    search.focus();
  };
  btn.addEventListener("click", () => (pop.hidden ? open() : close()));
  wrap.addEventListener("focusout", (e) => {
    if (!pop.hidden && !wrap.contains(e.relatedTarget as Node | null)) close();
  });

  show();
  return {
    query: () => ({ ...q, types: [...q.types] }),
    clear: () => change(defaultQuery()),
    close,
    nothingMatches: () => {
      const reset = el("button", { type: "button", className: "btn soft sm" }, "Clear filters");
      reset.addEventListener("click", () => change(defaultQuery()));
      return el("div", { className: "empty" }, emptyArt("history"), el("b", {}, "Nothing matches."), el("div", { className: "empty-act" }, reset));
    },
  };
}
