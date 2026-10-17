// Building requests and reading responses of the native host contract (docs/extension.md).
import { LIMITS, PROTOCOL_VERSION, type BaseRequest, type DeviceInfo, type ErrorCode, type HostRequest, type HostResult, type SendRequest, type StatusInfo } from "./protocol";
import { base64DecodedSize, checkText, checkUrl, cleanTitle, imageNameFromUrl } from "./validate";

export interface Failure {
  ok: false;
  code: ErrorCode;
  message: string;
}
export type Built<T> = { ok: true; request: T } | Failure;

export function newId(): string {
  return globalThis.crypto.randomUUID();
}

function fail(code: ErrorCode, message?: string): Failure {
  return { ok: false, code, message: message ?? describeError(code) };
}

export function buildSimple(type: "status" | "start", id = newId()): HostRequest {
  return { v: PROTOCOL_VERSION, id, type };
}

export function buildOpen(target: "main" | "pairing", id = newId()): HostRequest {
  return { v: PROTOCOL_VERSION, id, type: "open", target };
}

function sendBase(deviceId: string | undefined, id: string): BaseRequest & { type: "send"; deviceId?: string } {
  return deviceId ? { v: PROTOCOL_VERSION, id, type: "send", deviceId } : { v: PROTOCOL_VERSION, id, type: "send" };
}

export function buildSendText(text: string, deviceId?: string, id = newId()): Built<SendRequest> {
  const t = checkText(text);
  if (!t.ok) return fail(t.code, t.code === "TOO_LARGE" ? "That text is too big to send (the limit is 4 MiB)." : "There is no text to send.");
  return { ok: true, request: { ...sendBase(deviceId, id), kind: "text", text: t.text } };
}

export function buildSendLink(
  kind: "link" | "page",
  url: string,
  title: string | undefined,
  opts: { deviceId?: string; timestampSec?: number; id?: string } = {},
): Built<SendRequest> {
  const u = checkUrl(url);
  if (!u.ok) return fail("BAD_URL", u.message);
  const base = sendBase(opts.deviceId, opts.id ?? newId());
  const t = cleanTitle(title);
  if (kind === "page") return { ok: true, request: { ...base, kind: "page", url: u.url, ...(t ? { title: t } : {}) } };
  return {
    ok: true,
    request: {
      ...base,
      kind: "link",
      url: u.url,
      ...(t ? { title: t } : {}),
      ...(opts.timestampSec !== undefined ? { timestampSec: opts.timestampSec } : {}),
    },
  };
}

export function buildSendImageUrl(url: string, pageUrl: string | undefined, deviceId?: string, id = newId()): Built<SendRequest> {
  const u = checkUrl(url);
  if (!u.ok) return fail("BAD_URL", u.message);
  const page = pageUrl !== undefined ? checkUrl(pageUrl) : undefined;
  return {
    ok: true,
    request: {
      ...sendBase(deviceId, id),
      kind: "image",
      url: u.url,
      ...(page?.ok ? { pageUrl: page.url } : {}),
      name: imageNameFromUrl(u.url),
    },
  };
}

export function buildSendImageData(
  data: string,
  mime: string,
  name: string,
  opts: { deviceId?: string; retryOf?: string; id?: string } = {},
): Built<SendRequest> {
  if (data === "") return fail("BAD_REQUEST");
  if (base64DecodedSize(data) > LIMITS.imageBytes) return fail("TOO_LARGE", "That image is too big to send from the browser (the limit is 32 MiB).");
  return {
    ok: true,
    request: {
      ...sendBase(opts.deviceId, opts.id ?? newId()),
      kind: "image",
      data,
      mime,
      name,
      ...(opts.retryOf ? { retryOf: opts.retryOf } : {}),
    },
  };
}

