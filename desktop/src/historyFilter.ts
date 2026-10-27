// The History filter and sort, as plain functions: no DOM and no Tauri, so `bun test tests` runs them. The rules are contract C3 of
// docs/implementation-plan/01 - One window and History filters.md; the phone's `HistoryView.kt` follows the same rules.

/** The seven types of the filter. The backend sends them in `types` (history.rs `item_types`). */
export type TypeId = "photo" | "video" | "document" | "app" | "other" | "text" | "link";
export type SortId = "newest" | "oldest" | "name" | "size";
export type DirectionId = "both" | "out" | "in";

export type Query = {
  /** What the person typed; matched as a case-insensitive "contains". */
  search: string;
  /** An item matching any of them is shown. Empty: no type filter. */
  types: TypeId[];
  sort: SortId;
  direction: DirectionId;
};

/** What the filter reads of a History item (the page's `Item` has all of it). */
export type FilterItem = {
  direction: "in" | "out";
  kind: "file" | "text";
  createdMs: number;
  totalBytes: number;
  /** The first file names the backend sends (up to 20). */
  names: string[];
  text: string | null;
  isUrl: boolean;
  isLink: boolean;
  title: string | null;
  types: string[];
};

type Option<T extends string> = { id: T; label: string };

export const TYPE_OPTIONS: readonly Option<TypeId>[] = [
  { id: "photo", label: "Photos" },
  { id: "video", label: "Videos" },
  { id: "document", label: "Documents" },
  { id: "app", label: "Apps" },
  { id: "other", label: "Other files" },
  { id: "text", label: "Texts" },
  { id: "link", label: "Links" },
];
export const SORT_OPTIONS: readonly Option<SortId>[] = [
  { id: "newest", label: "Newest" },
  { id: "oldest", label: "Oldest" },
  { id: "name", label: "Name" },
  { id: "size", label: "Size" },
];
export const DIRECTION_OPTIONS: readonly Option<DirectionId>[] = [
  { id: "both", label: "Both" },
  { id: "out", label: "Sent" },
  { id: "in", label: "Received" },
];

const labelOf = <T extends string>(options: readonly Option<T>[], id: T): string => options.find((o) => o.id === id)?.label ?? id;

export function defaultQuery(): Query {
  return { search: "", types: [], sort: "newest", direction: "both" };
}

/** True when the query changes nothing. */
export function isDefault(q: Query): boolean {
  return chips(q).length === 0;
}

/** The text a Name sort and a row go by: the first file name, or what a text row shows (a link's title, else its address). */
function nameOf(i: FilterItem): string {
  if (i.kind === "text") return (i.isLink || i.isUrl) && i.title ? i.title : (i.text ?? "");
  return i.names[0] ?? "";
}

const collator = new Intl.Collator(undefined, { sensitivity: "base" });

function matchesSearch(i: FilterItem, needle: string): boolean {
  return [...i.names, i.text ?? "", i.title ?? ""].some((s) => s.toLowerCase().includes(needle));
}

/** Filters and sorts `items` (any order in, the list the page shows out). The input is not changed. */
export function applyQuery<T extends FilterItem>(items: readonly T[], q: Query): T[] {
  const needle = q.search.trim().toLowerCase();
  const kept = items
    .map((item, at) => ({ item, at }))
    .filter(({ item }) => {
      if (q.direction !== "both" && item.direction !== q.direction) return false;
      if (q.types.length > 0 && !q.types.some((t) => (item.types ?? []).includes(t))) return false;
      return needle === "" || matchesSearch(item, needle);
    });
  // `at` is the place in the list the backend sent (newest first), so a tie keeps that order and "Oldest" is its exact reverse.
  const newestFirst = (a: (typeof kept)[number], b: (typeof kept)[number]) => b.item.createdMs - a.item.createdMs || a.at - b.at;
  const by: Record<SortId, (a: (typeof kept)[number], b: (typeof kept)[number]) => number> = {
    newest: newestFirst,
    oldest: (a, b) => a.item.createdMs - b.item.createdMs || b.at - a.at,
    name: (a, b) => collator.compare(nameOf(a.item), nameOf(b.item)) || newestFirst(a, b),
    size: (a, b) => {
      const textA = a.item.kind === "text";
      const textB = b.item.kind === "text";
      if (textA !== textB) return textA ? 1 : -1;
      return textA ? newestFirst(a, b) : b.item.totalBytes - a.item.totalBytes || newestFirst(a, b);
    },
  };
  return kept.sort(by[q.sort]).map(({ item }) => item);
}

/** One removable chip: `id` says which choice its x resets (see `without`). */
export type Chip = { id: string; label: string };

/** The chips of the choices that are not the default, in the order types, direction, sort, search. */
export function chips(q: Query): Chip[] {
  const out: Chip[] = TYPE_OPTIONS.filter((o) => q.types.includes(o.id)).map((o) => ({ id: `type:${o.id}`, label: o.label }));
  if (q.direction !== "both") out.push({ id: "direction", label: labelOf(DIRECTION_OPTIONS, q.direction) });
  if (q.sort !== "newest") out.push({ id: "sort", label: labelOf(SORT_OPTIONS, q.sort) });
  const words = q.search.trim();
  if (words !== "") out.push({ id: "search", label: `"${words}"` });
  return out;
}

/** The query with the choice of chip `id` reset. */
export function without(q: Query, id: string): Query {
  if (id.startsWith("type:")) return { ...q, types: q.types.filter((t) => `type:${t}` !== id) };
  if (id === "direction") return { ...q, direction: "both" };
  if (id === "sort") return { ...q, sort: "newest" };
  if (id === "search") return { ...q, search: "" };
  return q;
}

/** The query with type `t` switched on or off. */
export function toggleType(q: Query, t: TypeId): Query {
  return { ...q, types: q.types.includes(t) ? q.types.filter((x) => x !== t) : [...q.types, t] };
}
