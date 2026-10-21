// A mock of kin-native-host.exe + Kin, speaking the native messaging framing and answering per
// docs/extension.md. Run by a .cmd wrapper that the smoke test registers:
//   bun mock-host.ts --state <dir> [browser args...]
// <dir>/scenario.json picks the behavior (re-read for every request), <dir>/log.jsonl gets one line
// per request received (images are logged as size + sha256, never the bytes).
//
// It is also the reference for what a real host must answer: every request type, every error code.
import { createHash } from "node:crypto";
import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const stateDir = process.argv[process.argv.indexOf("--state") + 1]!;
const MAX_REQUEST = 64 * 1024 * 1024;
const MAX_RESPONSE = 1024 * 1024;

interface Device {
  id: string;
  name: string;
  online: boolean;
  queuedCount: number;
}
interface Scenario {
  running: boolean;
  /** `start` makes Kin run. */
  startable: boolean;
  devices: Device[];
  defaultDeviceId?: string;
  /** The PC is paused (2B.4): `status.paused`, and an `auto` alert is answered PAUSED_SKIPPED. */
  paused?: boolean;
  /** Force a failure for every send. */
  sendError?: { code: string; message: string };
  /** Exit without answering (a crashing host). */
  exitWithoutAnswer?: boolean;
  /** Answer with a different ref (a broken host). */
  badRef?: boolean;
  /** Milliseconds to wait before answering. */
  delayMs?: number;
}

const defaults: Scenario = { running: true, startable: true, devices: [], };

function scenario(): Scenario {
  const p = join(stateDir, "scenario.json");
  return { ...defaults, ...(existsSync(p) ? JSON.parse(readFileSync(p, "utf8")) : {}) };
}
function saveScenario(s: Scenario): void {
  writeFileSync(join(stateDir, "scenario.json"), JSON.stringify(s));
}
function log(entry: unknown): void {
  appendFileSync(join(stateDir, "log.jsonl"), JSON.stringify(entry) + "\n");
}

type Json = Record<string, unknown>;
const err = (ref: string | undefined, type: string, code: string, message: string): Json => ({ v: 1, ...(ref ? { ref } : {}), type, ok: false, code, message });

function statusBody(s: Scenario): Json {
  if (!s.running) return { running: false, hostVersion: "0.0.0-mock", devices: [] };
  return { running: true, kinVersion: "0.0.0-mock", hostVersion: "0.0.0-mock", devices: s.devices, ...(s.defaultDeviceId ? { defaultDeviceId: s.defaultDeviceId } : {}), ...(s.paused ? { paused: true } : {}) };
}

function isHttpUrl(u: unknown): u is string {
  if (typeof u !== "string" || u.length > 8192 || /[\s\u0000-\u001f\u007f-\u009f]/u.test(u)) return false;
  try {
    const p = new URL(u);
    return p.protocol === "http:" || p.protocol === "https:";
  } catch {
    return false;
  }
}

function looksLikeImage(b: Uint8Array): boolean {
  const h = Buffer.from(b.subarray(0, 12));
  return h.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) || (h[0] === 0xff && h[1] === 0xd8) || h.subarray(0, 3).toString("latin1") === "GIF";
}

/** What Kin does with an image URL: GET without cookies, check the bytes. */
async function downloadImage(url: string, referer: string | undefined): Promise<{ ok: true; size: number } | { ok: false }> {
  try {
    const resp = await fetch(url, { headers: referer ? { Referer: referer } : {}, signal: AbortSignal.timeout(10_000) });
    if (!resp.ok) return { ok: false };
    const bytes = new Uint8Array(await resp.arrayBuffer());
    if (bytes.length > 64 * 1024 * 1024 || !looksLikeImage(bytes)) return { ok: false };
    return { ok: true, size: bytes.length };
  } catch {
    return { ok: false };
  }
}

