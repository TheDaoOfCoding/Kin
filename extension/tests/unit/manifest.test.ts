import { describe, expect, test } from "bun:test";
import { addHistory, type HistoryEntry, labelForImage, labelForText, labelForUrl, sanitizeHistory } from "../../src/shared/history";
import { buildManifest, CHROMIUM_ID, PERMISSIONS } from "../../scripts/manifest";
import { createHash } from "node:crypto";
import key from "../../manifest-key.json";

describe("manifest", () => {
  test("permissions are exactly the five allowed ones, in every target and flavor", () => {
    for (const target of ["chromium", "firefox"] as const) {
      for (const flavor of ["release", "dev", "test"] as const) {
        const m = buildManifest(target, flavor);
        expect([...(m.permissions as string[])].sort()).toEqual(["activeTab", "contextMenus", "nativeMessaging", "scripting", "storage"]);
        expect(m.optional_permissions).toBeUndefined();
        expect(m.content_scripts).toBeUndefined();
        expect(m.externally_connectable).toBeUndefined();
      }
    }
    expect(PERMISSIONS).toHaveLength(5);
  });
  test("release and dev have no host permissions at all", () => {
    for (const target of ["chromium", "firefox"] as const) {
      for (const flavor of ["release", "dev"] as const) {
        const m = buildManifest(target, flavor);
        expect(m.host_permissions).toBeUndefined();
        expect(JSON.stringify(m)).not.toContain("all_urls");
      }
    }
  });
  test("the test build may reach only the local test servers", () => {
    expect(buildManifest("chromium", "test").host_permissions).toEqual(["http://127.0.0.1/*", "http://localhost/*", "https://www.youtube.com/*", "https://youtu.be/*"]);
  });
  test("shortcut is Alt+Shift+K", () => {
    const m = buildManifest("chromium", "release") as { commands: Record<string, { suggested_key: { default: string } }> };
    expect(m.commands["send-tab"]!.suggested_key.default).toBe("Alt+Shift+K");
  });
  test("chromium uses a service worker and the stable key; firefox a background script and the gecko id", () => {
    const c = buildManifest("chromium", "release") as { background: unknown; key?: string };
    expect(c.background).toEqual({ service_worker: "background.js" });
    expect(c.key).toBe(key.key);
    const f = buildManifest("firefox", "release") as { background: unknown; browser_specific_settings: { gecko: { id: string } }; key?: string };
    expect(f.background).toEqual({ scripts: ["background.js"] });
    expect(f.browser_specific_settings.gecko.id).toBe("kin@kinrelay.app");
    expect(f.key).toBeUndefined();
  });
  test("the store build can leave the key out", () => {
    expect((buildManifest("chromium", "release", { withKey: false }) as { key?: string }).key).toBeUndefined();
  });
  test("the extension ID is the one derived from the key (and the one docs/extension.md names)", () => {
    const der = Buffer.from(key.key, "base64");
    const id = [...createHash("sha256").update(der).digest().subarray(0, 16)].map((b) => String.fromCharCode(97 + (b >> 4)) + String.fromCharCode(97 + (b & 15))).join("");
    expect(id).toBe(CHROMIUM_ID);
    expect(CHROMIUM_ID).toBe("cbioefpddjpaionhljhffgbedncfchmn");
  });
});

describe("history", () => {
  const e = (n: number) => ({ at: n, kind: "link" as const, label: `l${n}`, outcome: "sent" as const });
  test("keeps the newest 10", () => {
    let list: HistoryEntry[] = [];
    for (let i = 0; i < 15; i++) list = addHistory(list, e(i));
    expect(list).toHaveLength(10);
    expect(list[0]!.at).toBe(14);
    expect(list[9]!.at).toBe(5);
  });
  test("labels never contain the content of a text", () => {
    expect(labelForText("my secret password")).toBe("Text, 18 characters");
    expect(labelForText("a")).toBe("Text, 1 character");
  });
  test("a link is labelled by title or host, never the URL", () => {
    expect(labelForUrl("A page", "https://example.com/private?token=1")).toBe("A page");
    expect(labelForUrl(undefined, "https://example.com/private?token=1")).toBe("example.com");
    expect(labelForUrl("  ", "nonsense")).toBe("Link");
  });
  test("an image is labelled by file name", () => {
    expect(labelForImage("https://x.example/p/cat.jpg?sig=secret")).toBe("Image cat.jpg");
    expect(labelForImage("data:image/png;base64,AAAA")).toBe("Image");
    expect(labelForImage("blob:https://x.example/1234")).toBe("Image");
  });
  test("sanitizeHistory drops junk", () => {
    expect(sanitizeHistory("nope")).toEqual([]);
    expect(sanitizeHistory([{ at: 1, kind: "link", label: "a", outcome: "sent" }, { at: "x" }, null, { at: 2, kind: "bad", label: "a", outcome: "sent" }])).toEqual([
      { at: 1, kind: "link", label: "a", outcome: "sent" },
    ]);
  });
});
