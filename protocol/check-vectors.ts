// Reference implementation of the parts of docs/protocol.md that the test vectors cover.
// It exists to prove the vectors and the spec agree. It is NOT shipped in either app.
//
//   bun protocol/check-vectors.ts
//
// Exit code 0 = every vector passes. Exit code 1 = something disagrees (details are printed).
//
// Everything here is written straight from the spec text, not from the vectors: the Rust and
// Kotlin implementations should do the same, then run the same vectors against it.

import { createHash, createPrivateKey, createPublicKey, X509Certificate } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const load = (file: string): any => JSON.parse(readFileSync(join(dir, file), "utf8"));

// ------------------------------------------------------------------ test harness
let failures = 0;
const counts: Record<string, number> = {};
function check(group: string, name: string, ok: boolean, detail?: () => string) {
  counts[group] = (counts[group] ?? 0) + 1;
  if (!ok) {
    failures++;
    console.log(`FAIL [${group}] ${name}${detail ? "\n       " + detail() : ""}`);
  }
}
const j = (x: unknown) => JSON.stringify(x);
// deep equality that ignores object key order (JSON objects are unordered)
const canon = (x: any): any => (Array.isArray(x) ? x.map(canon) : x && typeof x === "object" ? Object.fromEntries(Object.keys(x).sort().map((k) => [k, canon(x[k])])) : x);
const eq = (a: unknown, b: unknown) => j(canon(a)) === j(canon(b));
function uniqueNames(group: string, items: any[]) {
  const seen = new Set<string>();
  for (const it of items) {
    check(group, `unique name: ${it.name}`, !seen.has(it.name), () => "duplicate vector name");
    seen.add(it.name);
  }
}
const hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
const unhex = (h: string) => new Uint8Array(Buffer.from(h, "hex"));
const cps = (s: string) => Array.from(s); // Unicode code points

// ------------------------------------------------------------------ encodings
const B64URL = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
function base64url(bytes: Uint8Array): string {
  let out = "";
  let bits = 0;
  let val = 0;
  for (const b of bytes) {
    val = (val << 8) | b;
    bits += 8;
    while (bits >= 6) {
      out += B64URL[(val >>> (bits - 6)) & 63];
      bits -= 6;
    }
    val &= (1 << bits) - 1;
  }
  if (bits > 0) out += B64URL[(val << (6 - bits)) & 63]; // no padding
  return out;
}
const B32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
function base32(bytes: Uint8Array): string {
  let out = "";
  let bits = 0;
  let val = 0;
  for (const b of bytes) {
    val = (val << 8) | b;
    bits += 8;
    while (bits >= 5) {
      out += B32[(val >>> (bits - 5)) & 31];
      bits -= 5;
    }
    val &= (1 << bits) - 1;
  }
  if (bits > 0) out += B32[(val << (5 - bits)) & 31]; // no padding
  return out;
}
const sha256 = (b: Uint8Array) => new Uint8Array(createHash("sha256").update(b).digest());
const deviceIdOfHash = (hash: Uint8Array) => base64url(hash);
const shortIdOfHash = (hash: Uint8Array) => base32(hash).slice(0, 8);
const deviceIdOfSpki = (spki: Uint8Array) => deviceIdOfHash(sha256(spki));
const validDeviceId = (s: string) => /^[A-Za-z0-9_-]{43}$/.test(s);
const validToken = (s: string) => /^[A-Za-z0-9_-]{22}$/.test(s);

// ------------------------------------------------------------------ certificate profile (protocol.md, Transport)
// A tiny DER walker, just enough to check the profile. It reads the certificate bytes, not a library's view of them,
// because the point of the profile is what rustls-webpki sees: version, key, signature algorithm, serial, times, extensions.
type Tlv = { tag: number; start: number; hdr: number; end: number };
function readTlv(b: Uint8Array, pos: number, limit: number): Tlv {
  if (pos + 2 > limit) throw new Error("truncated TLV");
  const tag = b[pos];
  let len = b[pos + 1];
  let hdr = 2;
  if (len & 0x80) {
    const n = len & 0x7f;
    if (n === 0 || n > 3) throw new Error("unsupported length form");
    len = 0;
    for (let i = 0; i < n; i++) len = len * 256 + b[pos + 2 + i];
    hdr = 2 + n;
    if (len < 128) throw new Error("non-minimal DER length");
  }
  if (pos + hdr + len > limit) throw new Error("TLV runs past its parent");
  return { tag, start: pos, hdr, end: pos + hdr + len };
}
const kids = (b: Uint8Array, t: Tlv): Tlv[] => {
  const out: Tlv[] = [];
  for (let p = t.start + t.hdr; p < t.end; ) {
    const c = readTlv(b, p, t.end);
    out.push(c);
    p = c.end;
  }
  return out;
};
const body = (b: Uint8Array, t: Tlv) => b.subarray(t.start + t.hdr, t.end);
const whole = (b: Uint8Array, t: Tlv) => b.subarray(t.start, t.end);
function oidOf(b: Uint8Array, t: Tlv): string {
  if (t.tag !== 0x06) throw new Error("not an OID");
  const c = body(b, t);
  const arcs: number[] = [Math.floor(c[0] / 40), c[0] % 40];
  for (let i = 1, v = 0; i < c.length; i++) {
    v = v * 128 + (c[i] & 0x7f);
    if (!(c[i] & 0x80)) {
      arcs.push(v);
      v = 0;
    }
  }
  return arcs.join(".");
}
const OID_EC_PUBLIC_KEY = "1.2.840.10045.2.1";
const OID_PRIME256V1 = "1.2.840.10045.3.1.7";
const OID_ECDSA_SHA256 = "1.2.840.10045.4.3.2";
const MAX_SERIAL_BYTES = 20;

// Returns the list of ways the certificate leaves the profile (empty = it matches).
function certProfileViolations(der: Uint8Array): string[] {
  const v: string[] = [];
  try {
    const cert = readTlv(der, 0, der.length);
    if (cert.tag !== 0x30 || cert.end !== der.length) return ["not one DER SEQUENCE"];
    const [tbs, outerAlg, sigValue] = kids(der, cert);
    if (!tbs || tbs.tag !== 0x30 || !outerAlg || !sigValue || sigValue.tag !== 0x03) return ["not a Certificate structure"];
    const f = kids(der, tbs);
    let i = 0;
    // version: [0] EXPLICIT INTEGER, 2 = v3. Absent means v1.
    if (f[i]?.tag === 0xa0) {
      const [ver] = kids(der, f[i]);
      const n = ver && ver.tag === 0x02 ? body(der, ver) : new Uint8Array();
      if (!(n.length === 1 && n[0] === 2)) v.push(`version is not v3 (version field ${hex(n)})`);
      i++;
    } else v.push("version is v1 (no version field)");
    // serial: positive, at most 20 bytes, minimal DER
    const serial = f[i++];
    if (!serial || serial.tag !== 0x02) return [...v, "no serial number"];
    const s = body(der, serial);
    if (s.length === 0 || s.length > MAX_SERIAL_BYTES) v.push(`serial is ${s.length} bytes (1 to ${MAX_SERIAL_BYTES} allowed)`);
    if (s.length > 0 && (s[0] & 0x80)) v.push("serial is negative");
    else if (s.length > 0 && s.every((x) => x === 0)) v.push("serial is zero");
    if (s.length > 1 && s[0] === 0 && !(s[1] & 0x80)) v.push("serial is not minimal DER");
    // signature algorithm: ecdsa-with-SHA256, no parameters, and the same in the outer copy
    const alg = f[i++];
    if (!alg || alg.tag !== 0x30) return [...v, "no signature algorithm"];
    const algKids = kids(der, alg);
    if (algKids.length < 1 || oidOf(der, algKids[0]) !== OID_ECDSA_SHA256) v.push("signature algorithm is not ecdsa-with-SHA256");
    else if (algKids.length !== 1) v.push("signature algorithm has parameters");
    if (hex(whole(der, alg)) !== hex(whole(der, outerAlg))) v.push("outer and inner signature algorithms differ");
    i++; // issuer: free
    // validity: RFC 5280 times, UTCTime up to 2049 and GeneralizedTime from 2050
    const validity = f[i++];
    if (!validity || validity.tag !== 0x30 || kids(der, validity).length !== 2) v.push("validity is not two times");
    else
      for (const t of kids(der, validity)) {
        const text = new TextDecoder().decode(body(der, t));
        if (t.tag === 0x17) {
          if (!/^[0-9]{12}Z$/.test(text)) v.push(`UTCTime ${text} is not YYMMDDHHMMSSZ`);
        } else if (t.tag === 0x18) {
          if (!/^[0-9]{14}Z$/.test(text)) v.push(`GeneralizedTime ${text} is not YYYYMMDDHHMMSSZ`);
          else if (Number(text.slice(0, 4)) < 2050) v.push(`GeneralizedTime ${text} is before 2050 (RFC 5280 wants UTCTime)`);
        } else v.push("a validity bound is neither UTCTime nor GeneralizedTime");
      }
    i++; // subject: free
    // subject public key: id-ecPublicKey, prime256v1, uncompressed point, 91 bytes in all
    const spki = f[i++];
    if (!spki || spki.tag !== 0x30) return [...v, "no SubjectPublicKeyInfo"];
    if (spki.end - spki.start !== 91) v.push(`SPKI is ${spki.end - spki.start} bytes (91 for P-256)`);
    const [keyAlg, keyBits] = kids(der, spki);
    const ka = keyAlg ? kids(der, keyAlg) : [];
    if (ka.length !== 2 || oidOf(der, ka[0]) !== OID_EC_PUBLIC_KEY) v.push("key type is not id-ecPublicKey");
    else if (ka[1].tag !== 0x06 || oidOf(der, ka[1]) !== OID_PRIME256V1) v.push("curve is not prime256v1");
    const bits = keyBits && keyBits.tag === 0x03 ? body(der, keyBits) : new Uint8Array();
    if (!(bits.length === 66 && bits[0] === 0 && bits[1] === 0x04)) v.push("public key is not an uncompressed P-256 point");
    // optional fields after the SPKI: only [3] extensions matter, and none may be critical
    for (; i < f.length; i++) {
      if (f[i].tag !== 0xa3) continue;
      const [list] = kids(der, f[i]);
      for (const ext of kids(der, list)) {
        const parts = kids(der, ext);
        const id = oidOf(der, parts[0]);
        if (parts[1]?.tag === 0x01 && body(der, parts[1])[0] !== 0) v.push(`critical extension ${id}`);
      }
    }
  } catch (e) {
    v.push(`unparseable: ${(e as Error).message}`);
  }
  return v;
}

// OIDs of the extensions a certificate carries (empty when it has none).
function certExtensionOids(der: Uint8Array): string[] {
  const cert = readTlv(der, 0, der.length);
  const f = kids(der, kids(der, cert)[0]);
  const ext = f.find((x) => x.tag === 0xa3);
  return ext ? kids(der, kids(der, ext)[0]).map((e) => oidOf(der, kids(der, e)[0])) : [];
}

// ------------------------------------------------------------------ pairing URL
type Addr = { host: string; port: number };
type ParseError = { category: string; param?: string };
type Parsed = { version: 1; addresses: Addr[]; id: string; token: string; name: string | null };

class Fail extends Error {
  constructor(public err: ParseError) {
    super(err.category);
  }
}
const fail = (category: string, param?: string): never => {
  throw new Fail(param ? { category, param } : { category });
};

