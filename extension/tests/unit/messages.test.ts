import { describe, expect, test } from "bun:test";
import {
  buildOpen,
  buildSendImageData,
  buildSendImageUrl,
  buildSendLink,
  buildSendText,
  buildSimple,
  classifyNativeError,
  describeError,
  parseResponse,
} from "../../src/shared/messages";

const req = (type: string) => ({ id: "r1", type });

describe("building requests", () => {
  test("status and start", () => {
    expect(buildSimple("status", "x")).toEqual({ v: 1, id: "x", type: "status" });
    expect(buildSimple("start", "y")).toEqual({ v: 1, id: "y", type: "start" });
    expect(buildOpen("pairing", "z")).toEqual({ v: 1, id: "z", type: "open", target: "pairing" });
  });
  test("ids are unique", () => {
    expect(buildSimple("status").id).not.toBe(buildSimple("status").id);
  });
  test("text", () => {
    expect(buildSendText("hello", undefined, "i")).toEqual({ ok: true, request: { v: 1, id: "i", type: "send", kind: "text", text: "hello" } });
    expect(buildSendText("hello", "dev1", "i")).toEqual({ ok: true, request: { v: 1, id: "i", type: "send", deviceId: "dev1", kind: "text", text: "hello" } });
    expect(buildSendText("")).toMatchObject({ ok: false, code: "BAD_REQUEST" });
    expect(buildSendText("a".repeat(4 * 1024 * 1024 + 1))).toMatchObject({ ok: false, code: "TOO_LARGE" });
  });
  test("link carries the normalised url, a clean title and the timestamp", () => {
    const r = buildSendLink("link", "https://www.youtube.com/watch?v=abcdefghijk&t=61s", "  A\n title ", { timestampSec: 61, id: "i" });
    expect(r).toEqual({
      ok: true,
      request: { v: 1, id: "i", type: "send", kind: "link", url: "https://www.youtube.com/watch?v=abcdefghijk&t=61s", title: "A title", timestampSec: 61 },
    });
  });
  test("link with no title leaves the field out", () => {
    const r = buildSendLink("page", "https://example.com/", "   ", { id: "i" });
    expect(r).toEqual({ ok: true, request: { v: 1, id: "i", type: "send", kind: "page", url: "https://example.com/" } });
  });
  test("a long title is cut to 300 characters", () => {
    const r = buildSendLink("page", "https://example.com/", "x".repeat(500));
    expect(r.ok && "title" in r.request && r.request.title!.length).toBe(300);
  });
  test("a bad url is BAD_URL with a message for the user", () => {
    const r = buildSendLink("link", "javascript:alert(1)", "x");
    expect(r).toMatchObject({ ok: false, code: "BAD_URL" });
    expect(r.ok === false && r.message).toContain("web links");
  });
  test("image by url: url, page url and a name, no data", () => {
    const r = buildSendImageUrl("https://cdn.example.com/p/cat.jpg", "https://example.com/post", undefined, "i");
    expect(r).toEqual({
      ok: true,
      request: { v: 1, id: "i", type: "send", kind: "image", url: "https://cdn.example.com/p/cat.jpg", pageUrl: "https://example.com/post", name: "cat.jpg" },
    });
  });
  test("image by url: a bad page url is dropped, a bad image url fails", () => {
    const r = buildSendImageUrl("https://cdn.example.com/cat.jpg", "chrome://x", undefined, "i");
    expect(r.ok && "pageUrl" in r.request).toBe(false);
    expect(buildSendImageUrl("data:image/png;base64,AAAA", undefined)).toMatchObject({ ok: false, code: "BAD_URL" });
  });
  test("image bytes", () => {
    const r = buildSendImageData("QUJD", "image/png", "a.png", { deviceId: "d", retryOf: "r0", id: "i" });
    expect(r).toEqual({ ok: true, request: { v: 1, id: "i", type: "send", deviceId: "d", kind: "image", data: "QUJD", mime: "image/png", name: "a.png", retryOf: "r0" } });
  });
  test("image bytes: the 32 MiB limit counts decoded bytes", () => {
    const max = 32 * 1024 * 1024;
    const ok = "A".repeat((max / 3) * 4); // exactly 32 MiB decoded
    expect(buildSendImageData(ok, "image/png", "a.png").ok).toBe(true);
    const tooBig = "A".repeat((max / 3) * 4 + 4);
    expect(buildSendImageData(tooBig, "image/png", "a.png")).toMatchObject({ ok: false, code: "TOO_LARGE" });
    expect(buildSendImageData("", "image/png", "a.png")).toMatchObject({ ok: false, code: "BAD_REQUEST" });
  });
});

