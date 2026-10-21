import { LIMITS } from "./protocol";

const encoder = new TextEncoder();
export const utf8Length = (s: string): number => encoder.encode(s).length;

/** Whitespace or a control character anywhere (the "openable URL" rule of docs/protocol.md). */
const BAD_URL_CHARS = /[\s\u0000-\u001f\u007f-\u009f]/u;
/** The Unicode White_Space set of docs/protocol.md (JavaScript's own trim() leaves U+0085 and adds U+FEFF). */
const EDGE_WHITESPACE = /^[\x09-\x0d\x20\x85\xa0\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}]+|[\x09-\x0d\x20\x85\xa0\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}]+$/gu;
const CONTROL_RUNS = /[\u0000-\u001f\u007f-\u009f\s]+/gu;

export type UrlCheck = { ok: true; url: string } | { ok: false; reason: string; message: string };

const NOT_A_WEB_LINK = "Kin can only send web links (http or https).";

function rejected(reason: string, message = NOT_A_WEB_LINK): UrlCheck {
  return { ok: false, reason, message };
}

/**
 * The authority rules of docs/protocol.md ("Links: isUrl and openable URLs", steps 4 to 6) that a
 * browser's URL parser would let through but Kin refuses. `text` is the trimmed URL, already known
 * to start with http:// or https://. Returns the rejection, or undefined when the authority is fine.
 */
function checkAuthority(text: string): UrlCheck | undefined {
  const rest = text.slice(text.indexOf("://") + 3);
  const authority = rest.slice(0, rest.search(/[/?#]|$/));
  if (authority.includes("@")) {
    return rejected("user information", "Kin won't send a link with a username or password in it (like user@host). Remove that part and try again.");
  }
  if (authority.includes("\\") || authority.includes("%")) {
    return rejected("backslash or percent in the authority", "Kin won't send this link: its address has a \\ or a % before the first /, which browsers read in different ways.");
  }
  let host: string;
  let port: string | undefined;
  if (authority.startsWith("[")) {
    const m = /^\[([^\[\]]+)\](?::(.*))?$/su.exec(authority);
    if (!m) return rejected("bad IPv6 literal", "Kin won't send this link: the address in [brackets] isn't valid.");
    host = "[" + m[1] + "]";
    port = m[2];
  } else {
    const colon = authority.indexOf(":");
    host = colon < 0 ? authority : authority.slice(0, colon);
    port = colon < 0 ? undefined : authority.slice(colon + 1);
    if (/[\[\]<>^|]/.test(host)) {
      return rejected("bad host character", "Kin won't send this link: its address has a character that isn't allowed in a web address.");
    }
  }
  if (host === "" || /^\.*$/.test(host)) return rejected("no host", "That link has no website address in it.");
  if (port !== undefined && !(/^[0-9]+$/.test(port) && Number(port) >= 1 && Number(port) <= 65535)) {
    return rejected("bad port", "Kin won't send this link: its port number isn't valid (1 to 65535).");
  }
  return undefined;
}

/**
 * Only http and https, no whitespace or control characters, none of the authority forms Kin refuses
 * (user information, `\`, `%`, bad port), at most 8 KiB. On a rejection `message` is for the user.
 * The returned URL is `new URL(x).href`, so it is always in its escaped, ASCII-host form.
 */
export function checkUrl(input: unknown): UrlCheck {
  if (typeof input !== "string") return rejected("not a string");
  const trimmed = input.replace(EDGE_WHITESPACE, "");
  if (trimmed === "") return rejected("empty");
  if (BAD_URL_CHARS.test(trimmed)) return rejected("whitespace or control character");
  if (!/^https?:\/\//i.test(trimmed)) return rejected("not http or https");
  const authorityProblem = checkAuthority(trimmed);
  if (authorityProblem) return authorityProblem;
  let parsed: URL;
  try {
    parsed = new URL(trimmed);
  } catch {
    return rejected("not a URL");
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return rejected("not http or https");
  if (/^\.*$/.test(parsed.hostname)) return rejected("no host", "That link has no website address in it.");
  const href = parsed.href;
  if (utf8Length(href) > LIMITS.urlBytes) return rejected("too long", "That link is too long to send (the limit is 8 KiB).");
  return { ok: true, url: href };
}

function wellFormed(s: string): string {
  const w = (s as string & { toWellFormed?: () => string }).toWellFormed;
  return typeof w === "function"
    ? w.call(s)
    : s.replace(/[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/g, "�");
}

/** Collapses whitespace and control characters, cuts at 300 code points. Empty gives undefined. */
export function cleanTitle(title: unknown): string | undefined {
  if (typeof title !== "string") return undefined;
  const collapsed = wellFormed(title).replace(CONTROL_RUNS, " ").trim();
  if (collapsed === "") return undefined;
  const points = Array.from(collapsed);
  return points.length > LIMITS.titleChars ? points.slice(0, LIMITS.titleChars).join("").trimEnd() : collapsed;
}

export type TextCheck = { ok: true; text: string } | { ok: false; code: "BAD_REQUEST" | "TOO_LARGE" };

export function checkText(input: unknown): TextCheck {
  if (typeof input !== "string" || input === "") return { ok: false, code: "BAD_REQUEST" };
  const text = wellFormed(input);
  if (utf8Length(text) > LIMITS.textBytes) return { ok: false, code: "TOO_LARGE" };
  return { ok: true, text };
}

const MIME_EXT: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/bmp": "bmp",
  "image/avif": "avif",
  "image/svg+xml": "svg",
  "image/heic": "heic",
  "image/heif": "heif",
};

/** A short name for an image, taken from its URL (the desktop cleans it again). */
export function imageNameFromUrl(url: string, mime?: string): string {
  let base = "image";
  try {
    const last = new URL(url).pathname.split("/").filter(Boolean).pop();
    if (last) base = decodeURIComponent(last);
  } catch {
    // keep "image"
  }
  base = base.replace(/[\\/:*?"<>|\u0000-\u001f]/g, "_").slice(0, 120);
  if (!/\.[A-Za-z0-9]{2,5}$/.test(base) && mime) {
    const ext = MIME_EXT[mime.toLowerCase().split(";")[0]!.trim()];
    if (ext) base += "." + ext;
  }
  return base || "image";
}

/** Size in bytes of a base64 string once decoded. */
export function base64DecodedSize(b64: string): number {
  const pad = b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0;
  return Math.floor((b64.length * 3) / 4) - pad;
}
