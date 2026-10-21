// Status and send flows. Runs in the background (service worker or event page).
import { BADGE_MS, HISTORY_MAX, SEND_TIMEOUT_MS, START_TIMEOUT_MS, STATUS_TIMEOUT_MS } from "./shared/config";
import { bytesToBase64 } from "./shared/base64";
import { addHistory, DOWNLOAD_ALERT_LABEL, labelForImage, labelForText, labelForUrl, sanitizeHistory, type HistoryEntry, type Outcome } from "./shared/history";
import { buildOpen, buildSendAlert, buildSendImageData, buildSendImageUrl, buildSendLink, buildSendText, buildSimple, describeError, type Failure } from "./shared/messages";
import { ALERT_TTL_SEC, sanitizeSettings, type DownloadSettings } from "./shared/downloads";
import { pickDevice } from "./shared/menus";
import { LIMITS, type HostResult, type SendRequest, type StatusInfo } from "./shared/protocol";
import { checkUrl, imageNameFromUrl } from "./shared/validate";
import { shouldStamp, usableTime, withTimestamp } from "./shared/youtube";
import { fetchImageAsBase64, readVideoState, type PageFetchResult } from "./inject";
import { callHost, ext } from "./native";

// ---- storage --------------------------------------------------------------------------------

const sessionArea = (ext.storage as { session?: chrome.storage.StorageArea }).session ?? ext.storage.local;

export interface StatusView {
  ok: boolean;
  status?: StatusInfo;
  code?: string;
  message?: string;
}

export async function getCachedStatus(): Promise<StatusInfo | null> {
  const v = (await sessionArea.get("status")) as { status?: StatusInfo };
  return v.status ?? null;
}

export async function getSelectedDevice(): Promise<string | undefined> {
  const v = (await ext.storage.local.get("selectedDeviceId")) as { selectedDeviceId?: string };
  return typeof v.selectedDeviceId === "string" ? v.selectedDeviceId : undefined;
}

export async function setSelectedDevice(id: string | undefined): Promise<void> {
  if (id) await ext.storage.local.set({ selectedDeviceId: id });
  else await ext.storage.local.remove("selectedDeviceId");
}

export async function getHistory(): Promise<HistoryEntry[]> {
  const v = (await ext.storage.local.get("history")) as { history?: unknown };
  return sanitizeHistory(v.history);
}

export async function clearHistory(): Promise<void> {
  await ext.storage.local.remove(["history", "lastResult"]);
}

export interface LastResult {
  at: number;
  outcome: Outcome;
  message: string;
  code?: string;
}

export async function getLastResult(): Promise<LastResult | null> {
  const v = (await ext.storage.local.get("lastResult")) as { lastResult?: LastResult };
  return v.lastResult ?? null;
}

async function record(entry: HistoryEntry, code?: string): Promise<void> {
  const list = await getHistory();
  const lastResult: LastResult = { at: entry.at, outcome: entry.outcome, message: entry.message ?? "", ...(code ? { code } : {}) };
  await ext.storage.local.set({ history: addHistory(list, entry, HISTORY_MAX), lastResult });
}

// ---- status ---------------------------------------------------------------------------------

type StatusListener = (status: StatusInfo | null) => void;
let onStatus: StatusListener = () => {};
/** The background registers what to do when the status changes (it rebuilds the menu). */
export function setStatusListener(l: StatusListener): void {
  onStatus = l;
}

async function applyStatus(result: HostResult): Promise<StatusView> {
  if (result.ok && result.status) {
    await sessionArea.set({ status: result.status });
    onStatus(result.status);
    return { ok: true, status: result.status };
  }
  await sessionArea.remove("status");
  onStatus(null);
  return { ok: false, code: result.code ?? "INTERNAL", message: result.message ?? describeError(result.code ?? "INTERNAL") };
}

/** Asks Kin what it is doing. Never starts Kin. */
export async function refreshStatus(): Promise<StatusView> {
  return applyStatus(await callHost(buildSimple("status"), STATUS_TIMEOUT_MS));
}

