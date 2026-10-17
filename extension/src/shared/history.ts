// The last few sends, kept in the browser's local storage. Only labels are kept, never the content:
// a link shows its title or host, a text shows "Text, 42 characters".
import { HISTORY_MAX } from "./config";

export type Outcome = "sent" | "queued" | "failed";

export interface HistoryEntry {
  /** Unix milliseconds. */
  at: number;
  kind: "text" | "link" | "page" | "image";
  label: string;
  device?: string;
  outcome: Outcome;
  /** For a failure: the user-facing message. */
  message?: string;
}

export function addHistory(list: HistoryEntry[], entry: HistoryEntry, max = HISTORY_MAX): HistoryEntry[] {
  return [entry, ...list].slice(0, max);
}

export function labelForText(text: string): string {
  const n = Array.from(text).length;
  return `Text, ${n} character${n === 1 ? "" : "s"}`;
}

export function labelForUrl(title: string | undefined, url: string | undefined): string {
  if (title && title.trim() !== "") return Array.from(title.trim()).slice(0, 80).join("");
  try {
    return new URL(url ?? "").hostname || "Link";
  } catch {
    return "Link";
  }
}

export function labelForImage(url: string | undefined): string {
  // data: and blob: URLs carry no useful name (and a data: URL is the picture itself).
  if (!url || !/^https?:/i.test(url)) return "Image";
  try {
    const last = new URL(url ?? "").pathname.split("/").filter(Boolean).pop();
    return last ? `Image ${decodeURIComponent(last).slice(0, 60)}` : "Image";
  } catch {
    return "Image";
  }
}

export function sanitizeHistory(raw: unknown): HistoryEntry[] {
  if (!Array.isArray(raw)) return [];
  const out: HistoryEntry[] = [];
  for (const e of raw) {
    if (!e || typeof e !== "object") continue;
    const o = e as Record<string, unknown>;
    if (typeof o.at !== "number" || typeof o.label !== "string") continue;
    if (o.outcome !== "sent" && o.outcome !== "queued" && o.outcome !== "failed") continue;
    if (o.kind !== "text" && o.kind !== "link" && o.kind !== "page" && o.kind !== "image") continue;
    out.push({
      at: o.at,
      kind: o.kind,
      label: o.label.slice(0, 120),
      outcome: o.outcome,
      ...(typeof o.device === "string" ? { device: o.device.slice(0, 64) } : {}),
      ...(typeof o.message === "string" ? { message: o.message.slice(0, 300) } : {}),
    });
  }
  return out.slice(0, HISTORY_MAX);
}