const KNOWN_PATTERNS: [RegExp, ErrorCode][] = [
  [/not found|not registered|no such native|native messaging host .*(not|isn't)/i, "HOST_NOT_INSTALLED"],
  [/forbidden|does not have access/i, "HOST_FORBIDDEN"],
  [/has exited|error when communicating|failed to start|disconnected|closed/i, "HOST_EXITED"],
  [/timed? ?out/i, "TIMEOUT"],
];

/** Maps the text of a runtime.lastError / rejected sendNativeMessage to one of our codes. */
export function classifyNativeError(message: string | undefined): ErrorCode {
  const m = message ?? "";
  for (const [re, code] of KNOWN_PATTERNS) if (re.test(m)) return code;
  return "HOST_EXITED";
}

const DEFAULT_TEXT: Record<ErrorCode, string> = {
  KIN_NOT_RUNNING: "Kin isn't running. Start Kin and try again.",
  HOST_NOT_INSTALLED: "Kin isn't installed, or its browser support is missing. Install or repair Kin.",
  HOST_FORBIDDEN: "This copy of the extension isn't allowed to talk to Kin.",
  HOST_EXITED: "Kin's browser helper stopped unexpectedly.",
  TIMEOUT: "Kin didn't answer in time.",
  NOT_PAIRED: "Pair your phone with Kin first.",
  DEVICE_OFFLINE_QUEUED: "The phone is offline. It will be sent when the phone is back.",
  TOO_LARGE: "That's too big to send.",
  BAD_URL: "Kin can only send web links (http or https).",
  BAD_REQUEST: "Kin didn't understand the request. Update the extension and Kin.",
  UNSUPPORTED_TYPE: "This Kin doesn't support that yet. Update Kin.",
  UNSUPPORTED_VERSION: "The extension and Kin don't speak the same version. Update both.",
  IMAGE_NEEDS_BYTES: "Kin couldn't download that image.",
  IMAGE_UNREACHABLE: "Kin couldn't get that image. If the site needs you to be signed in, save it and send the file instead.",
  NOT_AN_IMAGE: "That doesn't look like an image.",
  INTERNAL: "Something went wrong inside Kin.",
};

export function describeError(code: string, message?: string): string {
  if (message && message.trim() !== "") return message.slice(0, 300);
  return (DEFAULT_TEXT as Record<string, string>)[code] ?? "Something went wrong.";
}

function parseDevices(raw: unknown): DeviceInfo[] {
  if (!Array.isArray(raw)) return [];
  const out: DeviceInfo[] = [];
  for (const d of raw.slice(0, 16)) {
    if (!d || typeof d !== "object") continue;
    const o = d as Record<string, unknown>;
    if (typeof o.id !== "string" || o.id === "" || typeof o.name !== "string") continue;
    const q = typeof o.queuedCount === "number" && Number.isFinite(o.queuedCount) && o.queuedCount > 0 ? Math.floor(o.queuedCount) : 0;
    out.push({ id: o.id, name: o.name.slice(0, 64) || "Phone", online: o.online === true, queuedCount: q });
  }
  return out;
}

/**
 * Checks a raw response against the request it answers. Anything that isn't a well formed v1
 * response to that request becomes a failure result, so callers only ever see HostResult.
 */
export function parseResponse(raw: unknown, request: { id: string; type: string }): HostResult {
  if (!raw || typeof raw !== "object") return { ok: false, code: "HOST_EXITED", message: describeError("HOST_EXITED") };
  const r = raw as Record<string, unknown>;
  if (r.ref !== request.id) {
    // The host answers a request it can't even read without a ref.
    if (r.ok === false && typeof r.code === "string" && r.ref === undefined) return { ok: false, code: r.code, message: describeError(r.code, str(r.message)) };
    return { ok: false, code: "HOST_EXITED", message: describeError("HOST_EXITED") };
  }
  if (r.v !== PROTOCOL_VERSION && r.code !== "UNSUPPORTED_VERSION") {
    return { ok: false, code: "UNSUPPORTED_VERSION", message: describeError("UNSUPPORTED_VERSION") };
  }
  const ok = r.ok === true;
  const code = typeof r.code === "string" ? r.code : undefined;
  const message = describeError(code ?? "", str(r.message));
  const result: HostResult = { ok };
  if (code) result.code = code;
  if (!ok) {
    result.code = code ?? "INTERNAL";
    result.message = describeError(result.code, str(r.message));
    return result;
  }
  if (code) result.message = message;
  if (request.type === "status" || request.type === "start") {
    const status: StatusInfo = {
      running: r.running === true,
      devices: parseDevices(r.devices),
    };
    if (typeof r.kinVersion === "string") status.kinVersion = r.kinVersion;
    if (typeof r.hostVersion === "string") status.hostVersion = r.hostVersion;
    if (typeof r.defaultDeviceId === "string") status.defaultDeviceId = r.defaultDeviceId;
    if (typeof r.paused === "boolean") status.paused = r.paused;
    result.status = status;
  }
  if (request.type === "send") {
    result.queued = r.queued === true;
    if (typeof r.deviceId === "string") result.deviceId = r.deviceId;
    if (typeof r.deviceName === "string") result.deviceName = r.deviceName.slice(0, 64);
  }
  return result;
}

function str(v: unknown): string | undefined {
  return typeof v === "string" ? v : undefined;
}