/** The "Start Kin" button. */
export async function startKin(): Promise<StatusView> {
  const result = await callHost(buildSimple("start"), START_TIMEOUT_MS);
  return applyStatus(result);
}

export async function openKin(target: "main" | "pairing"): Promise<HostResult> {
  return callHost(buildOpen(target), STATUS_TIMEOUT_MS);
}

// ---- sending --------------------------------------------------------------------------------

export type SendSpec =
  | { kind: "text"; text: string }
  | { kind: "link"; url: string; title?: string }
  | { kind: "page"; url: string; title?: string }
  | { kind: "image"; srcUrl: string; pageUrl?: string };

export interface SendContext {
  tab?: { id?: number; url?: string; title?: string };
  frameId?: number;
  deviceId?: string;
}

export interface SendOutcome {
  outcome: Outcome;
  message: string;
  code?: string;
  deviceName?: string;
}

function failed(code: string, message?: string, deviceName?: string): SendOutcome {
  return { outcome: "failed", code, message: describeError(code, message), ...(deviceName ? { deviceName } : {}) };
}

async function deviceNameFor(id: string | undefined): Promise<string | undefined> {
  const status = await getCachedStatus();
  return status?.devices.find((d) => d.id === id)?.name;
}

async function runInPage<A extends unknown[], R>(ctx: SendContext, func: (...args: A) => R, args: A): Promise<Awaited<R> | undefined> {
  const tabId = ctx.tab?.id;
  if (tabId === undefined) return undefined;
  const frames = ctx.frameId ? [ctx.frameId] : [0];
  const attempt = async (frameIds: number[]) => {
    const res = await ext.scripting.executeScript({ target: { tabId, frameIds }, func, args });
    return res[0]?.result as Awaited<R> | undefined;
  };
  try {
    return await attempt(frames);
  } catch {
    if (frames[0] === 0) return undefined;
    try {
      return await attempt([0]);
    } catch {
      return undefined;
    }
  }
}

/** Adds t=<seconds>s to a link to the video the tab is playing. Never fails the send. */
async function stampYoutube(url: string, ctx: SendContext): Promise<{ url: string; timestampSec?: number }> {
  const checked = checkUrl(url);
  if (!checked.ok || !shouldStamp(checked.url, ctx.tab?.url)) return { url };
  const state = await runInPage({ ...ctx, frameId: 0 }, readVideoState, []);
  const t = usableTime(state ?? null);
  return t === null ? { url } : { url: withTimestamp(checked.url, t), timestampSec: t };
}

function interpret(result: HostResult, device: string | undefined): SendOutcome {
  const deviceName = result.deviceName ?? device;
  if (!result.ok) return failed(result.code ?? "INTERNAL", result.message, deviceName);
  if (result.queued) {
    return { outcome: "queued", code: "DEVICE_OFFLINE_QUEUED", message: describeError("DEVICE_OFFLINE_QUEUED", result.message), ...(deviceName ? { deviceName } : {}) };
  }
  return { outcome: "sent", message: deviceName ? `Sent to ${deviceName}` : "Sent to your phone", ...(deviceName ? { deviceName } : {}) };
}

async function sendBuilt(built: SendRequest | Failure, device: string | undefined): Promise<{ result: HostResult; request?: SendRequest }> {
  if ("ok" in built && built.ok === false) return { result: { ok: false, code: built.code, message: built.message } };
  const request = built as SendRequest;
  return { result: await callHost(request, SEND_TIMEOUT_MS), request };
}

