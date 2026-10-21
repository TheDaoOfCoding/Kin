// The mock host is the reference answer sheet for docs/extension.md. These tests run it the way a
// browser does (framed JSON on stdin and stdout), so they double as a conformance check for any
// real host: feed it the same requests, expect the same answers.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const mock = join(import.meta.dir, "..", "smoke", "mock-host.ts");
let dir = "";

beforeAll(() => {
  dir = mkdtempSync(join(tmpdir(), "kin-mock-test-"));
});
afterAll(() => rmSync(dir, { recursive: true, force: true }));

const PIXEL = { id: "px", name: "Pixel 8", online: true, queuedCount: 0 };
const GALAXY = { id: "gx", name: "Galaxy", online: false, queuedCount: 1 };

function frame(obj: unknown): Buffer {
  const body = Buffer.from(typeof obj === "string" ? obj : JSON.stringify(obj));
  const head = Buffer.alloc(4);
  head.writeUInt32LE(body.length, 0);
  return Buffer.concat([head, body]);
}

async function exchange(scenario: Record<string, unknown>, requests: unknown[]): Promise<any[]> {
  writeFileSync(join(dir, "scenario.json"), JSON.stringify(scenario));
  const proc = Bun.spawn(["bun", mock, "--state", dir, "chrome-extension://test/"], { stdin: "pipe", stdout: "pipe", stderr: "pipe" });
  for (const r of requests) proc.stdin.write(frame(r));
  await proc.stdin.end();
  const out = Buffer.from(await new Response(proc.stdout).arrayBuffer());
  await proc.exited;
  const answers: any[] = [];
  for (let o = 0; o < out.length; ) {
    const len = out.readUInt32LE(o);
    answers.push(JSON.parse(out.subarray(o + 4, o + 4 + len).toString("utf8")));
    o += 4 + len;
  }
  return answers;
}
const one = async (scenario: Record<string, unknown>, req: unknown) => (await exchange(scenario, [req]))[0];
const base = { running: true, startable: true, devices: [PIXEL, GALAXY], defaultDeviceId: "px" };

describe("status, start, open", () => {
  test("status when running lists the devices", async () => {
    const r = await one(base, { v: 1, id: "a", type: "status" });
    expect(r).toMatchObject({ v: 1, ref: "a", type: "status", ok: true, running: true, defaultDeviceId: "px" });
    expect(r.devices).toEqual([PIXEL, GALAXY]);
  });
  test("status when Kin isn't running is ok:true, running:false", async () => {
    const r = await one({ ...base, running: false }, { v: 1, id: "a", type: "status" });
    expect(r).toMatchObject({ ok: true, running: false, devices: [] });
  });
  test("start runs Kin; if it can't, KIN_NOT_RUNNING", async () => {
    expect(await one({ ...base, running: false }, { v: 1, id: "a", type: "start" })).toMatchObject({ ok: true, type: "start", running: true });
    expect(await one({ ...base, running: false, startable: false }, { v: 1, id: "a", type: "start" })).toMatchObject({ ok: false, code: "KIN_NOT_RUNNING" });
  });
  test("open", async () => {
    expect(await one(base, { v: 1, id: "a", type: "open", target: "pairing" })).toMatchObject({ ok: true });
    expect(await one({ ...base, running: false }, { v: 1, id: "a", type: "open", target: "main" })).toMatchObject({ ok: false, code: "KIN_NOT_RUNNING" });
  });
});

describe("envelope errors", () => {
  test("unsupported version", async () => {
    expect(await one(base, { v: 2, id: "a", type: "status" })).toMatchObject({ ok: false, code: "UNSUPPORTED_VERSION", supported: [1], ref: "a" });
  });
  test("no id or type: BAD_REQUEST without a ref", async () => {
    const r = await one(base, { v: 1, type: "status" });
    expect(r).toMatchObject({ ok: false, code: "BAD_REQUEST", type: "error" });
    expect(r.ref).toBeUndefined();
    expect((await one(base, { v: 1, id: "a" })).code).toBe("BAD_REQUEST");
  });
  test("not JSON", async () => {
    expect((await one(base, "{nope")).code).toBe("BAD_REQUEST");
  });
  test("unknown type", async () => {
    expect(await one(base, { v: 1, id: "a", type: "format-disk" })).toMatchObject({ ok: false, code: "UNSUPPORTED_TYPE", ref: "a" });
  });
  test("several requests on one stdin are answered in order (connectNative style)", async () => {
    const rs = await exchange(base, [
      { v: 1, id: "1", type: "status" },
      { v: 1, id: "2", type: "status" },
    ]);
    expect(rs.map((r) => r.ref)).toEqual(["1", "2"]);
  });
});