describe("classifyNativeError", () => {
  test("the messages the browsers use", () => {
    expect(classifyNativeError("Specified native messaging host not found.")).toBe("HOST_NOT_INSTALLED");
    expect(classifyNativeError("Native messaging host app.kinrelay.desktop is not registered.")).toBe("HOST_NOT_INSTALLED");
    expect(classifyNativeError("No such native application app.kinrelay.desktop")).toBe("HOST_NOT_INSTALLED");
    expect(classifyNativeError("Access to the specified native messaging host is forbidden.")).toBe("HOST_FORBIDDEN");
    expect(classifyNativeError("Native host has exited.")).toBe("HOST_EXITED");
    expect(classifyNativeError("Error when communicating with the native messaging host.")).toBe("HOST_EXITED");
    expect(classifyNativeError(undefined)).toBe("HOST_EXITED");
  });
});

describe("parseResponse", () => {
  test("status running", () => {
    const r = parseResponse(
      {
        v: 1,
        ref: "r1",
        type: "status",
        ok: true,
        running: true,
        kinVersion: "0.2.0",
        devices: [
          { id: "a", name: "Pixel 8", online: true, queuedCount: 0 },
          { id: "b", name: "Galaxy", online: false, queuedCount: 2 },
          { id: "", name: "bad", online: true, queuedCount: 0 },
          "junk",
        ],
        defaultDeviceId: "a",
      },
      req("status"),
    );
    expect(r.ok).toBe(true);
    expect(r.status).toEqual({
      running: true,
      kinVersion: "0.2.0",
      defaultDeviceId: "a",
      devices: [
        { id: "a", name: "Pixel 8", online: true, queuedCount: 0 },
        { id: "b", name: "Galaxy", online: false, queuedCount: 2 },
      ],
    });
  });
  test("status with Kin not running is not an error", () => {
    const r = parseResponse({ v: 1, ref: "r1", type: "status", ok: true, running: false, hostVersion: "0.2.0", devices: [] }, req("status"));
    expect(r.ok).toBe(true);
    expect(r.status?.running).toBe(false);
  });
  test("send ok and queued", () => {
    expect(parseResponse({ v: 1, ref: "r1", type: "send", ok: true, queued: false, deviceName: "Pixel 8" }, req("send"))).toMatchObject({ ok: true, queued: false, deviceName: "Pixel 8" });
    const q = parseResponse({ v: 1, ref: "r1", type: "send", ok: true, queued: true, code: "DEVICE_OFFLINE_QUEUED", message: "Pixel is offline" }, req("send"));
    expect(q).toMatchObject({ ok: true, queued: true, code: "DEVICE_OFFLINE_QUEUED", message: "Pixel is offline" });
  });
  test("send failure keeps Kin's message, or falls back to ours", () => {
    expect(parseResponse({ v: 1, ref: "r1", type: "send", ok: false, code: "NOT_PAIRED", message: "No phone." }, req("send"))).toMatchObject({ ok: false, code: "NOT_PAIRED", message: "No phone." });
    expect(parseResponse({ v: 1, ref: "r1", type: "send", ok: false, code: "TOO_LARGE" }, req("send")).message).toBe(describeError("TOO_LARGE"));
    expect(parseResponse({ v: 1, ref: "r1", type: "send", ok: false, code: "SOMETHING_NEW", message: "x" }, req("send"))).toMatchObject({ code: "SOMETHING_NEW", message: "x" });
  });
  test("a failure without a code is INTERNAL", () => {
    expect(parseResponse({ v: 1, ref: "r1", type: "send", ok: false }, req("send"))).toMatchObject({ ok: false, code: "INTERNAL" });
  });
  test("a response to another request is refused", () => {
    expect(parseResponse({ v: 1, ref: "other", type: "send", ok: true }, req("send"))).toMatchObject({ ok: false, code: "HOST_EXITED" });
  });
  test("a host that couldn't read the request answers without a ref", () => {
    expect(parseResponse({ v: 1, type: "error", ok: false, code: "BAD_REQUEST", message: "Bad JSON" }, req("send"))).toMatchObject({ ok: false, code: "BAD_REQUEST", message: "Bad JSON" });
  });
  test("wrong version", () => {
    expect(parseResponse({ v: 2, ref: "r1", type: "send", ok: true }, req("send"))).toMatchObject({ ok: false, code: "UNSUPPORTED_VERSION" });
    expect(parseResponse({ v: 1, ref: "r1", type: "error", ok: false, code: "UNSUPPORTED_VERSION", supported: [1] }, req("send"))).toMatchObject({ code: "UNSUPPORTED_VERSION" });
  });
  test("garbage", () => {
    expect(parseResponse(null, req("send"))).toMatchObject({ ok: false });
    expect(parseResponse("x", req("send"))).toMatchObject({ ok: false });
    expect(parseResponse(undefined, req("send"))).toMatchObject({ ok: false });
  });
  test("caps the device list and names", () => {
    const devices = Array.from({ length: 30 }, (_, i) => ({ id: `d${i}`, name: "n".repeat(100), online: true, queuedCount: 0 }));
    const r = parseResponse({ v: 1, ref: "r1", type: "status", ok: true, running: true, devices }, req("status"));
    expect(r.status?.devices.length).toBe(16);
    expect(r.status?.devices[0]?.name.length).toBe(64);
  });
});