// %XX -> byte, everything else is itself ('+' stays '+'), then strict UTF-8.
function percentDecode(raw: string): string | null {
  const src = new TextEncoder().encode(raw);
  const out: number[] = [];
  for (let i = 0; i < src.length; i++) {
    if (src[i] === 0x25) {
      const h = String.fromCharCode(src[i + 1] ?? 0, src[i + 2] ?? 0);
      if (i + 2 >= src.length || !/^[0-9A-Fa-f]{2}$/.test(h)) return null;
      out.push(parseInt(h, 16));
      i += 2;
    } else out.push(src[i]);
  }
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(new Uint8Array(out));
  } catch {
    return null;
  }
}
function percentEncode(s: string, keep: string): string {
  let out = "";
  for (const b of new TextEncoder().encode(s)) {
    const c = String.fromCharCode(b);
    out += /[A-Za-z0-9\-._~]/.test(c) || (b < 128 && keep.includes(c)) ? c : "%" + b.toString(16).toUpperCase().padStart(2, "0");
  }
  return out;
}
// ASCII digits only ([0-9], never \d or a library number parser), no sign, no leading zeros.
const ASCII_DIGITS = /^[0-9]+$/;
const noLeadingZero = (x: string) => x === "0" || !x.startsWith("0");
function parseAddress(entry: string): Addr | null {
  const m = /^([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3}):([0-9]{1,5})$/.exec(entry);
  if (!m) return null;
  if (!m.slice(1, 6).every(noLeadingZero)) return null;
  const o = m.slice(1, 5).map(Number);
  if (o.some((x) => !(x >= 0 && x <= 255))) return null;
  const port = Number(m[5]);
  if (port < 1 || port > 65535) return null;
  const [a, b] = o;
  const priv = a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168);
  const shared = a === 100 && b >= 64 && b <= 127; // 100.64.0.0/10 (CGNAT, Tailscale): accepted, not advertised by the PC
  if (!priv && !shared) return null;
  return { host: o.join("."), port };
}

