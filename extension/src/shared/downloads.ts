// "Download finished" alerts (2B.5, docs/extension.md "Download alerts"). The rules are here and
// pure, so a unit test can run them without a browser: which downloads count, what leaves the
// browser (the file's base name and nothing else) and how a burst is folded into one alert.
//
// This file must never read a download's URL, referrer or folder: a test greps for those words.
import { cleanTitle } from "./validate";

/** Downloads that finish within this window of the first one are folded into one alert. */
export const BURST_MS = 3_000;
/** "Only for downloads over 30 seconds": a download the user watched needs no alert. */
export const LONG_DOWNLOAD_MS = 30_000;
/** A stale "download finished" is worth nothing: Kin drops the alert after 30 minutes. */
export const ALERT_TTL_SEC = 1_800;
export const NAME_MAX = 120;
export const TITLE_MAX = 60;
/** How many handled downloads are remembered for the de-duplication. */
const SEEN_MAX = 200;

export interface DownloadSettings {
  enabled: boolean;
  /** Only downloads that took 30 seconds or more. */
  longOnly: boolean;
}
export const DEFAULT_SETTINGS: DownloadSettings = { enabled: false, longOnly: true };

export function sanitizeSettings(raw: unknown): DownloadSettings {
  const o = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  return {
    enabled: o.enabled === true,
    longOnly: typeof o.longOnly === "boolean" ? o.longOnly : DEFAULT_SETTINGS.longOnly,
  };
}

/** What the extension keeps of a download: nothing about where it came from. */
export interface DownloadFacts {
  state: string;
  error?: string;
  incognito: boolean;
  /** The full path the browser gives. Only its base name is ever used. */
  filename: string;
  startMs?: number;
  endMs?: number;
}

/** Reads exactly the fields above from a browser DownloadItem. */
export function factsFromItem(item: unknown): DownloadFacts | null {
  if (!item || typeof item !== "object") return null;
  const o = item as Record<string, unknown>;
  const time = (v: unknown): number | undefined => {
    const t = typeof v === "string" ? Date.parse(v) : NaN;
    return Number.isFinite(t) ? t : undefined;
  };
  const start = time(o.startTime);
  const end = time(o.endTime);
  return {
    state: typeof o.state === "string" ? o.state : "",
    ...(typeof o.error === "string" ? { error: o.error } : {}),
    incognito: o.incognito === true,
    filename: typeof o.filename === "string" ? o.filename : "",
    ...(start !== undefined ? { startMs: start } : {}),
    ...(end !== undefined ? { endMs: end } : {}),
  };
}

// ALM, LRM, RLM, LRE..RLO and LRI..PDI: the characters that make a name read backwards.
const BIDI = /[\u061C\u200E\u200F\u202A-\u202E\u2066-\u2069]/gu;

function cut(s: string, max: number): string {
  const points = Array.from(s);
  return points.length > max ? points.slice(0, max).join("").trimEnd() : s;
}

/** The file's own name: no folder, no bidi controls, whitespace collapsed, at most 120 characters. */
export function baseName(filename: string): string {
  const last = filename.split(/[\\/]/u).pop() ?? "";
  const cleaned = cleanTitle(last.replace(BIDI, "_"));
  return cut(cleaned ?? "", NAME_MAX);
}

export type Verdict = { alert: true; kind: "finished" | "failed"; name: string } | { alert: false; reason: "not finished" | "private window" | "canceled" | "short" };

/**
 * Decides whether one finished download is worth an alert. `nowMs` is the time of the `onChanged` event that reported
 * it: a browser may leave `endTime` out (Firefox always, Chromium for an interrupted download), and then the download
 * ended when we were told, not never.
 */
export function judge(f: DownloadFacts, longOnly: boolean, nowMs: number = Date.now()): Verdict {
  const finished = f.state === "complete";
  if (!finished && f.state !== "interrupted") return { alert: false, reason: "not finished" };
  if (f.incognito) return { alert: false, reason: "private window" };
  // The user pressed Cancel: they know.
  if (!finished && f.error === "USER_CANCELED") return { alert: false, reason: "canceled" };
  if (longOnly) {
    const took = f.startMs !== undefined ? (f.endMs ?? nowMs) - f.startMs : -1;
    if (took < LONG_DOWNLOAD_MS) return { alert: false, reason: "short" };
  }
  return { alert: true, kind: finished ? "finished" : "failed", name: baseName(f.filename) || "A file" };
}