async function handle(req: Json, s: Scenario): Promise<Json> {
  const id = typeof req.id === "string" && req.id.length >= 1 && req.id.length <= 64 ? req.id : undefined;
  if (!id || typeof req.type !== "string") return err(undefined, "error", "BAD_REQUEST", "The request has no id or type.");
  if (req.v !== 1) return { v: 1, ref: id, type: "error", ok: false, code: "UNSUPPORTED_VERSION", supported: [1], message: "Update Kin or the extension." };
  const ok = (type: string, extra: Json = {}): Json => ({ v: 1, ref: id, type, ok: true, ...extra });
  const bad = (code: string, message: string): Json => err(id, req.type as string, code, message);

  switch (req.type) {
    case "status":
      return ok("status", statusBody(s));
    case "start": {
      if (!s.running) {
        if (!s.startable) return bad("KIN_NOT_RUNNING", "Kin couldn't be started.");
        s.running = true;
        saveScenario(s);
      }
      return ok("start", statusBody(s));
    }
    case "open":
      if (!s.running) return bad("KIN_NOT_RUNNING", "Kin isn't running.");
      return ok("open");
    case "send":
      break;
    default:
      return bad("UNSUPPORTED_TYPE", "Unknown request type.");
  }

  // send
  if (!s.running) {
    // Like the real host: an automatic alert (a finished download) never starts Kin.
    if (!s.startable || (req.kind === "alert" && req.auto === true)) return bad("KIN_NOT_RUNNING", "Kin isn't running and couldn't be started.");
    s.running = true; // the host starts Kin and goes on
    saveScenario(s);
  }
  if (s.sendError) return bad(s.sendError.code, s.sendError.message);
  if (s.devices.length === 0) return bad("NOT_PAIRED", "Pair your phone with Kin first.");
  const wanted = typeof req.deviceId === "string" ? req.deviceId : s.defaultDeviceId ?? s.devices[0]!.id;
  const device = s.devices.find((d) => d.id === wanted);
  if (!device) return bad("NOT_PAIRED", "That phone isn't paired.");

  const sent = (): Json => {
    const notice = device.online ? {} : { code: "DEVICE_OFFLINE_QUEUED", message: `${device.name} is offline. It will be sent when the phone is back.` };
    return ok("send", { queued: !device.online, deviceId: device.id, deviceName: device.name, ...notice });
  };

  switch (req.kind) {
    case "text": {
      if (typeof req.text !== "string" || req.text === "") return bad("BAD_REQUEST", "No text.");
      if (Buffer.byteLength(req.text) > 4 * 1024 * 1024) return bad("TOO_LARGE", "That text is too big.");
      return sent();
    }
    case "link":
    case "page": {
      if (!isHttpUrl(req.url)) return bad("BAD_URL", "Kin can only send web links (http or https).");
      if (req.title !== undefined && (typeof req.title !== "string" || Array.from(req.title).length > 300)) return bad("BAD_REQUEST", "Bad title.");
      return sent();
    }
    case "image": {
      const hasUrl = req.url !== undefined;
      const hasData = req.data !== undefined;
      if (hasUrl === hasData) return bad("BAD_REQUEST", "Exactly one of url and data.");
      if (hasUrl) {
        if (!isHttpUrl(req.url)) return bad("BAD_URL", "Kin can only send web links (http or https).");
        const dl = await downloadImage(req.url, typeof req.pageUrl === "string" ? req.pageUrl : undefined);
        if (!dl.ok) return bad("IMAGE_NEEDS_BYTES", "The site wouldn't give Kin the picture.");
        return sent();
      }
      if (typeof req.data !== "string" || typeof req.mime !== "string" || typeof req.name !== "string") return bad("BAD_REQUEST", "Bad image.");
      const bytes = Buffer.from(req.data, "base64");
      if (bytes.length > 32 * 1024 * 1024) return bad("TOO_LARGE", "That image is too big.");
      if (!looksLikeImage(bytes)) return bad("NOT_AN_IMAGE", "That doesn't look like an image.");
      return sent();
    }
    case "alert": {
      // docs/extension.md "send" kind alert (2B.5): text 1 to 300 characters after trimming, title up to 60,
      // ttlSec 60 to 604800 (default 86400), source cli or downloads, auto a boolean.
      const text = typeof req.text === "string" ? req.text.trim() : "";
      if (text === "" || Array.from(text).length > 300) return bad("BAD_REQUEST", "The alert text must be 1 to 300 characters.");
      if (req.title !== undefined && (typeof req.title !== "string" || Array.from(req.title.trim()).length > 60)) return bad("BAD_REQUEST", "Bad title.");
      if (req.ttlSec !== undefined && !(Number.isInteger(req.ttlSec) && (req.ttlSec as number) >= 60 && (req.ttlSec as number) <= 604_800)) return bad("BAD_REQUEST", "Bad ttlSec.");
      if (req.source !== undefined && req.source !== "cli" && req.source !== "downloads") return bad("BAD_REQUEST", "Bad source.");
      if (req.auto !== undefined && typeof req.auto !== "boolean") return bad("BAD_REQUEST", "Bad auto.");
      // An event-driven alert is skipped while the PC is paused: a success notice, nothing is sent.
      if (req.auto === true && s.paused) return ok("send", { queued: false, deviceId: device.id, deviceName: device.name, code: "PAUSED_SKIPPED", message: "Kin is paused, so it skipped this alert." });
      return sent();
    }
    default:
      return bad("BAD_REQUEST", "Unknown kind.");
  }
}

function summarize(req: Json): Json {
  const copy: Json = { ...req };
  if (typeof copy.data === "string") {
    const bytes = Buffer.from(copy.data, "base64");
    copy.data = `<${bytes.length} bytes>`;
    copy.dataSha256 = createHash("sha256").update(bytes).digest("hex");
  }
  return copy;
}

async function* frames(): AsyncGenerator<Buffer> {
  let buf = Buffer.alloc(0);
  for await (const chunk of Bun.stdin.stream()) {
    buf = Buffer.concat([buf, chunk]);
    while (buf.length >= 4) {
      const len = buf.readUInt32LE(0);
      if (len > MAX_REQUEST) {
        log({ error: "request too large", len });
        return;
      }
      if (buf.length < 4 + len) break;
      yield buf.subarray(4, 4 + len);
      buf = buf.subarray(4 + len);
    }
  }
}

function write(msg: Json): Promise<void> {
  const body = Buffer.from(JSON.stringify(msg), "utf8");
  if (body.length > MAX_RESPONSE) throw new Error("response over 1 MB");
  const head = Buffer.alloc(4);
  head.writeUInt32LE(body.length, 0);
  return new Promise((resolve, reject) => process.stdout.write(Buffer.concat([head, body]), (e) => (e ? reject(e) : resolve())));
}

for await (const frame of frames()) {
  const s = scenario();
  let req: Json;
  try {
    req = JSON.parse(frame.toString("utf8")) as Json;
  } catch {
    log({ error: "bad json" });
    await write(err(undefined, "error", "BAD_REQUEST", "The request isn't valid JSON."));
    continue;
  }
  log({ at: Date.now(), origin: process.argv[2], req: summarize(req) });
  if (s.exitWithoutAnswer) process.exit(1);
  if (s.delayMs) await new Promise((r) => setTimeout(r, s.delayMs));
  const response = await handle(req, s);
  if (s.badRef) response.ref = "someone-else";
  log({ at: Date.now(), res: response });
  await write(response);
}