async function sendImage(spec: Extract<SendSpec, { kind: "image" }>, ctx: SendContext): Promise<SendOutcome> {
  const device = await deviceNameFor(ctx.deviceId);
  const src = spec.srcUrl;
  const viaPage = async (retryOf?: string): Promise<SendOutcome> => {
    const fetched = (await runInPage(ctx, fetchImageAsBase64, [src, LIMITS.imageBytes])) as PageFetchResult | undefined;
    if (!fetched) return failed("IMAGE_UNREACHABLE", undefined, device);
    if (!fetched.ok) {
      if (fetched.reason === "too_large") return failed("TOO_LARGE", "That image is too big to send from the browser (the limit is 32 MiB).", device);
      if (fetched.reason === "not_image") return failed("NOT_AN_IMAGE", undefined, device);
      return failed("IMAGE_UNREACHABLE", undefined, device);
    }
    const built = buildSendImageData(fetched.data, fetched.mime, imageNameFromUrl(src, fetched.mime), {
      ...(ctx.deviceId ? { deviceId: ctx.deviceId } : {}),
      ...(retryOf ? { retryOf } : {}),
    });
    const { result } = await sendBuilt(built.ok ? built.request : built, device);
    return interpret(result, device);
  };

  if (src.startsWith("data:")) {
    try {
      const blob = await (await fetch(src)).blob();
      if (blob.size > LIMITS.imageBytes) return failed("TOO_LARGE", "That image is too big to send from the browser (the limit is 32 MiB).", device);
      const mime = blob.type || "application/octet-stream";
      const built = buildSendImageData(bytesToBase64(new Uint8Array(await blob.arrayBuffer())), mime, imageNameFromUrl("https://x/image", mime), ctx.deviceId ? { deviceId: ctx.deviceId } : {});
      const { result } = await sendBuilt(built.ok ? built.request : built, device);
      return interpret(result, device);
    } catch {
      return failed("BAD_URL", undefined, device);
    }
  }
  if (src.startsWith("blob:")) return viaPage();

  const built = buildSendImageUrl(src, spec.pageUrl ?? ctx.tab?.url, ctx.deviceId);
  const first = await sendBuilt(built.ok ? built.request : built, device);
  if (!first.result.ok && first.result.code === "IMAGE_NEEDS_BYTES" && first.request) return viaPage(first.request.id);
  return interpret(first.result, device);
}

function labelFor(spec: SendSpec): { kind: HistoryEntry["kind"]; label: string } {
  switch (spec.kind) {
    case "text":
      return { kind: "text", label: labelForText(spec.text) };
    case "link":
    case "page":
      return { kind: spec.kind, label: labelForUrl(spec.title, spec.url) };
    case "image":
      return { kind: "image", label: labelForImage(spec.srcUrl) };
  }
}

/** Sends one thing, records it in the history and returns what to tell the user. */
export async function performSend(spec: SendSpec, ctx: SendContext = {}): Promise<SendOutcome> {
  let outcome: SendOutcome;
  const device = await deviceNameFor(ctx.deviceId);
  if (spec.kind === "image") {
    outcome = await sendImage(spec, ctx);
  } else if (spec.kind === "text") {
    const built = buildSendText(spec.text, ctx.deviceId);
    const { result } = await sendBuilt(built.ok ? built.request : built, device);
    outcome = interpret(result, device);
  } else {
    const stamped = await stampYoutube(spec.url, ctx);
    const built = buildSendLink(spec.kind, stamped.url, spec.title, {
      ...(ctx.deviceId ? { deviceId: ctx.deviceId } : {}),
      ...(stamped.timestampSec !== undefined ? { timestampSec: stamped.timestampSec } : {}),
    });
    const { result } = await sendBuilt(built.ok ? built.request : built, device);
    outcome = interpret(result, device);
  }
  const { kind, label } = labelFor(spec);
  await record({
    at: Date.now(),
    kind,
    label,
    outcome: outcome.outcome,
    ...(outcome.deviceName ? { device: outcome.deviceName } : {}),
    ...(outcome.outcome !== "sent" ? { message: outcome.message } : {}),
  }, outcome.code);
  void showBadge(outcome, ctx.tab?.id);
  // The queue length and online state changed; keep the menu honest.
  void refreshStatus();
  return outcome;
}