export interface Burst {
  finished: string[];
  failed: string[];
}

/** One alert for what finished in a burst: the file name for one download, counts for several. */
export function composeAlert(b: Burst): { title: string; text: string } | null {
  const n = b.finished.length;
  const m = b.failed.length;
  if (n === 0 && m === 0) return null;
  // One download: the singular title and the file's name. Several: the plural title and a count (protocol/alerts.json `downloadAlert[]`).
  if (m === 0) return n === 1 ? { title: "Download finished", text: b.finished[0]! } : { title: "Downloads finished", text: `${n} downloads finished` };
  if (n === 0) return m === 1 ? { title: "Download failed", text: b.failed[0]! } : { title: "Downloads failed", text: `${m} downloads failed` };
  return { title: "Downloads finished", text: `${n} finished, ${m} failed` };
}

export interface ChangeDelta {
  id: number;
  state?: { current?: string };
}

export interface AlerterDeps {
  settings(): Promise<DownloadSettings>;
  /** The facts of one download, or null if the browser no longer knows it. */
  lookup(id: number): Promise<DownloadFacts | null>;
  schedule(fn: () => void, ms: number): unknown;
  /** Sends the alert. Never throws. */
  deliver(alert: { title: string; text: string }): Promise<void>;
  /** The clock, for downloads that come without an end time. Defaults to `Date.now`. */
  now?(): number;
  /** Called when an event arrives while the switch is off: the listener has no reason to stay registered. */
  disabled?(): void;
}

/**
 * The brain of the listener: filters, de-duplicates and folds bursts. A download is remembered as `id:state`, not
 * as `id`: one that is interrupted and later resumed to completion is two events and gives two alerts (a failure,
 * then the finish), which is what the user wants to know.
 */
export function createDownloadAlerter(deps: AlerterDeps) {
  const seen = new Set<string>();
  let pending: Burst = { finished: [], failed: [] };
  let timer: unknown;

  async function flush(): Promise<void> {
    const burst = pending;
    pending = { finished: [], failed: [] };
    timer = undefined;
    try {
      // Turned off while the burst was filling: nothing leaves.
      if (!(await deps.settings()).enabled) return;
      const alert = composeAlert(burst);
      if (alert) await deps.deliver(alert);
    } catch {
      // an alert is best effort
    }
  }

  async function onChanged(delta: ChangeDelta): Promise<void> {
    const state = delta.state?.current;
    if (state !== "complete" && state !== "interrupted") return;
    const at = (deps.now ?? Date.now)();
    try {
      const s = await deps.settings();
      if (!s.enabled) {
        // Nothing is remembered while off, so turning the switch on later doesn't find old downloads "seen".
        deps.disabled?.();
        return;
      }
      const key = `${delta.id}:${state}`;
      if (seen.has(key)) return;
      seen.add(key);
      if (seen.size > SEEN_MAX) seen.delete(seen.values().next().value as string);
      const facts = await deps.lookup(delta.id);
      if (!facts) return;
      const v = judge(facts, s.longOnly, at);
      if (!v.alert) return;
      (v.kind === "finished" ? pending.finished : pending.failed).push(v.name);
      if (timer === undefined) timer = deps.schedule(() => void flush(), BURST_MS);
    } catch {
      // a failed lookup is a missed alert, never an error
    }
  }

  return { onChanged };
}

/** The part of a browser's `downloads` API the listener needs. */
export interface DownloadsApi {
  onChanged?: {
    addListener(l: (delta: ChangeDelta) => void): void;
    removeListener?(l: (delta: ChangeDelta) => void): void;
  };
}

/**
 * Registers the listener when the browser has the API, and only then. Without the permission
 * `chrome.downloads` doesn't exist, so there is nothing to register and no listener at all.
 * Safe to call again: it registers once.
 */
export function createRegistrar(listener: (delta: ChangeDelta) => void) {
  let active: DownloadsApi["onChanged"];
  return {
    register(api: DownloadsApi | undefined): boolean {
      if (active || !api?.onChanged || typeof api.onChanged.addListener !== "function") return false;
      api.onChanged.addListener(listener);
      active = api.onChanged;
      return true;
    },
    /** The permission was removed or the switch turned off. */
    release(): void {
      try {
        active?.removeListener?.(listener);
      } catch {
        // the API is already gone with the permission
      }
      active = undefined;
    },
    get registered(): boolean {
      return active !== undefined;
    },
  };
}
