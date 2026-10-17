import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { checkText, checkUrl, cleanTitle, imageNameFromUrl } from "../../src/shared/validate";

describe("checkUrl", () => {
  test("accepts http and https", () => {
    expect(checkUrl("https://example.com/a?b=1#c")).toEqual({ ok: true, url: "https://example.com/a?b=1#c" });
    expect(checkUrl("http://example.com")).toEqual({ ok: true, url: "http://example.com/" });
  });
  test("trims the outside, rejects whitespace inside", () => {
    expect(checkUrl("  https://example.com/x \n")).toEqual({ ok: true, url: "https://example.com/x" });
    expect(checkUrl("https://example.com/a b").ok).toBe(false);
    expect(checkUrl("https://exa\tmple.com").ok).toBe(false);
    expect(checkUrl("https://example.com/\u0085x").ok).toBe(false);
    expect(checkUrl("https://example.com/\u00a0x").ok).toBe(false);
  });
  test("rejects every other scheme", () => {
    for (const bad of [
      "javascript:alert(1)",
      "file:///C:/x",
      "data:text/html,hi",
      "chrome://settings",
      "about:blank",
      "blob:https://example.com/abc",
      "ftp://example.com/x",
      "intent://x#Intent;end",
      "mailto:a@b.c",
    ]) {
      expect(checkUrl(bad).ok).toBe(false);
    }
  });
  test("rejects scheme tricks", () => {
    expect(checkUrl("//example.com").ok).toBe(false);
    expect(checkUrl("example.com").ok).toBe(false);
    expect(checkUrl("https:///").ok).toBe(false);
    expect(checkUrl("https://...").ok).toBe(false);
    expect(checkUrl("").ok).toBe(false);
    expect(checkUrl(undefined).ok).toBe(false);
    expect(checkUrl(42).ok).toBe(false);
  });
  test("a non-ASCII host becomes punycode, so no non-ASCII reaches the wire in the host", () => {
    const r = checkUrl("https://bücher.example/straße");
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.url.startsWith("https://xn--bcher-kva.example/")).toBe(true);
  });
  test("limits the length to 8 KiB of UTF-8", () => {
    expect(checkUrl("https://example.com/" + "a".repeat(8192 - 20)).ok).toBe(true);
    expect(checkUrl("https://example.com/" + "a".repeat(8192)).ok).toBe(false);
  });
  test("refuses user information, with a message that says so", () => {
    for (const bad of ["https://user@example.com/", "https://google.com@evil.example/", "https://google.com:443@evil.example/", "https://@example.com/", "https://me@[::1]:9/"]) {
      const r = checkUrl(bad);
      expect(r.ok).toBe(false);
      if (!r.ok) expect(r.message).toContain("username or password");
    }
  });
  test("an @ after the authority is fine", () => {
    expect(checkUrl("https://example.com/a@b").ok).toBe(true);
    expect(checkUrl("https://example.com/?email=a@b.c#x@y").ok).toBe(true);
  });
  test("refuses a backslash or a percent in the authority, not after it", () => {
    for (const bad of ["https://example.com\\", "https://goog%6Ce.com/", "https://example.com:%38%30/"]) {
      const r = checkUrl(bad);
      expect(r.ok).toBe(false);
      if (!r.ok) expect(r.message).toContain("before the first /");
    }
    expect(checkUrl("https://evil.example\\@google.com").ok).toBe(false);
    expect(checkUrl("https://example.com/a\\b").ok).toBe(true);
    expect(checkUrl("https://example.com/100%25?x=%20").ok).toBe(true);
  });
  test("refuses [ ] < > ^ | in a host that is not an IPv6 literal", () => {
    for (const c of ["[", "]", "<", ">", "^", "|"]) {
      const r = checkUrl(`https://exa${c}mple.com/`);
      expect(r.ok).toBe(false);
      if (!r.ok) expect(r.message).toContain("character that isn't allowed");
    }
    expect(checkUrl("http://[::1]:8080/x").ok).toBe(true);
    expect(checkUrl("http://[::1]]/").ok).toBe(false);
  });
  test("refuses a bad port", () => {
    for (const bad of ["https://example.com:", "https://example.com:0", "https://example.com:65536", "https://example.com:+80", "https://example.com:٨٠", "https://a:b:80", "http://[::1]:/"]) {
      expect(checkUrl(bad).ok).toBe(false);
    }
    expect(checkUrl("https://example.com:080/")).toEqual({ ok: true, url: "https://example.com:80/" });
  });
  test("a scheme must be followed by ://", () => {
    for (const bad of ["https:example.com", "https:/example.com", "https:\\example.com"]) expect(checkUrl(bad).ok).toBe(false);
  });
  test("keeps a YouTube t= untouched", () => {
    expect(checkUrl("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s")).toEqual({ ok: true, url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s" });
  });
});

describe("cleanTitle", () => {
  test("collapses whitespace and control characters", () => {
    expect(cleanTitle("  Hello \n\t world\u0000!  ")).toBe("Hello world !");
  });
  test("empty means absent", () => {
    expect(cleanTitle("   ")).toBeUndefined();
    expect(cleanTitle(undefined)).toBeUndefined();
    expect(cleanTitle(5)).toBeUndefined();
  });
  test("cuts at 300 code points, not UTF-16 units", () => {
    const t = cleanTitle("😀".repeat(400))!;
    expect(Array.from(t).length).toBe(300);
    expect(cleanTitle("a".repeat(300))!.length).toBe(300);
    expect(cleanTitle("a".repeat(301))!.length).toBe(300);
  });
  test("replaces a lone surrogate", () => {
    expect(cleanTitle("a\ud800b")).toBe("a\ufffdb");
  });
});

describe("checkText", () => {
  test("accepts text, rejects empty and over 4 MiB", () => {
    expect(checkText("hi")).toEqual({ ok: true, text: "hi" });
    expect(checkText("")).toEqual({ ok: false, code: "BAD_REQUEST" });
    expect(checkText("a".repeat(4 * 1024 * 1024)).ok).toBe(true);
    expect(checkText("a".repeat(4 * 1024 * 1024 + 1))).toEqual({ ok: false, code: "TOO_LARGE" });
    expect(checkText("é".repeat(2 * 1024 * 1024 + 1))).toEqual({ ok: false, code: "TOO_LARGE" });
  });
});

describe("imageNameFromUrl", () => {
  test("uses the last path segment", () => {
    expect(imageNameFromUrl("https://example.com/a/b/cat.jpg?x=1")).toBe("cat.jpg");
  });
  test("adds an extension from the type when the name has none", () => {
    expect(imageNameFromUrl("https://example.com/render/12345", "image/png")).toBe("12345.png");
  });
  test("falls back to image", () => {
    expect(imageNameFromUrl("https://example.com/")).toBe("image");
    expect(imageNameFromUrl("not a url", "image/webp")).toBe("image.webp");
  });
  test("strips path separators and reserved characters", () => {
    expect(imageNameFromUrl("https://example.com/a%2F..%2Fb%3Ac.png")).toBe("a_.._b_c.png");
  });
});

type UrlsVectors = { cases: { name: string; input?: string; isUrl: boolean }[] };
type LinksVectors = { urls: { name: string; input?: string; valid: boolean }[] };
const urlVectors = JSON.parse(readFileSync(new URL("../../../protocol/urls.json", import.meta.url), "utf8")) as UrlsVectors;
const linkVectors = JSON.parse(readFileSync(new URL("../../../protocol/links.json", import.meta.url), "utf8")) as LinksVectors;

/** A URL Kin accepts that a browser's own parser throws on (a host it refuses): the extension can't send those. */
const BROWSER_REFUSES = new Set(["a-host-a-browser-refuses", "ipv6-literal-is-not-checked-further"]);

describe("checkUrl against the shared vectors", () => {
  test("every URL Kin refuses is refused (urls.json)", () => {
    for (const c of urlVectors.cases) {
      if (c.input !== undefined && !c.isUrl) expect([c.name, checkUrl(c.input).ok]).toEqual([c.name, false]);
    }
  });
  test("every URL Kin accepts is accepted unless a browser can't parse its host (urls.json)", () => {
    for (const c of urlVectors.cases) {
      if (c.input === undefined || !c.isUrl || BROWSER_REFUSES.has(c.name)) continue;
      expect([c.name, checkUrl(c.input).ok]).toEqual([c.name, true]);
    }
  });
  test("every link URL Kin refuses is refused, ignoring whitespace at the ends (links.json)", () => {
    for (const c of linkVectors.urls) {
      if (c.input === undefined || c.valid || c.input !== c.input.trim()) continue;
      expect([c.name, checkUrl(c.input).ok]).toEqual([c.name, false]);
    }
  });
  test("every link URL Kin accepts is accepted, unless a browser can't parse its host (links.json)", () => {
    for (const c of linkVectors.urls) {
      if (c.input === undefined || !c.valid || BROWSER_REFUSES.has(c.name)) continue;
      expect([c.name, checkUrl(c.input).ok]).toEqual([c.name, true]);
    }
  });
});