/** Sends the tab the user is looking at. `deviceId` comes from the popup's choice or the saved one. */
export async function sendTab(tab: { id?: number; url?: string; title?: string } | undefined, deviceId?: string): Promise<SendOutcome> {
  let target = tab;
  if (!target?.url) {
    const [active] = await ext.tabs.query({ active: true, lastFocusedWindow: true });
    target = active;
  }
  const checked = target?.url ? checkUrl(target.url) : undefined;
  if (!target?.url || !checked?.ok) {
    const o = failed("BAD_URL", checked && !checked.ok && checked.reason !== "not http or https" ? `This page can't be sent. ${checked.message}` : "This page can't be sent: Kin sends web pages (http or https) only.");
    await record({ at: Date.now(), kind: "page", label: "This tab", outcome: "failed", message: o.message }, o.code);
    void showBadge(o, target?.id);
    return o;
  }
  const chosen = deviceId ?? pickDevice(await getCachedStatus(), await getSelectedDevice());
  return performSend({ kind: "page", url: target.url, ...(target.title ? { title: target.title } : {}) }, { tab: target, ...(chosen ? { deviceId: chosen } : {}) });
}

// ---- download alerts ------------------------------------------------------------------------

export async function getDownloadSettings(): Promise<DownloadSettings> {
  const v = (await ext.storage.local.get("downloadAlerts")) as { downloadAlerts?: unknown };
  return sanitizeSettings(v.downloadAlerts);
}

export async function setDownloadSettings(patch: Partial<DownloadSettings>): Promise<DownloadSettings> {
  const next = { ...(await getDownloadSettings()), ...patch };
  await ext.storage.local.set({ downloadAlerts: next });
  return next;
}

/**
 * Sends one "Download finished" alert. It asks Kin for its status first (that never starts Kin) so a closed Kin
 * or a Kin with no phone costs one cheap request and no history entry; it also gives the device to address. The
 * status check is not what keeps Kin closed: the request carries `auto:true`, and the native host never starts
 * Kin for an automatic alert (it answers KIN_NOT_RUNNING), so a Kin that quits between the two steps stays quit.
 * Kin paused answers PAUSED_SKIPPED, which is a notice and leaves no trace. Returns undefined when nothing was sent.
 */
export async function sendDownloadAlert(alert: { title: string; text: string }): Promise<SendOutcome | undefined> {
  const view = await refreshStatus();
  if (!view.ok || !view.status?.running || view.status.devices.length === 0) return undefined;
  const deviceId = pickDevice(view.status, await getSelectedDevice());
  const built = buildSendAlert(alert.title, alert.text, { source: "downloads", ttlSec: ALERT_TTL_SEC, ...(deviceId ? { deviceId } : {}) });
  if (!built.ok) return undefined;
  const result = await callHost(built.request, SEND_TIMEOUT_MS);
  if (result.ok && result.code === "PAUSED_SKIPPED") return undefined;
  const device = view.status.devices.find((d) => d.id === deviceId)?.name;
  const outcome = interpret(result, device);
  const list = await getHistory();
  const entry: HistoryEntry = {
    at: Date.now(),
    kind: "download",
    label: DOWNLOAD_ALERT_LABEL,
    outcome: outcome.outcome,
    ...(outcome.deviceName ? { device: outcome.deviceName } : {}),
    ...(outcome.outcome !== "sent" ? { message: outcome.message } : {}),
  };
  // History only: no toolbar badge and no "last result" banner for something the user didn't just do.
  await ext.storage.local.set({ history: addHistory(list, entry, HISTORY_MAX) });
  return outcome;
}

// ---- toolbar feedback -----------------------------------------------------------------------

const BADGE: Record<Outcome, { text: string; color: string }> = {
  sent: { text: "✓", color: "#16a34a" },
  queued: { text: "Q", color: "#d97706" },
  failed: { text: "!", color: "#dc2626" },
};

async function showBadge(o: SendOutcome, tabId?: number): Promise<void> {
  try {
    const b = BADGE[o.outcome];
    const scope = tabId !== undefined ? { tabId } : {};
    await ext.action.setBadgeBackgroundColor({ color: b.color, ...scope });
    await ext.action.setBadgeText({ text: b.text, ...scope });
    await ext.action.setTitle({ title: `Kin: ${o.message}`, ...scope });
    setTimeout(() => {
      void ext.action.setBadgeText({ text: "", ...scope });
      void ext.action.setTitle({ title: "Send to Kin", ...scope });
    }, BADGE_MS);
  } catch {
    // the tab may be gone; the popup still shows the result
  }
}