describe("send", () => {
  const send = (extra: Record<string, unknown>, scenario: Record<string, unknown> = base) => one(scenario, { v: 1, id: "s", type: "send", ...extra });

  test("text, link and page to an online phone", async () => {
    expect(await send({ kind: "text", text: "hi" })).toMatchObject({ ok: true, queued: false, deviceId: "px", deviceName: "Pixel 8" });
    expect(await send({ kind: "link", url: "https://example.com/a", title: "T", timestampSec: 3 })).toMatchObject({ ok: true, queued: false });
    expect(await send({ kind: "page", url: "http://example.com/" })).toMatchObject({ ok: true });
  });
  test("an offline phone queues, with the DEVICE_OFFLINE_QUEUED notice", async () => {
    expect(await send({ kind: "text", text: "hi", deviceId: "gx" })).toMatchObject({ ok: true, queued: true, code: "DEVICE_OFFLINE_QUEUED", deviceId: "gx" });
  });
  test("NOT_PAIRED: nobody paired, or an unknown deviceId", async () => {
    expect(await send({ kind: "text", text: "hi" }, { ...base, devices: [] })).toMatchObject({ ok: false, code: "NOT_PAIRED" });
    expect(await send({ kind: "text", text: "hi", deviceId: "nope" })).toMatchObject({ ok: false, code: "NOT_PAIRED" });
  });
  test("BAD_URL for every scheme that isn't http or https, and for whitespace", async () => {
    for (const url of ["javascript:alert(1)", "file:///C:/x", "data:text/plain,x", "chrome://x", "https://a b/", "https://example.com/\n", "x".repeat(10)]) {
      expect(await send({ kind: "link", url })).toMatchObject({ ok: false, code: "BAD_URL" });
    }
  });
  test("BAD_REQUEST for an unknown kind, a title over 300 characters, an empty text", async () => {
    expect((await send({ kind: "video" })).code).toBe("BAD_REQUEST");
    expect((await send({ kind: "link", url: "https://example.com/", title: "x".repeat(301) })).code).toBe("BAD_REQUEST");
    expect((await send({ kind: "text", text: "" })).code).toBe("BAD_REQUEST");
  });
  test("TOO_LARGE for a text over 4 MiB", async () => {
    expect((await send({ kind: "text", text: "a".repeat(4 * 1024 * 1024 + 1) })).code).toBe("TOO_LARGE");
  });
  test("send starts Kin when it isn't running (the host does that), or fails with KIN_NOT_RUNNING", async () => {
    expect(await send({ kind: "text", text: "hi" }, { ...base, running: false })).toMatchObject({ ok: true });
    expect(await send({ kind: "text", text: "hi" }, { ...base, running: false, startable: false })).toMatchObject({ ok: false, code: "KIN_NOT_RUNNING" });
  });
  test("an automatic alert never starts Kin (like the real host): KIN_NOT_RUNNING, and a manual one does", async () => {
    const alert = { kind: "alert", title: "Download finished", text: "a.zip", source: "downloads" };
    expect(await send({ ...alert, auto: true }, { ...base, running: false })).toMatchObject({ ok: false, code: "KIN_NOT_RUNNING" });
    expect(await send(alert, { ...base, running: false })).toMatchObject({ ok: true });
  });
  test("image: exactly one of url and data", async () => {
    expect((await send({ kind: "image" })).code).toBe("BAD_REQUEST");
    expect((await send({ kind: "image", url: "https://example.com/a.png", data: "AAAA", mime: "image/png", name: "a.png" })).code).toBe("BAD_REQUEST");
  });
  test("image bytes: a PNG is accepted, anything else is NOT_AN_IMAGE", async () => {
    const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==", "base64");
    expect(await send({ kind: "image", data: png.toString("base64"), mime: "image/png", name: "a.png" })).toMatchObject({ ok: true });
    expect(await send({ kind: "image", data: Buffer.from("<html>sign in</html>").toString("base64"), mime: "image/png", name: "a.png" })).toMatchObject({ ok: false, code: "NOT_AN_IMAGE" });
  });
  test("alert: the fields of docs/extension.md, to an online phone", async () => {
    const a = { kind: "alert", title: "Download finished", text: "report.pdf", source: "downloads", auto: true, ttlSec: 1800 };
    expect(await send(a)).toMatchObject({ ok: true, queued: false, deviceId: "px" });
    expect((await send(a)).code).toBeUndefined();
    expect(await send({ ...a, deviceId: "gx" })).toMatchObject({ ok: true, queued: true, code: "DEVICE_OFFLINE_QUEUED" });
    expect(await send({ kind: "alert", text: "done" })).toMatchObject({ ok: true }); // title, ttlSec, source, auto are optional
  });
  test("alert: BAD_REQUEST for a bad text, title, ttlSec, source or auto", async () => {
    const a = { kind: "alert", text: "ok" };
    for (const bad of [{ text: "" }, { text: "  " }, { text: "x".repeat(301) }, { title: "t".repeat(61) }, { title: 5 }, { ttlSec: 59 }, { ttlSec: 604_801 }, { ttlSec: 1.5 }, { source: "web" }, { auto: "yes" }]) {
      expect((await send({ ...a, ...bad })).code).toBe("BAD_REQUEST");
    }
    expect((await send({ ...a, text: "x".repeat(300), title: "t".repeat(60), ttlSec: 60 })).ok).toBe(true);
  });
  test("alert: a paused PC skips an auto alert with the PAUSED_SKIPPED notice, and still sends a script's own", async () => {
    const paused = { ...base, paused: true };
    expect(await send({ kind: "alert", text: "x", auto: true }, paused)).toMatchObject({ ok: true, queued: false, code: "PAUSED_SKIPPED" });
    expect(await send({ kind: "alert", text: "x", auto: false }, paused)).toMatchObject({ ok: true, queued: false });
    expect((await send({ kind: "alert", text: "x", auto: false }, paused)).code).toBeUndefined();
    expect(await one(paused, { v: 1, id: "a", type: "status" })).toMatchObject({ ok: true, paused: true });
  });
  test("a forced error is passed through", async () => {
    expect(await send({ kind: "text", text: "hi" }, { ...base, sendError: { code: "INTERNAL", message: "boom" } })).toMatchObject({ ok: false, code: "INTERNAL", message: "boom" });
  });
});