function parsePairingUrl(input: string): { ok: true; value: Parsed } | { ok: false; error: ParseError } {
  try {
    const s = input.replace(/^[ \t\r\n]+|[ \t\r\n]+$/g, "");
    const lower = s.toLowerCase();
    let params: string;
    if (lower.startsWith("kinrelay://")) {
      let rest = s.slice("kinrelay://".length);
      const hash = rest.indexOf("#");
      if (hash >= 0) rest = rest.slice(0, hash);
      const q = rest.indexOf("?");
      const head = (q >= 0 ? rest.slice(0, q) : rest).toLowerCase();
      params = q >= 0 ? rest.slice(q + 1) : "";
      if (head !== "pair" && head !== "pair/") return fail("not_pairing_url");
    } else if (lower.startsWith("https://")) {
      const rest = s.slice("https://".length);
      const end = rest.search(/[/?#]/);
      const authority = end < 0 ? rest : rest.slice(0, end);
      if (authority === "") return fail("not_pairing_url");
      const tail = end < 0 ? "" : rest.slice(end);
      const hash = tail.indexOf("#");
      const beforeFragment = hash >= 0 ? tail.slice(0, hash) : tail;
      params = hash >= 0 ? tail.slice(hash + 1) : "";
      const path = beforeFragment.split("?")[0];
      if (path !== "/pair" && path !== "/pair/") return fail("not_pairing_url");
    } else return fail("not_pairing_url");

    const found = new Map<string, string[]>();
    for (const piece of params.split("&")) {
      if (piece === "") continue;
      const eqAt = piece.indexOf("=");
      const name = eqAt < 0 ? piece : piece.slice(0, eqAt);
      const value = eqAt < 0 ? "" : piece.slice(eqAt + 1);
      found.set(name, [...(found.get(name) ?? []), value]);
    }
    const get = (name: string, required: boolean): string | undefined => {
      const vals = found.get(name);
      if (vals && vals.length > 1) return fail("duplicate_param", name);
      if (!vals) return required ? fail("missing_param", name) : undefined;
      const dec = percentDecode(vals[0]);
      if (dec === null) return fail("invalid_param", name);
      return dec;
    };

    const v = get("v", true)!;
    if (!ASCII_DIGITS.test(v)) return fail("invalid_param", "v");
    if (v !== "1") return fail("unsupported_version");

    const a = get("a", true)!;
    const entries = a.split(",");
    if (entries.length < 1 || entries.length > 3) return fail("invalid_param", "a");
    const addresses = entries.map((e) => parseAddress(e) ?? fail("invalid_param", "a"));

    const id = get("id", true)!;
    if (!validDeviceId(id)) return fail("invalid_param", "id");
    const token = get("tk", true)!;
    if (!validToken(token)) return fail("invalid_param", "tk");

    const n = get("n", false);
    const name = n === undefined || n === "" ? null : cps(n).slice(0, 64).join("");
    return { ok: true, value: { version: 1, addresses, id, token, name } };
  } catch (e) {
    if (e instanceof Fail) return { ok: false, error: e.err };
    throw e;
  }
}

// What the PC writes into the QR code.
function buildPairingUrl(p: Parsed): string {
  let url = `kinrelay://pair?v=1&a=${p.addresses.map((x) => `${x.host}:${x.port}`).join(",")}&id=${p.id}&tk=${p.token}`;
  if (p.name !== null) url += `&n=${percentEncode(cps(p.name).slice(0, 64).join(""), "")}`;
  return url;
}

// ------------------------------------------------------------------ file names
const RESERVED = new Set([
  "CON", "PRN", "AUX", "NUL",
  ..."0123456789".split("").flatMap((d) => [`COM${d}`, `LPT${d}`]),
  ...["\u00b9", "\u00b2", "\u00b3"].flatMap((d) => [`COM${d}`, `LPT${d}`]), // superscript 1, 2, 3
]);
const asciiUpper = (s: string) => s.replace(/[a-z]/g, (c) => c.toUpperCase());
const MAX_NAME = 200; // UTF-8 bytes
const MAX_EXT = 16; // UTF-8 bytes, dot included
const utf8Len = (s: string) => Buffer.byteLength(s, "utf8");
// Bidi_Control: U+061C, U+200E, U+200F, U+202A-U+202E, U+2066-U+2069
const isBidiControl = (cp: number) => cp === 0x061c || cp === 0x200e || cp === 0x200f || (cp >= 0x202a && cp <= 0x202e) || (cp >= 0x2066 && cp <= 0x2069);
// the step 5 test: the part before the first dot, trailing spaces removed, equals a reserved name ignoring ASCII case
function isReservedName(s: string): boolean {
  const dot = s.indexOf(".");
  return RESERVED.has(asciiUpper((dot < 0 ? s : s.slice(0, dot)).replace(/ +$/, "")));
}

// The collision loop tries at most this many candidates (the plain name and " (1)" to " (99)"); then the file fails with internal.
const MAX_CANDIDATES = 100;

function cleanFileName(input: string, existing: string[]): string {
  return cleanFileNameWithin(input, existing, Infinity)!;
}

// The first free candidate among the first `limit` ones, or null if every one of them is taken.
function cleanFileNameWithin(input: string, existing: string[], limit: number): string | null {
  // 1. path
  let s = input;
  const cut = Math.max(s.lastIndexOf("/"), s.lastIndexOf("\\"));
  if (cut >= 0) s = s.slice(cut + 1);
  // 2. forbidden characters, control characters and bidi controls
  s = cps(s)
    .map((c) => {
      const cp = c.codePointAt(0)!;
      return "<>:\"/\\|?*".includes(c) || cp <= 0x1f || (cp >= 0x7f && cp <= 0x9f) || isBidiControl(cp) ? "_" : c;
    })
    .join("");
  // 3. trailing dots and spaces
  s = s.replace(/[. ]+$/, "");
  // 4. empty
  if (s === "") s = "file";
  // 5. reserved names
  if (isReservedName(s)) s = "_" + s;
  // 6. split into stem and extension (16 BYTES with the dot), then find the first free candidate within 200 BYTES
  const chars = cps(s);
  const lastDot = chars.lastIndexOf(".");
  const ext = lastDot > 0 && utf8Len(chars.slice(lastDot).join("")) <= MAX_EXT ? chars.slice(lastDot).join("") : "";
  const stem = ext ? chars.slice(0, lastDot) : chars;
  const taken = new Set(existing.map((n) => n.toLowerCase()));
  for (let n = 0; n < limit; n++) {
    const suffix = n === 0 ? "" : ` (${n})`;
    const budget = MAX_NAME - utf8Len(suffix) - utf8Len(ext);
    // the longest prefix of whole code points that fits
    let used = 0;
    let take = 0;
    for (const ch of stem) {
      const b = utf8Len(ch);
      if (used + b > budget) break;
      used += b;
      take++;
    }
    let head = stem.slice(0, take).join("");
    if (take < stem.length) {
      // the cut removed something: strip trailing dots and spaces, empty -> file, reserved -> _ prefix
      head = head.replace(/[. ]+$/, "");
      if (head === "") head = "file";
      if (isReservedName(head)) head = "_" + head;
    }
    const candidate = head + suffix + ext;
    if (!taken.has(candidate.toLowerCase())) return candidate;
  }
  return null;
}

// Create-new semantics: the create call is the real collision check. "already exists" -> next suffix; any other failure -> internal.
function createNewModel(input: string, existing: string[], onDisk: string[], createFails?: string): string | { fails: "internal" } {
  const known = [...existing];
  const disk = new Set(onDisk.map((n) => n.toLowerCase()));
  for (;;) {
    const name = cleanFileNameWithin(input, known, MAX_CANDIDATES);
    if (name === null) return { fails: "internal" }; // all 100 candidates are taken
    if (disk.has(name.toLowerCase())) {
      known.push(name);
      continue;
    }
    if (createFails) return { fails: "internal" };
    return name;
  }
}

// ------------------------------------------------------------------ WebSocket messages
type Role = "phone" | "pc";
const MAX_FRAME = 1048576;
const MAX_OFFER_FILES = 800; // more files in one offer: the receiver answers error too_large
const TRANSFER_ID = /^[A-Za-z0-9_-]{1,64}$/;
const INT_FIELDS = ["protocol", "size", "totalSize", "offset", "received", "sentAt"];
const DIRECTION: Record<string, Role | "both"> = {
  hello: "both", ack: "both", error: "both", text: "both", clipboard: "pc", offer: "pc",
  complete: "phone", cancel: "both", unpair: "both", ping: "both",
};
const ACKED = new Set(["text", "clipboard", "offer", "complete", "cancel", "unpair", "ping"]);
const ACTION: Record<string, string> = {
  hello: "handshake_ok", ack: "mark_delivered", error: "mark_failed", text: "deliver_text", clipboard: "set_clipboard",
  offer: "start_download", complete: "mark_transfer_done", cancel: "cancel_transfer", unpair: "remove_peer_and_close", ping: "show_ping_toast",
};
type Outcome = { reply: "ack" | "error" | "none"; action?: string; code?: string; ref?: string | null; close: number | null; sensitive?: boolean };
const isObj = (x: any) => x !== null && typeof x === "object" && !Array.isArray(x);
const isStr = (x: any) => typeof x === "string";
const isNonEmptyStr = (x: any) => typeof x === "string" && x.length > 0;
const validMsgId = (x: any) => typeof x === "string" && cps(x).length >= 1 && cps(x).length <= 64;

// JSON text -> tree. Duplicate keys: the last value wins (JSON.parse does that). A string or key with an
// unpaired surrogate (from a \ud800-style escape) makes the whole text invalid, like a syntax error.
const LONE_SURROGATE = /[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/;
function hasLoneSurrogate(x: any): boolean {
  if (typeof x === "string") return LONE_SURROGATE.test(x);
  if (Array.isArray(x)) return x.some(hasLoneSurrogate);
  if (x !== null && typeof x === "object") return Object.entries(x).some(([k, v]) => LONE_SURROGATE.test(k) || hasLoneSurrogate(v));
  return false;
}
// Nesting depth, counted on the raw text (strings skipped) so a hostile 1 MiB of "[" can't hurt a recursive walk. Top-level object = 1.
const MAX_JSON_DEPTH = 64;
function jsonDepth(text: string): number {
  let depth = 0;
  let max = 0;
  let inString = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inString) {
      if (c === "\\") i++;
      else if (c === '"') inString = false;
    } else if (c === '"') inString = true;
    else if (c === "[" || c === "{") max = Math.max(max, ++depth);
    else if (c === "]" || c === "}") depth--;
  }
  return max;
}
// The text must be strict RFC 8259 JSON (JSON.parse is: no unquoted literals, no 01, +1, NaN, comments or trailing commas),
// at most 64 levels deep, and must not hold a lone surrogate. Anything else is "invalid JSON".
function parseJsonText(text: string): { ok: true; value: any } | { ok: false } {
  if (jsonDepth(text) > MAX_JSON_DEPTH) return { ok: false };
  try {
    const value = JSON.parse(text);
    return hasLoneSurrogate(value) ? { ok: false } : { ok: true, value };
  } catch {
    return { ok: false };
  }
}

function frameBytes(input: any): { kind: "binary" | "text"; bytes: Uint8Array } {
  if (input.binaryHex !== undefined) return { kind: "binary", bytes: unhex(input.binaryHex) };
  if (input.rawHex !== undefined) return { kind: "text", bytes: unhex(input.rawHex) };
  if (input.raw !== undefined) return { kind: "text", bytes: new TextEncoder().encode(input.raw) };
  if (input.synthesize) {
    const s = input.synthesize;
    const frame = { ...s.base, [s.field]: s.fill.repeat(s.fillCount) };
    return { kind: "text", bytes: new TextEncoder().encode(j(frame)) };
  }
  return { kind: "text", bytes: new TextEncoder().encode(j(input.frame)) };
}

function validateFields(type: string, o: any, badInts: Set<string>): boolean {
  const int = (f: string) => !badInts.has(f) && typeof o[f] === "number" && Number.isSafeInteger(o[f]) && o[f] >= 0;
  const id = () => validMsgId(o.id);
  switch (type) {
    case "hello": return id() && int("protocol") && isNonEmptyStr(o.name) && isNonEmptyStr(o.platform) && isStr(o.appVersion);
    case "ack": return validMsgId(o.ref);
    case "error": return isStr(o.code) && isStr(o.message) && (o.ref === undefined || o.ref === null || validMsgId(o.ref));
    // sensitive is an optional strict boolean (null counts as absent; "true", 1 and 0 are invalid)
    case "text": return id() && isNonEmptyStr(o.text) && typeof o.isUrl === "boolean" && (o.sensitive === undefined || o.sensitive === null || typeof o.sensitive === "boolean");
    case "clipboard": return id() && isNonEmptyStr(o.text);
    case "offer": {
      if (!id() || !TRANSFER_ID.test(o.id) || !int("totalSize") || !Array.isArray(o.files) || o.files.length < 1) return false;
      const ids = new Set<string>();
      for (const f of o.files) {
        if (!isObj(f) || !isStr(f.fileId) || !TRANSFER_ID.test(f.fileId) || ids.has(f.fileId)) return false;
        ids.add(f.fileId);
        if (!isStr(f.name) || !isStr(f.mime) || badInts.has("size") || !Number.isSafeInteger(f.size) || f.size < 0) return false;
      }
      return true;
    }
    case "complete": return id() && isStr(o.transferId) && TRANSFER_ID.test(o.transferId);
    case "cancel": return id() && isStr(o.transferId) && TRANSFER_ID.test(o.transferId) && isStr(o.reason);
    case "unpair": return id();
    case "ping": return id() && int("sentAt");
  }
  return false;
}

class Receiver {
  state: "before_hello" | "open";
  // processed message IDs, per paired PC (`device|id`), as the database remembers them
  seen = new Set<string>();
  // offers by transfer ID, shared by all paired PCs: who made it, how its download stands, and its files (`fileId:size`, sorted)
  offers = new Map<string, { device: string; how: "unfinished" | "done" | "canceled" | "failed"; files: string }>();
  // the paired PC on this connection
  device = "first";
  constructor(public role: Role, state: "before_hello" | "open") {
    this.state = state;
  }
  // a new connection: the state starts again, the memory of IDs stays (it is stored in the database). `device`: the connection is from
  // another paired PC.
  reconnect(device?: string) {
    this.state = "before_hello";
    if (device !== undefined) this.device = device;
  }
  transferOver(transferId: string, how: "done" | "canceled" | "failed") {
    const offer = this.offers.get(transferId);
    if (offer) offer.how = how;
  }
  receive(from: Role, input: any): Outcome {
    const fb = frameBytes(input);
    if (fb.kind === "binary") return { reply: "none", action: "close_only", close: 1003 };
    if (fb.bytes.length > MAX_FRAME) return { reply: "none", action: "close_only", close: 1009 };
    let text: string;
    try {
      text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(fb.bytes);
    } catch {
      return { reply: "none", action: "close_only", close: 1007 };
    }
    const bad = (ref: string | null, code = "bad_request", close: number | null = null): Outcome => ({ reply: "error", code, ref, close });
    // step 2: one valid JSON value (duplicate keys: last wins; lone surrogate escapes are invalid), and an object
    const parsed = parseJsonText(text);
    if (!parsed.ok) return bad(null);
    const o: any = parsed.value;
    if (!isObj(o)) return bad(null);
    const ref = validMsgId(o.id) ? (o.id as string) : null;
    // step 3
    if (!isStr(o.type)) return bad(ref);
    const type: string = o.type;
    // step 4: before a hello is accepted, only a hello may follow. Only here does the connection close with 1002.
    if (this.state === "before_hello" && type !== "hello") return bad(ref, "bad_request", 1002);
    // step 5
    const dir = DIRECTION[type];
    if (dir === undefined || (dir !== "both" && dir !== from)) return bad(ref, "unsupported_type");
    // integers must be written as integer literals: look at the raw text of the fields we know
    const badInts = new Set<string>();
    for (const f of INT_FIELDS) {
      for (const m of text.matchAll(new RegExp(`"${f}"\\s*:\\s*(-?[0-9][0-9.eE+\\-]*)`, "g"))) {
        if (!/^(0|[1-9][0-9]*)$/.test(m[1])) badInts.add(f);
      }
    }
    // step 6: for hello, protocol is checked before every other field
    if (type === "hello" && !badInts.has("protocol") && typeof o.protocol === "number" && Number.isSafeInteger(o.protocol) && o.protocol >= 0 && o.protocol !== 1)
      return bad(ref, "unsupported_version", 4002);
    // step 7
    if (!validateFields(type, o, badInts)) return bad(ref);
    // a valid offer with more than 800 files is rejected (an error, no ack, nothing remembered)
    if (type === "offer" && o.files.length > MAX_OFFER_FILES) return bad(ref, "too_large");
    if (type === "hello") {
      this.state = "open";
      return { reply: "none", action: ACTION.hello, close: null };
    }
    // ack and error are never acked and have no id; an error without a ref has nothing to mark failed
    if (type === "error") return { reply: "none", action: o.ref ? "mark_failed" : "show_error_only", close: null };
    if (!ACKED.has(type)) return { reply: "none", action: ACTION[type], close: null };
    // step 8
    const seenKey = `${this.device}|${o.id}`;
    if (type === "offer") {
      // changelog 52: the same paired PC repeating its own offer is a repeat WHATEVER the state of the download (acked with the first
      // response, nothing downloaded again). Any other reuse is bad_request with ref = the offer id: an offer of another PC, the id of any
      // other message, and (optional) a repeat that lists other fileIds or sizes.
      const known = this.offers.get(o.id);
      if (known && known.device !== this.device) return bad(ref);
      const files = o.files.map((f: any) => `${f.fileId}:${f.size}`).sort().join(",");
      if (known) return known.files === files ? { reply: "ack", ref: o.id, action: "ignore_duplicate", close: null } : bad(ref);
      if (this.seen.has(seenKey)) return bad(ref);
      this.offers.set(o.id, { device: this.device, how: "unfinished", files });
    } else if (this.seen.has(seenKey)) {
      return { reply: "ack", ref: o.id, action: "ignore_duplicate", close: null };
    }
    this.seen.add(seenKey);
    return { reply: "ack", ref: o.id, action: ACTION[type], close: type === "unpair" ? 1000 : null, ...(type === "text" ? { sensitive: o.sensitive === true } : {}) };
  }
}

// The 10-second rule: no hello accepted within 10 seconds of the socket opening closes it with 1002 (no error message first).
const HELLO_TIMEOUT_MS = 10000;
const closeAfterSilence = (state: "before_hello" | "open", elapsedSinceOpenMs: number): number | null =>
  state === "before_hello" && elapsedSinceOpenMs > HELLO_TIMEOUT_MS ? 1002 : null;

// What a client does after the connection ends (close code, or null for a network error / no code) or the upgrade is refused.
type CloseAction = "reconnect_with_backoff" | "reconnect_on_user_action" | "stop_update_app" | "stop_pair_again";
function clientActionOnClose(ev: { closeCode?: number | null; upgradeStatus?: number }): CloseAction {
  if (ev.upgradeStatus === 401) return "stop_pair_again";
  switch (ev.closeCode) {
    case 4001: return "reconnect_on_user_action";
    case 4002: return "stop_update_app";
    case 4003: return "stop_pair_again";
    default: return "reconnect_with_backoff"; // 1000, 1001, 1002, 1003, 1006, 1007, 1009, 1011, unknown codes, none
  }
}

// The backoff resets only when a connection stayed open with the peer's hello accepted for at least 60 s (monotonic clock, counted from
// the accepted hello). An instantly closed connection, or one where no hello was ever accepted, leaves it where it was.
const BACKOFF_RESET_AFTER_MS = 60000;
const backoffResets = (helloAcceptedForMs: number | null): boolean => helloAcceptedForMs !== null && helloAcceptedForMs >= BACKOFF_RESET_AFTER_MS;

function sameOutcome(actual: Outcome, expect: any): boolean {
  if (actual.reply !== expect.reply) return false;
  if ((actual.close ?? null) !== (expect.close ?? null)) return false;
  if (expect.action !== undefined && actual.action !== expect.action) return false;
  if (actual.reply === "error") {
    if (actual.code !== expect.code) return false;
    if ((actual.ref ?? null) !== (expect.ref ?? null)) return false;
  }
  // an ack carries the id of the message it answers; a case may pin it (the duplicate-key case does)
  if (actual.reply === "ack" && expect.ref !== undefined && actual.ref !== expect.ref) return false;
  // a deliver_text case says whether the text is sensitive; absent means false
  if (actual.action === "deliver_text" && (expect.sensitive ?? false) !== (actual.sensitive ?? false)) return false;
  return true;
}

// ------------------------------------------------------------------ HTTP model
const JSON_CT = "application/json; charset=utf-8";
// Content-Type of a request body (POST /v1/pair, POST /v1/transfers): the media type is application/json, ASCII case-insensitive,
// with no parameter or exactly charset=utf-8. A missing header, another type, another charset or any other parameter: not JSON.
const isJsonContentType = (value: string | undefined): boolean => value !== undefined && /^application\/json[ \t]*(;[ \t]*charset=utf-8)?$/i.test(value);
const STATUS_OF: Record<string, number[]> = {
  bad_request: [400, 416], unauthorized: [401], bad_token: [401], token_expired: [401], token_used: [401],
  unsupported_version: [426], not_found: [404], offset_mismatch: [409], source_changed: [409],
  too_large: [413], disk_full: [507], internal: [500],
};
type Reply = { status: number; error?: string; offset?: number; body?: any; headers?: Record<string, string> };
const err = (status: number, error: string, extra: Partial<Reply> = {}): Reply => ({ status, error, ...extra });

function serve(ctx: any, req: any): Reply {
  if (ctx.server?.bug) return err(500, "internal");
  const [path, query = ""] = req.path.split("?");
  const method: string = req.method;
  const headers: Record<string, string> = req.headers ?? {};
  const client = ctx.client ?? {};
  // A client certificate is mandatory in the TLS handshake, so a request always has a device ID.
  // "paired" says whether that ID is in devices. A transfer or offer belongs to the device that created / received it.
  const authorized = typeof client.deviceId === "string" && client.paired === true;
  const owns = (x: any) => x.owner === undefined || x.owner === client.deviceId;
  const jsonType = isJsonContentType(headers["Content-Type"]);
  const body = req.body;
  // Free space on the PC: diskFull is freeBytes 0. The check counts only the bytes the PC does not have yet, with no reserve.
  const freeBytes: number = ctx.server?.diskFull ? 0 : ctx.server?.freeBytes ?? Number.POSITIVE_INFINITY;

  if (method === "POST" && path === "/v1/pair") {
    // POST /v1/pair never answers 401 unauthorized: any certificate is fine. Order: 413, then body invalid (400), then 426, then the token.
    if ((req.bodyBytes ?? 0) > MAX_FRAME) return err(413, "too_large");
    if (!jsonType || !isObj(body)) return err(400, "bad_request");
    const optStr = (f: string) => body[f] === undefined || body[f] === null || isStr(body[f]);
    const okBody = isStr(body.token) && isNonEmptyStr(body.name) && isNonEmptyStr(body.platform) &&
      typeof body.protocol === "number" && Number.isSafeInteger(body.protocol) && body.protocol >= 0 && optStr("model") && optStr("appVersion");
    if (!okBody) return err(400, "bad_request");
    if (body.protocol !== 1) return err(426, "unsupported_version");
    const t = ctx.pairingToken;
    if (!t || t.state === "none" || body.token !== t.value) {
      if (t && t.state === "active") {
        t.failures = (t.failures ?? 0) + 1;
        if (t.failures >= 5) t.state = "invalidated";
      }
      return err(401, "bad_token");
    }
    if (t.state === "used") return err(401, "token_used"); // "used" is checked first: a used token that has also expired is token_used
    if (t.state === "expired" || t.state === "invalidated" || t.expired === true) return err(401, "token_expired");
    t.state = "used";
    return { status: 200, body: { name: "Desk PC", deviceId: ctx.pcDeviceId, protocol: 1 } };
  }

  if (method === "POST" && path === "/v1/transfers") {
    if (!authorized) return err(401, "unauthorized");
    if ((req.bodyBytes ?? 0) > MAX_FRAME) return err(413, "too_large");
    if (!jsonType || !isObj(body) || !isStr(body.id) || !TRANSFER_ID.test(body.id) || !Array.isArray(body.files) || body.files.length < 1) return err(400, "bad_request");
    const ids = new Set<string>();
    for (const f of body.files) {
      if (!isObj(f) || !isStr(f.fileId) || !TRANSFER_ID.test(f.fileId) || ids.has(f.fileId)) return err(400, "bad_request");
      ids.add(f.fileId);
      if (!isStr(f.name) || !isStr(f.mime) || !Number.isSafeInteger(f.size) || f.size < 0) return err(400, "bad_request");
    }
    const stored = ctx.transfer && ctx.transfer.id === body.id ? ctx.transfer : undefined;
    if (stored && !owns(stored)) return err(404, "not_found"); // belongs to another device
    // a transfer ID is never used twice: one that was used and is over (canceled, or finished more than 24 hours ago) is 404, not a new transfer
    if (!stored && ctx.usedIds?.includes(body.id)) return err(404, "not_found");
    if (!stored && ctx.offer && ctx.offer.id === body.id) return err(404, "not_found"); // the ID is a PC -> phone offer: it can't be created as an upload
    if (stored) {
      // a repeated POST resumes only with the same fileIds (as a set); anything else is bad_request and the stored list stays
      const have = Object.keys(stored.files);
      if (have.length !== ids.size || !have.every((k) => ids.has(k))) return err(400, "bad_request");
    }
    // too large and disk full are checked again for a resumed POST; only a new transfer creates files
    if (ctx.server?.tooLarge) return err(413, "too_large");
    const missing = body.files.reduce((sum: number, f: any) => sum + (f.size - (stored?.files[f.fileId]?.stored ?? 0)), 0);
    if (!stored ? body.files.reduce((a: number, f: any) => a + f.size, 0) > freeBytes : missing > freeBytes) return err(507, "disk_full");
    if (!stored && ctx.server?.createFails) return err(500, "internal");
    return { status: 200, body: { id: body.id, files: body.files.map((f: any) => ({ fileId: f.fileId, offset: stored?.files[f.fileId]?.stored ?? 0 })) } };
  }

  // PUT ignores Content-Type (jsonType is deliberately not used below). GET and DELETE have no body.
  const m = /^\/v1\/transfers\/([^/]+)\/files\/([^/]+)$/.exec(path);
  if (m && method === "PUT") {
    if (!authorized) return err(401, "unauthorized");
    const file = ctx.transfer && ctx.transfer.id === m[1] && owns(ctx.transfer) ? ctx.transfer.files[m[2]] : undefined;
    if (!file) return err(404, "not_found");
    // offset: ASCII digits, no sign, no leading zeros, at most 2^53-1
    // (given twice is 400, whatever the values)
    const offs = [...query.matchAll(/(?:^|&)offset=([^&]*)/g)];
    const om = offs.length === 1 ? offs[0] : null;
    if (!om || !/^(0|[1-9][0-9]*)$/.test(om[1]) || BigInt(om[1]) > 9007199254740991n) return err(400, "bad_request");
    if (headers["Content-Length"] === undefined) return err(400, "bad_request");
    const n = Number(om[1]);
    if (n !== file.stored) return err(409, "offset_mismatch", { offset: file.stored });
    if (Number(headers["Content-Length"]) !== file.size - n) return err(400, "bad_request");
    if (file.size - n > freeBytes) return err(507, "disk_full"); // this file's remaining bytes
    if (ctx.server?.createFails) return err(500, "internal"); // the .part file can't be created or written
    return { status: 200, body: { received: file.size } };
  }
  if (m && method === "GET") {
    if (!authorized) return err(401, "unauthorized");
    const offer = ctx.offer && ctx.offer.id === m[1] && owns(ctx.offer) ? ctx.offer : undefined;
    if (!offer || offer.expired) return err(404, "not_found");
    const file = offer.files[m[2]];
    if (!file) return err(404, "not_found");
    if (offer.sourceChanged) return err(409, "source_changed");
    const ct = { "Content-Type": "application/octet-stream", "Accept-Ranges": "bytes" };
    const range = headers["Range"];
    if (range === undefined) return { status: 200, headers: { ...ct, "Content-Length": String(file.size) } };
    const rm = /^bytes=(0|[1-9][0-9]*)-$/.exec(range);
    if (!rm || Number(rm[1]) >= file.size) return err(416, "bad_request", { headers: { "Content-Range": `bytes */${file.size}` } });
    const start = Number(rm[1]);
    return { status: 206, headers: { ...ct, "Content-Length": String(file.size - start), "Content-Range": `bytes ${start}-${file.size - 1}/${file.size}` } };
  }
  const dm = /^\/v1\/transfers\/([^/]+)$/.exec(path);
  if (dm && method === "DELETE") {
    if (!authorized) return err(401, "unauthorized");
    // DELETE is only for phone -> PC transfers (ctx.transfer); a PC -> phone offer is not one, so its ID is 404 here
    if (!ctx.transfer || ctx.transfer.id !== dm[1] || !owns(ctx.transfer)) return err(404, "not_found");
    return { status: 204 };
  }
  if (method === "GET" && path === "/v1/ws") {
    if (!authorized) return err(401, "unauthorized");
    return { status: 101 };
  }
  // an unknown route or method is 404 for everyone, paired or not: authorization only applies to endpoints that exist
  return err(404, "not_found");
}

function checkResponse(group: string, name: string, expected: any, actual: Reply) {
  const exp = expected;
  check(group, `${name}: status`, exp.status === actual.status, () => `expected ${exp.status}, model says ${actual.status}`);
  if (actual.error) {
    const b = exp.body;
    check(group, `${name}: error body shape`, isObj(b) && isStr(b.error) && isStr(b.message), () => j(b));
    if (!isObj(b)) return;
    check(group, `${name}: error code`, b.error === actual.error, () => `expected ${b.error}, model says ${actual.error}`);
    const allowed = new Set(["error", "message", ...(b.error === "offset_mismatch" ? ["offset"] : [])]);
    check(group, `${name}: error body keys`, Object.keys(b).every((k) => allowed.has(k)) && (b.error !== "offset_mismatch" || Number.isSafeInteger(b.offset)), () => Object.keys(b).join(","));
    if (actual.offset !== undefined) check(group, `${name}: offset`, b.offset === actual.offset, () => `expected ${b.offset}, model says ${actual.offset}`);
    check(group, `${name}: error content-type`, exp.headers?.["Content-Type"] === JSON_CT, () => String(exp.headers?.["Content-Type"]));
    check(group, `${name}: status matches code table`, (STATUS_OF[b.error] ?? []).includes(exp.status), () => `${b.error} with ${exp.status}`);
    for (const [k, v] of Object.entries(actual.headers ?? {})) check(group, `${name}: header ${k}`, exp.headers?.[k] === v, () => `expected ${exp.headers?.[k]}, model says ${v}`);
  } else {
    if (actual.body !== undefined) {
      check(group, `${name}: body`, eq(exp.body, actual.body), () => `expected ${j(exp.body)}, model says ${j(actual.body)}`);
      check(group, `${name}: json content-type`, exp.headers?.["Content-Type"] === JSON_CT, () => String(exp.headers?.["Content-Type"]));
    }
    for (const [k, v] of Object.entries(actual.headers ?? {})) check(group, `${name}: header ${k}`, exp.headers?.[k] === v, () => `expected ${exp.headers?.[k]}, model says ${v}`);
    if (exp.bodyBytes !== undefined && exp.headers?.["Content-Length"] !== undefined)
      check(group, `${name}: bodyBytes equals Content-Length`, String(exp.bodyBytes) === exp.headers["Content-Length"], () => `${exp.bodyBytes} vs ${exp.headers["Content-Length"]}`);
    if (exp.status === 204) check(group, `${name}: 204 has no body`, exp.body === undefined && exp.bodyBytes === undefined);
  }
}

// ================================================================== run
// ---- device-id.json
{
  const g = "device-id";
  const d = load("device-id.json");
  uniqueNames(g, d.keys);
  for (const k of d.keys) {
    const priv = createPrivateKey(k.privateKeyPkcs8Pem);
    check(g, `${k.name}: is a P-256 key`, priv.asymmetricKeyType === "ec" && priv.asymmetricKeyDetails?.namedCurve === "prime256v1");
    check(g, `${k.name}: PEM is PKCS#8`, k.privateKeyPkcs8Pem.startsWith("-----BEGIN PRIVATE KEY-----"));
    check(g, `${k.name}: marked test-only`, k.testOnly === true);
    const spki = new Uint8Array(createPublicKey(priv).export({ type: "spki", format: "der" }));
    check(g, `${k.name}: SPKI hex`, hex(spki) === k.spkiDerHex, () => hex(spki));
    check(g, `${k.name}: SPKI length`, spki.length === 91 && k.spkiLength === 91);
    const certSpki = new Uint8Array(new X509Certificate(k.certificatePem).publicKey.export({ type: "spki", format: "der" }));
    check(g, `${k.name}: certificate SPKI equals key SPKI`, hex(certSpki) === k.spkiDerHex);
    // certificate profile (protocol.md, Transport): v3, P-256 key, ecdsa-with-SHA256, serial of 1 to 20 bytes, no critical extension
    const x509 = new X509Certificate(k.certificatePem);
    const der = new Uint8Array(x509.raw);
    const violations = certProfileViolations(der);
    check(g, `${k.name}: certificate matches the profile`, violations.length === 0, () => violations.join("; "));
    check(g, `${k.name}: certificate is self-signed (it verifies with its own key)`, x509.verify(x509.publicKey));
    const exts = certExtensionOids(der);
    check(g, `${k.name}: certificateVariant matches the extensions`, k.certificateVariant === "minimal" ? exts.length === 0 : k.certificateVariant === "with-noncritical-extension" ? exts.length > 0 : false, () => `${k.certificateVariant}: ${exts.join(",")}`);
    check(g, `${k.name}: certificate fits a hand-written DER (at most 400 bytes)`, der.length <= 400, () => `${der.length} bytes`);
    check(g, `${k.name}: sha256`, hex(sha256(spki)) === k.sha256Hex);
    check(g, `${k.name}: device id`, deviceIdOfSpki(spki) === k.deviceId, () => deviceIdOfSpki(spki));
    check(g, `${k.name}: device id is 43 base64url characters`, validDeviceId(k.deviceId));
    check(g, `${k.name}: short id`, shortIdOfHash(sha256(spki)) === k.shortId, () => shortIdOfHash(sha256(spki)));
    check(g, `${k.name}: short id shape`, /^[A-Z2-7]{8}$/.test(k.shortId));
  }
  for (const h of d.hashVectors) {
    const hash = unhex(h.sha256Hex);
    check(g, `${h.name}: 32 bytes`, hash.length === 32);
    check(g, `${h.name}: device id`, deviceIdOfHash(hash) === h.deviceId, () => deviceIdOfHash(hash));
    check(g, `${h.name}: short id`, shortIdOfHash(hash) === h.shortId, () => shortIdOfHash(hash));
    check(g, `${h.name}: matches Node base64url`, Buffer.from(hash).toString("base64url") === h.deviceId);
  }
  for (const t of d.tokenVectors) {
    const bytes = unhex(t.bytesHex);
    check(g, `${t.name}: 16 bytes`, bytes.length === 16);
    check(g, `${t.name}: token`, base64url(bytes) === t.token, () => base64url(bytes));
    check(g, `${t.name}: token is 22 characters`, validToken(t.token));
  }
  for (const s of d.shapeVectors) {
    const ok = s.kind === "deviceId" ? validDeviceId(s.value) : validToken(s.value);
    check(g, `shape ${s.kind} ${j(s.value)}`, ok === s.valid, () => `expected valid=${s.valid}`);
  }
}

// ---- pairing-urls.json
{
  const g = "pairing-urls";
  const d = load("pairing-urls.json");
  uniqueNames(g + ".valid", d.valid);
  uniqueNames(g + ".invalid", d.invalid);
  for (const c of d.valid) {
    const r = parsePairingUrl(c.url);
    check(g, `valid ${c.name}: parses`, r.ok, () => (r.ok ? "" : j(r.error)));
    if (!r.ok) continue;
    check(g, `valid ${c.name}: result`, eq(r.value, c.parsed), () => `expected ${j(c.parsed)}\n       got      ${j(r.value)}`);
    if (c.canonical) check(g, `valid ${c.name}: canonical build`, buildPairingUrl(c.parsed) === c.url, () => buildPairingUrl(c.parsed));
  }
  for (const c of d.invalid) {
    const r = parsePairingUrl(c.url);
    check(g, `invalid ${c.name}: is rejected`, !r.ok, () => (r.ok ? `parsed as ${j(r.value)}` : ""));
    if (r.ok) continue;
    check(g, `invalid ${c.name}: error`, eq(r.error, c.error), () => `expected ${j(c.error)}, got ${j(r.error)}`);
    check(g, `invalid ${c.name}: known category`, d.errorCategories.includes(c.error.category));
  }
}

// ---- file-names.json
{
  const g = "file-names";
  const d = load("file-names.json");
  uniqueNames(g, d.cases);
  for (const c of d.cases) {
    const got = cleanFileName(c.input, c.existing);
    check(g, c.name, got === c.expected, () => `input ${j(c.input)}\n       expected ${j(c.expected)}\n       got      ${j(got)}`);
    check(g, `${c.name}: result is within ${d.maxLength} UTF-8 bytes`, utf8Len(c.expected) <= d.maxLength, () => `${utf8Len(c.expected)} bytes`);
    check(g, `${c.name}: expected name is not in existing`, !c.existing.some((e: string) => e.toLowerCase() === c.expected.toLowerCase()));
    check(g, `${c.name}: expected name is already clean`, cleanFileName(c.expected, []) === c.expected, () => `re-cleaning ${j(c.expected)} gave ${j(cleanFileName(c.expected, []))}`);
    // invariants of every result: never empty, never ends in a dot or space, never a reserved device name, no bidi control
    check(g, `${c.name}: expected name is not empty`, c.expected.length > 0);
    check(g, `${c.name}: expected name does not end in a dot or space`, !/[. ]$/.test(c.expected));
    check(g, `${c.name}: expected name is not a reserved device name`, !isReservedName(c.expected));
    check(g, `${c.name}: expected name has no bidi control character`, !cps(c.expected).some((ch) => isBidiControl(ch.codePointAt(0)!)));
  }
  check(g, "limits are in UTF-8 bytes", d.maxLengthUnit === "utf8-bytes" && d.maxLength === MAX_NAME && d.maxExtensionBytes === MAX_EXT);
  uniqueNames(g + ".createNew", d.createNew);
  // operation: "create" (the file is first created) or "move_part" (name.ext.part is moved to its final name). Both use create-new
  // semantics and the same loop: "already exists" takes the next suffix, any other failure is internal, and it is never a 500 for "exists".
  for (const c of d.createNew) {
    const got = createNewModel(c.input, c.existing, c.onDisk, c.createFails);
    check(g, `create-new ${c.name}`, eq(got, c.expected), () => `expected ${j(c.expected)}, model says ${j(got)}`);
    check(g, `create-new ${c.name}: operation is create or move_part`, c.operation === undefined || ["create", "move_part"].includes(c.operation), () => String(c.operation));
    // a move never fails on "already exists" alone: only the limit of 100 candidates (all of them taken) fails it
    if (c.operation === "move_part" && !c.createFails) check(g, `create-new ${c.name}: a move fails on "already exists" only after all ${MAX_CANDIDATES} candidates are taken`, typeof c.expected === "string" || new Set([...c.onDisk, ...c.existing].map((n: string) => n.toLowerCase())).size >= MAX_CANDIDATES);
  }
  check(g, "create-new has cases for both operations", d.createNew.some((c: any) => c.operation === "move_part") && d.createNew.some((c: any) => c.operation !== "move_part"));
}

// ---- messages.json
{
  const g = "messages";
  const d = load("messages.json");
  uniqueNames(g + ".valid", d.valid);
  uniqueNames(g + ".invalid", d.invalid);
  uniqueNames(g + ".sequences", d.sequences);
  check(g, "maxFrameBytes", d.maxFrameBytes === MAX_FRAME);
  for (const c of [...d.valid, ...d.invalid]) {
    if (c.synthesize) {
      const bytes = frameBytes(c).bytes.length;
      check(g, `${c.name}: synthesized size matches totalBytes`, bytes === c.synthesize.totalBytes, () => `${bytes} vs ${c.synthesize.totalBytes}`);
    }
    check(g, `${c.name}: to is the other role`, c.to === (c.from === "pc" ? "phone" : "pc"));
  }
  for (const c of d.valid) {
    const out = new Receiver(c.to, c.state).receive(c.from, c);
    check(g, `valid ${c.name}`, sameOutcome(out, c.expect), () => `expected ${j(c.expect)}\n       got      ${j(out)}`);
    check(g, `valid ${c.name}: is not an error reply`, out.reply !== "error");
    if (c.frame && out.reply === "ack") check(g, `valid ${c.name}: the ack carries the frame id`, out.ref === c.frame.id, () => `${out.ref} vs ${c.frame.id}`);
  }
  uniqueNames(g + ".timers", d.timers);
  for (const t of d.timers) {
    const got = closeAfterSilence(t.state, t.elapsedSinceOpenMs);
    check(g, `timer ${t.name}`, got === t.expect.close, () => `expected ${t.expect.close}, model says ${got}`);
  }
  uniqueNames(g + ".closeCodes", d.closeCodes);
  for (const c of d.closeCodes) {
    const got = clientActionOnClose(c);
    check(g, `closeCode ${c.name}`, got === c.action, () => `expected ${c.action}, model says ${got}`);
    // optional: how long the peer's hello had been accepted when the connection ended (null = never accepted) and whether the backoff resets
    if (c.resetsBackoff !== undefined) {
      const resets = backoffResets(c.helloAcceptedForMs);
      check(g, `closeCode ${c.name}: backoff reset`, resets === c.resetsBackoff, () => `expected ${c.resetsBackoff}, model says ${resets}`);
    }
  }
  for (const c of d.invalid) {
    const out = new Receiver(c.to, c.state).receive(c.from, c);
    check(g, `invalid ${c.name}`, sameOutcome(out, c.expect), () => `expected ${j(c.expect)}\n       got      ${j(out)}`);
  }
  for (const s of d.sequences) {
    const rx: Record<Role, Receiver> = { phone: new Receiver("phone", "before_hello"), pc: new Receiver("pc", "before_hello") };
    s.steps.forEach((st: any, i: number) => {
      // local events, not frames: a new connection (the receivers are before_hello again, their memory of IDs stays) and the end of a transfer
      if (st.event !== undefined) {
        check(g, `sequence ${s.name} step ${i + 1}: a known event`, ["reconnect", "transfer_over"].includes(st.event), () => String(st.event));
        for (const r of Object.values(rx)) {
          if (st.event === "reconnect") r.reconnect(st.device);
          else {
            check(g, `sequence ${s.name} step ${i + 1}: transfer_over has a transferId and how`, typeof st.transferId === "string" && ["done", "canceled", "failed"].includes(st.how));
            r.transferOver(st.transferId, st.how);
          }
        }
        return;
      }
      const to: Role = st.from === "pc" ? "phone" : "pc";
      // each role has its own receiver state, and the first message a role receives is its peer's hello
      const out = rx[to].receive(st.from, st);
      check(g, `sequence ${s.name} step ${i + 1}`, sameOutcome(out, st.expect), () => `expected ${j(st.expect)}\n       got      ${j(out)}`);
    });
  }
  // every message type has at least one valid example, and every type that is acked is in the acked list
  const types = new Set(d.valid.map((c: any) => c.frame?.type));
  for (const t of Object.keys(DIRECTION)) check(g, `has a valid example of ${t}`, types.has(t));
  for (const t of ACKED) {
    const ok = d.valid.filter((c: any) => c.frame?.type === t).every((c: any) => c.expect.reply === "ack");
    check(g, `every valid ${t} is acked`, ok);
  }
  for (const t of ["hello", "ack", "error"]) check(g, `no valid ${t} is acked`, d.valid.filter((c: any) => c.frame?.type === t).every((c: any) => c.expect.reply === "none"));
}

// ---- http.json
{
  const g = "http";
  const d = load("http.json");
  uniqueNames(g + ".exchanges", d.exchanges);
  uniqueNames(g + ".sequences", d.sequences);
  const PC = load("device-id.json").keys[0].deviceId;
  const PHONE = load("device-id.json").keys[1].deviceId;
  const clone = (x: any) => JSON.parse(JSON.stringify(x));
  // bodyRaw is JSON text sent as is (needed for duplicate keys and lone surrogate escapes, which a JSON value can't express)
  const prepare = (req: any) => {
    if (req.bodyRaw === undefined) return req;
    const p = parseJsonText(req.bodyRaw);
    return { ...req, body: p.ok ? p.value : req.bodyRaw };
  };
  for (const x of d.exchanges) {
    const ctx = clone(x.context);
    ctx.pcDeviceId = PC;
    // a client certificate is mandatory in the handshake, so every exchange has a device ID (the TLS vectors cover "none")
    check(g, `${x.name}: the client has a device ID (a certificate is mandatory)`, typeof x.context.client?.deviceId === "string");
    const reply = serve(ctx, prepare(x.request));
    checkResponse(g, x.name, x.response, reply);
    // a JSON body is sent with a JSON Content-Type unless the case is marked wrongContentType, and the mark must be true
    if ((x.request.body !== undefined || x.request.bodyRaw !== undefined) && x.request.method === "POST")
      check(g, `${x.name}: wrongContentType mark matches the header`, (x.request.wrongContentType === true) === !isJsonContentType(x.request.headers["Content-Type"]), () => String(x.request.headers["Content-Type"]));
    if (x.context.effect?.savedDeviceId) check(g, `${x.name}: saved id comes from the certificate`, x.context.effect.savedDeviceId === x.context.client.deviceId);
    if (x.request.headers?.["Content-Length"] !== undefined && x.request.bodyBytes !== undefined)
      check(g, `${x.name}: request bodyBytes equals Content-Length`, String(x.request.bodyBytes) === x.request.headers["Content-Length"]);
  }
  for (const s of d.sequences) {
    const ctx = clone(s.context);
    ctx.pcDeviceId = PC;
    s.steps.forEach((st: any, i: number) => {
      const label = `${s.name} step ${i + 1}`;
      if (st.clock) {
        // time passes on the PC. A finished upload is answered for 24 hours after its last file arrived, then its ID is gone for good.
        check(g, `${label}: a clock step has hours and no request`, Number.isSafeInteger(st.clock.advanceHours) && st.clock.advanceHours > 0 && st.request === undefined);
        if (ctx.transfer?.finishedHoursAgo !== undefined) {
          ctx.transfer.finishedHoursAgo += st.clock.advanceHours;
          if (ctx.transfer.finishedHoursAgo >= 24) {
            (ctx.usedIds ??= []).push(ctx.transfer.id);
            ctx.transfer = null;
          }
        }
        return;
      }
      if (st.response === null) {
        // interrupted request: the PC keeps what arrived
        const pm = /^\/v1\/transfers\/([^/]+)\/files\/([^/?]+)\?offset=(\d+)$/.exec(st.request.path);
        if (pm && st.interruptedAfterBytes !== undefined) ctx.transfer.files[pm[2]].stored = Number(pm[3]) + st.interruptedAfterBytes;
        check(g, `${label}: interrupted step is marked`, st.interruptedAfterBytes !== undefined);
        return;
      }
      const reply = serve(ctx, prepare(st.request));
      checkResponse(g, label, st.response, reply);
      // effects on the model
      if (st.request.method === "POST" && st.request.path === "/v1/transfers" && reply.status === 200) {
        // a repeated POST leaves the stored transfer (names, sizes, bytes) exactly as it was
        if (!(ctx.transfer && ctx.transfer.id === st.request.body.id))
          ctx.transfer = { id: st.request.body.id, files: Object.fromEntries(st.request.body.files.map((f: any) => [f.fileId, { size: f.size, stored: 0 }])) };
      }
      const pm = /^\/v1\/transfers\/([^/]+)\/files\/([^/?]+)/.exec(st.request.path);
      if (st.request.method === "PUT" && reply.status === 200 && pm) {
        ctx.transfer.files[pm[2]].stored = ctx.transfer.files[pm[2]].size;
        // the last file is in: the upload is finished, and its clock starts
        if (Object.values<any>(ctx.transfer.files).every((f) => f.stored === f.size)) ctx.transfer.finishedHoursAgo ??= 0;
      }
      // a DELETE that answers 204 drops the transfer: a retried DELETE then finds nothing (404), and the ID is never used again
      if (st.request.method === "DELETE" && reply.status === 204) {
        (ctx.usedIds ??= []).push(ctx.transfer.id);
        ctx.transfer = null;
      }
    });
  }
  for (const e of d.errors) {
    if (e.status === null) {
      check(g, `errors ${e.code}: WebSocket-only`, e.code === "unsupported_type");
      continue;
    }
    check(g, `errors ${e.code}: status matches table`, (STATUS_OF[e.code] ?? []).includes(e.status));
    check(g, `errors ${e.code}: body code`, e.body.error === e.code);
    check(g, `errors ${e.code}: body keys`, Object.keys(e.body).every((k) => ["error", "message", ...(e.code === "offset_mismatch" ? ["offset"] : [])].includes(k)));
  }
  const codes = new Set(d.errors.map((e: any) => e.code));
  for (const c of [...Object.keys(STATUS_OF), "unsupported_type"]) check(g, `errors list covers ${c}`, codes.has(c));
  // TLS handshake rules. A client certificate is mandatory, any client chain is accepted, and the CertificateVerify
  // signature is always verified (by the PC for the phone's certificate, by the phone for the server's).
  uniqueNames(g + ".tls", d.tls);
  for (const t of d.tls) {
    let ok: boolean;
    // certificateProfile says whether the presented leaf is inside the certificate profile (absent = it is): a v1 certificate, an
    // unknown critical extension or a key that is not P-256 is refused (rustls-webpki rejects them), whatever the issuer and dates are
    const inProfile = (t.certificateProfile ?? "ok") === "ok";
    if (t.verifier === "pc") ok = t.clientCertificate !== "none" && t.certificateVerify === "valid" && inProfile;
    else ok = t.serverCertificate.spkiMatchesPinnedId === true && t.certificateVerify === "valid"; // hostname and expiry are ignored
    check(g, `tls ${t.name}`, (ok ? "handshake_ok" : "handshake_fails") === t.expect, () => `expected ${t.expect}, model says ${ok ? "handshake_ok" : "handshake_fails"}`);
    // TLS 1.3: the client finishes its handshake before the server checks the client certificate, so a refused client sees the
    // failure on its first read. The phone checks the server's certificate inside its own handshake call.
    if (t.expect === "handshake_fails") check(g, `tls ${t.name}: where the failure shows up`, t.failureSurfacesOn === (t.verifier === "pc" ? "client_first_read" : "client_handshake"), () => String(t.failureSurfacesOn));
    else check(g, `tls ${t.name}: no failure to place`, t.failureSurfacesOn === undefined);
    // the platform guarantees the phone's check of the server's signature, so that one case is optional; nothing else is
    if (t.optional !== undefined) check(g, `tls ${t.name}: optional only for the phone-side signature case`, t.optional === true && t.verifier === "phone" && t.certificateVerify === "invalid");
    if (t.thenHttp) {
      const x = d.exchanges.find((e: any) => e.name === t.thenHttp);
      check(g, `tls ${t.name}: the follow-up exchange exists`, x !== undefined && ok, () => t.thenHttp);
      if (x) check(g, `tls ${t.name}: the follow-up exchange has a device ID`, typeof x.context.client.deviceId === "string");
    }
  }
  // the paired phone in these vectors is test key 2
  check(g, "paired phone is test key 2", d.exchanges.find((x: any) => x.name === "pair-ok").context.client.deviceId === PHONE);
}

// ---- milestone 1.1 scope: every name listed in protocol/README.md ("What each milestone needs") and in docs/phases/phase-1-mvp.md
// must match at least one case, so the list can't name something that was renamed or removed. A trailing * means "starts with".
{
  const g = "scope-1.1";
  const messages = load("messages.json");
  const http = load("http.json");
  const SCOPE: [string, any[], string[]][] = [
    ["messages.valid", messages.valid, ["hello-*", "ack", "ack-from-pc", "error-*", "ping-*", "unpair-*", "nesting-depth-64-*", "brackets-inside-strings-do-not-count-as-nesting"]],
    ["messages.invalid", messages.invalid, [
      "first-message-*", "hello-*", "ack-*", "error-*", "unpair-missing-id", "ping-*", "type-*", "unknown-type-is-checked-before-fields", "frame-*", "binary-frame*",
      "text-frame-1-MiB-plus-1-byte", "text-frame-multibyte-over-limit", "text-frame-invalid-utf8", "text-lone-*", "text-high-surrogate-followed-by-non-low-escape",
      "text-reversed-surrogate-pair-escape", "lone-surrogate-escape-*", "strict-*", "nesting-*"]],
    ["messages.sequences", messages.sequences, [
      "second-hello-after-handshake-is-a-normal-message", "invalid-hello-does-not-count-as-accepted", "garbage-before-hello-keeps-the-connection-open",
      "future-hello-gets-unsupported-version-not-a-timeout", "duplicate-ping-shows-one-toast"]],
    ["http.exchanges", http.exchanges, ["pair-*", "ws-upgrade-*", "unknown-route*", "unknown-version-path", "wrong-method-on-known-route*"]],
    ["http.sequences", http.sequences, ["pair-rate-limit-per-token", "pair-token-is-single-use"]],
  ];
  let inScope = 0;
  for (const [label, cases, patterns] of SCOPE) {
    for (const p of patterns) {
      const hits = cases.filter((c) => (p.endsWith("*") ? c.name.startsWith(p.slice(0, -1)) : c.name === p));
      check(g, `${label}: "${p}" matches a case`, hits.length > 0);
    }
    inScope += cases.filter((c) => patterns.some((p) => (p.endsWith("*") ? c.name.startsWith(p.slice(0, -1)) : c.name === p))).length;
  }
  // the whole arrays that are in scope exist and are not empty
  for (const [file, arrays] of [["device-id.json", ["keys", "hashVectors", "tokenVectors", "shapeVectors"]], ["pairing-urls.json", ["valid", "invalid"]], ["http.json", ["errors", "tls"]], ["messages.json", ["timers", "closeCodes"]]] as const) {
    const d = load(file);
    for (const a of arrays) check(g, `${file} ${a}[] exists`, Array.isArray(d[a]) && d[a].length > 0);
  }
  console.log(`milestone 1.1 scope: ${inScope} cases in messages.json and http.json, plus the whole of the arrays listed in the README`);
}

// ---- milestone 1.2 scope (text, links and the clipboard): the text and clipboard cases of messages.json. Same rule: every name in
// protocol/README.md must match a case, and the 1.3 cases (offer, complete, cancel) stay out of it.
{
  const g = "scope-1.2";
  const messages = load("messages.json");
  const SCOPE: [string, any[], string[], number][] = [
    ["messages.valid", messages.valid, ["text-*", "clipboard-*", "duplicate-keys-*"], 14],
    ["messages.invalid", messages.invalid, ["text-*", "clipboard-*"], 24],
    ["messages.sequences", messages.sequences, ["handshake-then-text-then-duplicate", "invalid-message-keeps-connection-open", "rejected-message-id-is-not-remembered"], 3],
  ];
  const matches = (name: string, p: string) => (p.endsWith("*") ? name.startsWith(p.slice(0, -1)) : name === p);
  let inScope = 0;
  for (const [label, cases, patterns, atLeast] of SCOPE) {
    for (const p of patterns) check(g, `${label}: "${p}" matches a case`, cases.some((c) => matches(c.name, p)));
    const hits = cases.filter((c) => patterns.some((p) => matches(c.name, p)));
    inScope += hits.length;
    check(g, `${label}: at least ${atLeast} cases in scope`, hits.length >= atLeast, () => String(hits.length));
    // nothing of 1.3 sneaks in through a wide pattern
    check(g, `${label}: no offer, complete or cancel case in the 1.2 scope`, hits.every((c) => !/^(offer|complete|cancel|duplicate-offer)/.test(c.name)));
  }
  // a text or clipboard case goes to the role that can receive it: `clipboard` only to the phone, `text` to either
  for (const c of messages.valid.filter((c: any) => c.name.startsWith("clipboard-"))) check(g, `${c.name}: a valid clipboard goes to the phone`, c.to === "phone");
  console.log(`milestone 1.2 scope: ${inScope} cases in messages.json (text and clipboard)`);
}

// ---- urls.json: the openable-URL rule (docs/protocol.md, "Text messages", "Links")
// Whitespace is the Unicode White_Space set, written out so no platform's isWhitespace() decides. Control characters: U+0000-U+001F, U+007F-U+009F.
const isWhiteSpaceCp = (cp: number) =>
  (cp >= 0x09 && cp <= 0x0d) || cp === 0x20 || cp === 0x85 || cp === 0xa0 || cp === 0x1680 || (cp >= 0x2000 && cp <= 0x200a) ||
  cp === 0x2028 || cp === 0x2029 || cp === 0x202f || cp === 0x205f || cp === 0x3000;
const isControlCp = (cp: number) => cp <= 0x1f || (cp >= 0x7f && cp <= 0x9f);
const MAX_URL_BYTES = 8192;
const sha256Hex = (b: Uint8Array) => hex(sha256(b));

function trimWhiteSpace(s: string): string {
  const chars = Array.from(s);
  let a = 0;
  let b = chars.length;
  while (a < b && isWhiteSpaceCp(chars[a].codePointAt(0)!)) a++;
  while (b > a && isWhiteSpaceCp(chars[b - 1].codePointAt(0)!)) b--;
  return chars.slice(a, b).join("");
}

function isOpenableUrl(text: string): boolean {
  const t = trimWhiteSpace(text);
  if (utf8Len(t) > MAX_URL_BYTES) return false;
  const scheme = /^https?:\/\//i.exec(t);
  if (!scheme) return false;
  if (Array.from(t).some((ch) => isWhiteSpaceCp(ch.codePointAt(0)!) || isControlCp(ch.codePointAt(0)!))) return false;
  const rest = t.slice(scheme[0].length);
  const cut = rest.search(/[/?#]/);
  const authority = cut < 0 ? rest : rest.slice(0, cut);
  const at = authority.lastIndexOf("@");
  const hostPart = at >= 0 ? authority.slice(at + 1) : authority;
  let host: string;
  let port: string | undefined;
  if (hostPart.startsWith("[")) {
    const close = hostPart.indexOf("]");
    if (close < 2) return false; // no closing bracket, or nothing inside
    host = hostPart.slice(1, close);
    if (host.includes("[")) return false;
    const after = hostPart.slice(close + 1);
    if (after === "") port = undefined;
    else if (after.startsWith(":")) port = after.slice(1);
    else return false;
  } else {
    const colon = hostPart.indexOf(":");
    host = colon < 0 ? hostPart : hostPart.slice(0, colon);
    port = colon < 0 ? undefined : hostPart.slice(colon + 1);
    if (host === "" || /^\.+$/.test(host)) return false;
  }
  if (port !== undefined) {
    if (!/^[0-9]+$/.test(port)) return false;
    const digits = port.replace(/^0+/, "");
    if (digits === "" || digits.length > 5 || Number(digits) > 65535) return false;
  }
  return true;
}

{
  const g = "urls";
  const d = load("urls.json");
  uniqueNames(g, d.cases);
  check(g, "maxBytes", d.maxBytes === MAX_URL_BYTES);
  for (const c of d.cases) {
    let text: string;
    if (c.synthesize) {
      const s = c.synthesize;
      text = s.prefix + s.fill.repeat(s.fillCount) + (s.suffix ?? "");
      check(g, `${c.name}: synthesized size matches trimmedBytes`, utf8Len(trimWhiteSpace(text)) === s.trimmedBytes, () => `${utf8Len(trimWhiteSpace(text))} vs ${s.trimmedBytes}`);
    } else text = c.input;
    const got = isOpenableUrl(text);
    check(g, c.name, got === c.isUrl, () => `input ${j(text.slice(0, 120))} expected ${c.isUrl}, model says ${got}`);
  }
  check(g, "has accepted and rejected cases", d.cases.some((c: any) => c.isUrl) && d.cases.some((c: any) => !c.isUrl));
  // the rule is about the trimmed text, so trimming never changes the answer
  for (const c of d.cases.filter((c: any) => c.input !== undefined && c.isUrl))
    check(g, `${c.name}: an accepted URL stays accepted with whitespace around it`, isOpenableUrl(`\n \t${c.input}　 `));
}

// ---- text-rules.json: line ends, the sender's size threshold, the sensitive flag
const toWire = (s: string) => s.replace(/\r\n/g, "\n"); // one left-to-right pass: "\r\r\n" becomes "\r\n"
const toWindowsClipboard = (s: string) => s.replace(/(?<!\r)\n/g, "\r\n"); // an LF that follows a CR stays as it is
const SENDER_THRESHOLD = 131072;
function decideSend(kind: "text" | "clipboard", sensitive: boolean, wireBytes: number): string {
  if (kind === "clipboard") return wireBytes > SENDER_THRESHOLD ? "too_large" : "clipboard";
  if (wireBytes <= SENDER_THRESHOLD) return "text";
  return sensitive ? "not_sent" : "file";
}
type Format = string | Record<string, number>;
function excludedBy(formats: Format[]): string | null {
  for (const f of formats) {
    if (typeof f === "string") {
      if (f === "ExcludeClipboardContentFromMonitorProcessing" || f === "Clipboard Viewer Ignore") return f;
    } else {
      for (const [name, value] of Object.entries(f)) if ((name === "CanIncludeInClipboardHistory" || name === "CanUploadToCloudClipboard") && value === 0) return name;
    }
  }
  return null;
}
function senderModel(c: any): { send: string; sensitive?: boolean; reason?: string } {
  if (c.side === "phone") return { send: "text", sensitive: c.extraIsSensitive === true };
  const why = excludedBy(c.formats);
  if (c.trigger === "automatic") return why ? { send: "none", reason: "password_manager" } : { send: "clipboard" };
  return { send: "text", sensitive: why !== null }; // the hotkey is deliberate: it sends such content, marked sensitive
}
function receiverModel(frame: any) {
  const sensitive = frame.sensitive === true;
  return {
    clipboard: "set",
    clipboardMarkedSensitive: sensitive,
    notification: "Text from <device>",
    preview: !sensitive,
    openAction: !sensitive && frame.isUrl === true && isOpenableUrl(frame.text),
    storedInHistory: !sensitive,
    logsTextLengthOrHash: !sensitive,
  };
}
{
  const g = "text-rules";
  const d = load("text-rules.json");
  check(g, "senderThresholdBytes", d.senderThresholdBytes === SENDER_THRESHOLD && SENDER_THRESHOLD === 128 * 1024);
  uniqueNames(g + ".lineEndings", d.lineEndings);
  for (const c of d.lineEndings) {
    if (c.clipboardText !== undefined) check(g, `line ends ${c.name}: toWire`, toWire(c.clipboardText) === c.wire, () => `${j(toWire(c.clipboardText))} vs ${j(c.wire)}`);
    check(g, `line ends ${c.name}: windows clipboard`, toWindowsClipboard(c.wire) === c.windowsClipboard, () => `${j(toWindowsClipboard(c.wire))} vs ${j(c.windowsClipboard)}`);
    check(g, `line ends ${c.name}: hash is of the wire form`, sha256Hex(new TextEncoder().encode(c.wire)) === c.wireSha256Hex);
    if (c.clipboardText !== undefined && c.clipboardText !== c.wire)
      check(g, `line ends ${c.name}: the clipboard form and the wire form hash differently`, sha256Hex(new TextEncoder().encode(c.clipboardText)) !== c.wireSha256Hex);
  }
  uniqueNames(g + ".senderSize", d.senderSize);
  for (const c of d.senderSize) {
    let wireBytes = c.wireBytes;
    if (c.synthesize) {
      const text = c.synthesize.fill.repeat(c.synthesize.fillCount) + (c.synthesize.suffix ?? "");
      check(g, `size ${c.name}: rawBytes`, utf8Len(text) === c.synthesize.rawBytes, () => `${utf8Len(text)} vs ${c.synthesize.rawBytes}`);
      check(g, `size ${c.name}: wireBytes is measured after the line-end conversion`, utf8Len(toWire(text)) === c.wireBytes, () => `${utf8Len(toWire(text))} vs ${c.wireBytes}`);
      wireBytes = utf8Len(toWire(text));
    }
    const got = decideSend(c.kind, c.sensitive === true, wireBytes);
    check(g, `size ${c.name}`, got === c.expected, () => `expected ${c.expected}, model says ${got}`);
  }
  // the worst-case frame for a text at the threshold is still under the receivers' 1 MiB limit (every byte a control character, escaped to 6)
  check(g, "a 128 KiB text escaped as \\u00XX stays under 1 MiB", SENDER_THRESHOLD * 6 + 1024 < MAX_FRAME);
  uniqueNames(g + ".sensitiveSender", d.sensitiveSender);
  for (const c of d.sensitiveSender) {
    const got = senderModel(c);
    check(g, `sender ${c.name}`, eq(got, c.expect), () => `expected ${j(c.expect)}, model says ${j(got)}`);
  }
  uniqueNames(g + ".receiver", d.receiver);
  for (const c of d.receiver) {
    const got = receiverModel(c.frame);
    check(g, `receiver ${c.name}`, eq(got, c.expect), () => `expected ${j(c.expect)}, model says ${j(got)}`);
  }
  const names = new Set(d.windowsSensitiveFormats.formats.map((f: any) => f.name));
  for (const n of ["ExcludeClipboardContentFromMonitorProcessing", "CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"]) check(g, `the Windows receiver writes ${n} for a sensitive text`, names.has(n));
}

// ---- milestone 1.3 scope (files in both directions). Same rule as for 1.1 and 1.2: every name in protocol/README.md must match a case.
{
  const g = "scope-1.3";
  const messages = load("messages.json");
  const http = load("http.json");
  const SCOPE: [string, any[], string[], number][] = [
    ["messages.valid", messages.valid, ["offer-*", "complete-*", "cancel-*", "text-sensitive-*"], 15],
    ["messages.invalid", messages.invalid, ["offer-*", "complete-*", "cancel-*", "text-sensitive-*"], 30],
    ["messages.sequences", messages.sequences, ["duplicate-offer-is-acked-not-restarted"], 1],
    ["http.exchanges", http.exchanges, ["transfers-*", "put-*", "get-*", "delete-*", "internal-error"], 100],
    ["http.sequences", http.sequences, ["upload-*", "download-*", "repeat-with-different-files-keeps-the-stored-list", "delete-*", "transfer-id-*"], 8],
  ];
  const matches = (name: string, p: string) => (p.endsWith("*") ? name.startsWith(p.slice(0, -1)) : name === p);
  let inScope = 0;
  for (const [label, cases, patterns, atLeast] of SCOPE) {
    for (const p of patterns) check(g, `${label}: "${p}" matches a case`, cases.some((c) => matches(c.name, p)));
    const hits = cases.filter((c) => patterns.some((p) => matches(c.name, p)));
    inScope += hits.length;
    check(g, `${label}: at least ${atLeast} cases in scope`, hits.length >= atLeast, () => String(hits.length));
  }
  // 1.1 + 1.2 + 1.3 together cover every exchange and sequence of http.json and every case of messages.json (nothing is orphaned)
  const covered11 = ["pair-*", "ws-upgrade-*", "unknown-route*", "unknown-version-path", "wrong-method-on-known-route*"];
  const covered13 = ["transfers-*", "put-*", "get-*", "delete-*", "internal-error"];
  for (const x of http.exchanges) check(g, `exchange ${x.name} belongs to a milestone`, [...covered11, ...covered13].some((p) => matches(x.name, p)));
  for (const f of ["file-names.json"]) {
    const d = load(f);
    for (const a of ["cases", "createNew"]) check(g, `${f} ${a}[] exists`, Array.isArray(d[a]) && d[a].length > 0);
  }
  for (const f of ["urls.json", "text-rules.json"]) check(g, `${f} exists`, Object.keys(load(f)).length > 0);
  console.log(`milestone 1.3 scope: ${inScope} cases in messages.json and http.json, plus file-names.json, urls.json and text-rules.json`);
}

// ---- milestone 1.4 scope (queue, and the reviewer's rulings on 1.3). Same rule as for the other milestones: every name in
// protocol/README.md must match a case. The `transfer-id-*` sequences are also in the 1.3 pattern list above (they come with it).
{
  const g = "scope-1.4";
  const messages = load("messages.json");
  const http = load("http.json");
  const matches = (name: string, p: string) => (p.endsWith("*") ? name.startsWith(p.slice(0, -1)) : name === p);
  const SCOPE: [string, any[], string[], number][] = [
    ["messages.sequences", messages.sequences, ["offer-reusing-*", "offer-from-another-device-is-bad-request", "offer-repeated-while-unfinished-is-acked-after-a-reconnect", "same-pc-repeat-*", "complete-re-sent-*", "cancel-re-sent-*", "queue-*"], 10],
    ["http.sequences", http.sequences, ["transfer-id-window-*", "transfer-id-retry-after-404-sends-only-the-missing-files"], 3],
  ];
  let inScope = 0;
  for (const [label, cases, patterns, atLeast] of SCOPE) {
    for (const p of patterns) check(g, `${label}: "${p}" matches a case`, cases.some((c) => matches(c.name, p)));
    const hits = cases.filter((c) => patterns.some((p) => matches(c.name, p)));
    inScope += hits.length;
    check(g, `${label}: at least ${atLeast} cases in scope`, hits.length >= atLeast, () => String(hits.length));
  }
  // the window sequences have a clock step of exactly 24 hours in total after the done time
  for (const s of http.sequences.filter((s: any) => s.name.startsWith("transfer-id-window-"))) {
    const hours = s.steps.filter((st: any) => st.clock).reduce((n: number, st: any) => n + st.clock.advanceHours, 0);
    check(g, `${s.name}: the clock moves exactly 24 hours in all`, hours === 24, () => String(hours));
  }
  // an offer that reuses an id is an error with ref = the offer id; a repeat that is not a reuse is an ack
  for (const s of messages.sequences.filter((s: any) => /^(offer-re|offer-from-|same-pc-repeat-)/.test(s.name))) {
    const reused = s.steps.filter((st: any) => st.expect?.reply === "error");
    for (const st of reused) check(g, `${s.name}: a reused offer id answers bad_request with ref = its id`, st.expect.code === "bad_request" && st.expect.ref === st.frame.id && st.frame.type === "offer");
  }
  // changelog 52: a repeat from the same PC after the download is over (done, canceled or failed) is acked, never bad_request. Both sequences
  // end the download with a transfer_over event and reconnect before the repeat.
  for (const [name, how] of [["same-pc-repeat-after-done-is-acked", ["done"]], ["same-pc-repeat-after-canceled-is-acked", ["canceled", "failed"]]] as [string, string[]][]) {
    const s = messages.sequences.find((s: any) => s.name === name);
    check(g, `${name}: exists`, s !== undefined);
    if (!s) continue;
    const ends = s.steps.filter((st: any) => st.event === "transfer_over").map((st: any) => st.how);
    check(g, `${name}: ends the download with ${how.join(" and ")}`, how.every((h) => ends.includes(h)), () => ends.join(","));
    check(g, `${name}: reconnects`, s.steps.some((st: any) => st.event === "reconnect"));
    const repeats = s.steps.filter((st: any) => st.expect?.action === "ignore_duplicate");
    check(g, `${name}: the repeats are acked as duplicates`, repeats.length >= how.length && repeats.every((st: any) => st.expect.reply === "ack" && st.expect.ref === st.frame.id));
  }
  check(g, "the old offer-reused-after-it-is-over sequence is gone", !messages.sequences.some((s: any) => s.name === "offer-reused-after-it-is-over-is-bad-request"));
  console.log(`milestone 1.4 scope: ${inScope} sequences in messages.json and http.json`);
}

// ---- v0.1.1 scope (several files of one transfer at once). The sequential vector format can't interleave requests, so this one pins what
// is checkable per file: independent offsets, a resume POST in the middle of a batch that lists every file, and a file cut off half way.
{
  const g = "scope-0.1.1";
  const http = load("http.json");
  const name = "batch-files-of-one-transfer-are-independent-and-a-resume-leaves-the-others-alone";
  const s = http.sequences.find((s: any) => s.name === name);
  check(g, `${name}: exists`, s !== undefined);
  if (s) {
    const posts = s.steps.filter((st: any) => st.request?.method === "POST");
    const puts = s.steps.filter((st: any) => st.request?.method === "PUT");
    check(g, `${name}: a POST at the start, one in the middle and one at the end`, posts.length === 3, () => String(posts.length));
    check(g, `${name}: one PUT is cut off`, puts.filter((st: any) => st.response === null && st.interruptedAfterBytes > 0).length === 1);
    // the middle POST reports the complete file complete, the cut one at what arrived, and the untouched one at 0
    const mid = posts[1].response.body.files;
    check(g, `${name}: the resume POST reports f1 complete, f2 at the cut and f3 at 0`, mid[0].offset === 5 && mid[1].offset === puts.find((st: any) => st.response === null).interruptedAfterBytes && mid[2].offset === 0);
    check(g, `${name}: every PUT that has a body continues from the offset its URL says`, puts.every((st: any) => st.request.headers["Content-Length"] === String(st.request.bodyBytes)));
    const last = posts[2].response.body.files.map((f: any) => f.offset);
    check(g, `${name}: the last POST says every file is complete`, last[0] === 5 && last[1] === 2097152 && last[2] === 7, () => last.join(","));
  }
  console.log("v0.1.1 scope: 1 sequence in http.json");
}

// ---- fixtures/: DER certificates for the negative tls[] cases (made by fixtures/make-fixtures.ts)
{
  const g = "fixtures";
  const read = (f: string) => new Uint8Array(readFileSync(join(dir, "fixtures", f)));
  const expectations: [string, RegExp][] = [
    ["client-v1.der", /version is v1/],
    ["client-unknown-critical-extension.der", /critical extension 1\.3\.6\.1\.4\.1\.99999\.1/],
    ["client-not-p256.der", /curve is not prime256v1/],
  ];
  for (const [file, expected] of expectations) {
    const der = read(file);
    const violations = certProfileViolations(der);
    check(g, `${file}: leaves the profile in the way its name says`, violations.some((v) => expected.test(v)), () => violations.join("; "));
    // Everything else is fine, so a refused handshake is refused for that one reason. (P-384 also changes the SPKI size, which is the same reason.)
    const others = violations.filter((v) => !expected.test(v) && !(file === "client-not-p256.der" && /SPKI|point/.test(v)));
    check(g, `${file}: nothing else is wrong with it`, others.length === 0, () => others.join("; "));
    // The signature is valid: the certificate is self-signed with its own key.
    const x509 = new X509Certificate(Buffer.from(der));
    check(g, `${file}: signature verifies with its own key`, x509.verify(x509.publicKey));
  }
  // The v1 and critical-extension certificates carry test key 2 of device-id.json, so a handshake that signs with key 2 has a valid CertificateVerify.
  const key2 = load("device-id.json").keys[1];
  for (const file of ["client-v1.der", "client-unknown-critical-extension.der"]) {
    const spki = new Uint8Array(new X509Certificate(Buffer.from(read(file))).publicKey.export({ type: "spki", format: "der" }));
    check(g, `${file}: carries the key of device-id.json test key 2`, hex(spki) === key2.spkiDerHex);
  }
  const p384 = createPrivateKey({ key: Buffer.from(read("client-not-p256.key.der")), format: "der", type: "pkcs8" });
  check(g, "client-not-p256.key.der is a P-384 key", p384.asymmetricKeyType === "ec" && p384.asymmetricKeyDetails?.namedCurve === "secp384r1");

  // The Android-generated certificate (exported by the app's own CertificateBuilder) is inside the profile, and its sidecar agrees.
  const android = read("android-client.der");
  const androidSide = JSON.parse(readFileSync(join(dir, "fixtures", "android-client.json"), "utf8"));
  const androidViolations = certProfileViolations(android);
  check(g, "android-client.der: inside the certificate profile", androidViolations.length === 0, () => androidViolations.join("; "));
  const androidX509 = new X509Certificate(Buffer.from(android));
  check(g, "android-client.der: signature verifies with its own key", androidX509.verify(androidX509.publicKey));
  const androidSpki = new Uint8Array(androidX509.publicKey.export({ type: "spki", format: "der" }));
  check(g, "android-client.json: spkiHex", hex(androidSpki) === androidSide.spkiHex);
  check(g, "android-client.json: deviceId", deviceIdOfSpki(androidSpki) === androidSide.deviceId, () => deviceIdOfSpki(androidSpki));
}

// ---- base64url sanity: the hand-written encoder agrees with Node on random input
{
  for (let n = 0; n < 200; n++) {
    const bytes = new Uint8Array(Array.from({ length: n % 40 }, () => Math.floor(Math.random() * 256)));
    check("self-check", `base64url length ${bytes.length}`, base64url(bytes) === Buffer.from(bytes).toString("base64url"));
  }
}

const total = Object.values(counts).reduce((a, b) => a + b, 0);
console.log("\nChecks per group:");
for (const [k, v] of Object.entries(counts)) console.log(`  ${k.padEnd(14)} ${v}`);
console.log(failures === 0 ? `\nOK: all ${total} checks passed.` : `\nFAILED: ${failures} of ${total} checks failed.`);
process.exit(failures === 0 ? 0 : 1);
