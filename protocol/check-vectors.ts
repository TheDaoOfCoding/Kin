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
  // changelog 55: an authority with user information, a backslash or an escape is ambiguous (a browser reads it another way) and is not openable
  if (/[@\\%]/.test(authority)) return false;
  const hostPart = authority;
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
    if (host === "" || /^\.+$/.test(host) || /[\[\]<>^|]/.test(host)) return false;
  }
  if (port !== undefined) {
    if (!/^[0-9]+$/.test(port)) return false;
    const digits = port.replace(/^0+/, "");
    if (digits === "" || digits.length > 5 || Number(digits) > 65535) return false;
  }
  return true;
}

// ---- link message rules (docs/protocol.md, "Links: link"; changelog 54)
// link.url is the openable-URL rule applied to the string as sent: no trimming, so whitespace at either end is invalid.
const isLinkUrl = (url: unknown): url is string => typeof url === "string" && url === trimWhiteSpace(url) && isOpenableUrl(url);
// The address a receiver shows (docs/protocol.md, "Showing the address of a link"): the host a browser connects to, and whether it carries a warning.
// The model asks the WHATWG parser of the runtime (Bun here) about `scheme://authority/`; a host it refuses is shown as written, ASCII lower-cased.
function displayAddress(url: string): { display: string; warn: boolean } {
  const scheme = /^https?:\/\//i.exec(url)![0].toLowerCase();
  const authority = url.slice(scheme.length).split(/[/?#]/)[0];
  const written = authority.startsWith("[") ? authority.slice(0, authority.indexOf("]") + 1) : authority.split(":")[0];
  let parsed: URL | null = null;
  try {
    parsed = new URL(scheme + authority + "/");
  } catch {
    parsed = null;
  }
  if (parsed === null) {
    const lower = written.replace(/[A-Z]/g, (c) => c.toLowerCase());
    return { display: Array.from(lower, (ch) => (ch >= "!" && ch <= "~" ? ch : "�")).join(""), warn: true };
  }
  const isIp = parsed.hostname.startsWith("[") || /^\d+\.\d+\.\d+\.\d+$/.test(parsed.hostname);
  const warn = /[^\x00-\x7f]/.test(written) || parsed.hostname.split(".").some((label) => label.startsWith("xn--")) || isIp;
  return { display: parsed.host, warn };
}
const isBidiControlCp = (cp: number) => cp === 0x061c || cp === 0x200e || cp === 0x200f || (cp >= 0x202a && cp <= 0x202e) || (cp >= 0x2066 && cp <= 0x2069);
// title and sourceApp: bidi controls -> "_", control characters -> a space, runs of spaces collapse and the ends are trimmed,
// cut to max code points (trailing spaces trimmed again), empty = absent (null).
function cleanLinkText(input: string, max: number): string | null {
  let out = "";
  for (const ch of input) {
    const cp = ch.codePointAt(0)!;
    out += isBidiControlCp(cp) ? "_" : isControlCp(cp) ? " " : ch;
  }
  out = out.replace(/ +/g, " ").replace(/^ | $/g, "");
  out = Array.from(out).slice(0, max).join("").replace(/ +$/, "");
  return out === "" ? null : out;
}
const MAX_TITLE = 300;
const MAX_SOURCE_APP = 64;
// the capabilities of a hello: an array of strings; anything else counts as none, and entries that are not strings are skipped
const capabilitiesOf = (hello: any): Set<string> => new Set(Array.isArray(hello?.capabilities) ? hello.capabilities.filter((c: unknown) => typeof c === "string") : []);
// How a sender puts a link on the wire: as a link only to a peer whose latest accepted hello lists "link", else as a text.
const sendLinkAs = (hellos: any[]): "link" | "text" => (hellos.length > 0 && capabilitiesOf(hellos[hellos.length - 1]).has("link") ? "link" : "text");
const linkOf = (o: any) => {
  const link: any = { url: o.url };
  const title = typeof o.title === "string" ? cleanLinkText(o.title, MAX_TITLE) : null;
  const sourceApp = typeof o.sourceApp === "string" ? cleanLinkText(o.sourceApp, MAX_SOURCE_APP) : null;
  if (title !== null) link.title = title;
  if (sourceApp !== null) link.sourceApp = sourceApp;
  return link;
};


// ------------------------------------------------------------------ WebSocket messages
type Role = "phone" | "pc";
const MAX_FRAME = 1048576;
const MAX_OFFER_FILES = 800; // more files in one offer: the receiver answers error too_large
const TRANSFER_ID = /^[A-Za-z0-9_-]{1,64}$/;
const INT_FIELDS = ["protocol", "size", "totalSize", "offset", "received", "sentAt", "importance", "apps", "i", "durationMs", "positionMs", "rateMilli", "seekMs", "level", "delta", "remainingMs", "waitedMs"];
const SIGNED_INT_FIELDS = new Set(["delta"]); // the one integer of the protocol that may be negative (media.cmd, changelog 57)
const DIRECTION: Record<string, Role | "both"> = {
  hello: "both", ack: "both", error: "both", text: "both", link: "both", clipboard: "pc", offer: "pc",
  complete: "phone", cancel: "both", unpair: "both", ping: "both",
  // changelog 56: the value is the SENDER's role
  "notif.status": "phone", "notif.icon": "phone", "notif.post": "phone", "notif.remove": "phone", "notif.sync": "phone", "notif.dismiss": "pc", "notif.action": "pc",
  // changelog 57: both sides send and receive every media.* type (each is the source of one direction and the viewer of the other); none is acked
  "media.status": "both", "media.state": "both", "media.image": "both", "media.clear": "both", "media.volume": "both", "media.cmd": "both",
  // changelog 58: presence goes both ways; ring.start and ring.stop are sent by the PC, ring.status by the phone
  presence: "both", "ring.start": "pc", "ring.stop": "pc", "ring.status": "phone",
  // changelog 59: an alert goes from the PC to the phone only
  alert: "pc",
};
// notif.action is the one notif message that is acked; the other six are ephemeral (no ack, no de-duplication, never stored).
// ring.start and ring.stop (changelog 58) are acked like notif.action; presence and ring.status are ephemeral.
const ACKED = new Set(["text", "link", "clipboard", "offer", "complete", "cancel", "unpair", "ping", "notif.action", "ring.start", "ring.stop", "alert"]);
const MAX_NOTIF_FRAME = 32768;
const MAX_NOTIF_ICON_FRAME = 49152; // a 32,768-byte PNG is 43,691 base64url characters, so notif.icon has a larger frame limit
const ACTION: Record<string, string> = {
  hello: "handshake_ok", ack: "mark_delivered", error: "mark_failed", text: "deliver_text", link: "deliver_link", clipboard: "set_clipboard",
  offer: "start_download", complete: "mark_transfer_done", cancel: "cancel_transfer", unpair: "remove_peer_and_close", ping: "show_ping_toast",
  "notif.status": "notif_status", "notif.icon": "notif_icon", "notif.post": "notif_post", "notif.remove": "notif_remove", "notif.sync": "notif_sync", "notif.dismiss": "notif_dismiss", "notif.action": "notif_action",
  "media.status": "media_status", "media.state": "media_state", "media.image": "media_image", "media.clear": "media_clear", "media.volume": "media_volume", "media.cmd": "media_cmd",
  presence: "presence", "ring.start": "ring_start", "ring.stop": "ring_stop", "ring.status": "ring_status",
  alert: "deliver_alert",
};
const MAX_PRESENCE_FRAME = 4096; // a presence or ring.status frame over this is error too_large (ring.status: the 2 KiB of every ring.* frame)
const MAX_RING_FRAME = 2048;
const MAX_MEDIA_FRAME = 32768; // a received media.state or any other media.* frame over this is error too_large
const MAX_MEDIA_IMAGE_FRAME = 49152; // a 24,576-byte image is 32,768 base64url characters
type Outcome = { reply: "ack" | "error" | "none"; action?: string; code?: string; ref?: string | null; close: number | null; sensitive?: boolean; link?: any; notif?: any; media?: any; presence?: any; alert?: any };
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
    // url is required and checked as sent; title and sourceApp are optional strings (null counts as absent)
    case "link": return id() && isLinkUrl(o.url) && (o.title === undefined || o.title === null || typeof o.title === "string") && (o.sourceApp === undefined || o.sourceApp === null || typeof o.sourceApp === "string");
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
    case "ring.start": case "ring.stop": return id();
    // changelog 59: text is a string that is not empty after cleaning; title and waitedMs are optional (null counts as absent)
    case "alert": return id() && isStr(o.text) && cleanLinkText(o.text, MAX_ALERT_TEXT) !== null && (o.title === undefined || o.title === null || isStr(o.title)) && (o.waitedMs === undefined || o.waitedMs === null || int("waitedMs"));
  }
  return false;
}
// what an alert's receiver ends up with (changelog 59): text and title cleaned like link.title, cut to 300 / 60 code points, an empty title is absent, waitedMs cut to 7 days
const MAX_ALERT_TEXT = 300;
const MAX_ALERT_TITLE = 60;
const MAX_ALERT_WAITED_MS = 604800000;
const MAX_ALERT_FRAME = 4096;
function alertOf(o: any): any {
  const out: any = {};
  const title = typeof o.title === "string" ? cleanLinkText(o.title, MAX_ALERT_TITLE) : null;
  if (title !== null) out.title = title;
  out.text = cleanLinkText(o.text, MAX_ALERT_TEXT);
  if (typeof o.waitedMs === "number") out.waitedMs = Math.min(o.waitedMs, MAX_ALERT_WAITED_MS);
  return out;
}

// ------------------------------------------------------------------ presence, pause and ring (changelog 58)
const PRESENCE_LIMITS = {
  frame: 4096, ringFrame: 2048, maxPauseMs: 691200000,
  batteryMinStep: 2, batteryCrossings: [15, 20, 100], batteryEveryMs: 30000, pauseTrailingMs: 1000,
  receiverCapacity: 10, receiverRefillMs: 3000, receiverErrorSpacingMs: 10000,
  ringTimeoutMs: 30000, ringStartsPerWindow: 3, ringWindowMs: 60000, ringCapMs: 120000, ringLockoutMs: 60000, pcRingEveryMs: 10000,
  tomorrowHour: 8, tomorrowMinAheadMs: 7200000,
  nudgeAt: 15, nudgeRearmAt: 20,
  shotStaleSec: 15, shotRateCapacity: 5, shotRateRefillMs: 20000, shotHoldMs: 5000,
};
// "bad" or the normalized content of one presence or ring.status frame that passed steps 1 to 6d
function presenceOf(type: string, o: any, badInts: Set<string>): any {
  const present = (v: any) => v !== undefined && v !== null;
  const bool = (v: any): boolean => {
    if (typeof v !== "boolean") throw BAD;
    return v;
  };
  try {
    if (!validMsgId(o.id)) throw BAD;
    if (type === "presence") {
      const out: any = {};
      if (present(o.battery)) {
        const b = o.battery;
        if (!isObj(b)) throw BAD;
        if (badInts.has("level") || typeof b.level !== "number" || !Number.isSafeInteger(b.level) || b.level < 0 || b.level > 100) throw BAD;
        const battery: any = { level: b.level, charging: bool(b.charging) };
        if (present(b.saver)) battery.saver = bool(b.saver);
        out.battery = battery;
      }
      if (present(o.pause)) {
        const p = o.pause;
        if (!isObj(p)) throw BAD;
        const hasMs = present(p.remainingMs);
        const hasUntil = present(p.untilResumed);
        if (hasMs === hasUntil) throw BAD; // both, or neither
        if (hasMs) {
          if (badInts.has("remainingMs") || typeof p.remainingMs !== "number" || !Number.isSafeInteger(p.remainingMs) || p.remainingMs < 1) throw BAD;
          out.pause = { remainingMs: Math.min(p.remainingMs, PRESENCE_LIMITS.maxPauseMs) };
        } else {
          if (p.untilResumed !== true) throw BAD;
          out.pause = { untilResumed: true };
        }
      }
      // changelog 59: dnd is a string (one of three words, any other string is unknown = absent); focus is a boolean (true counts, false is absent)
      if (present(o.dnd)) {
        if (typeof o.dnd !== "string") throw BAD;
        if (o.dnd === "priority" || o.dnd === "alarms" || o.dnd === "total") out.dnd = o.dnd;
      }
      if (present(o.focus)) {
        if (typeof o.focus !== "boolean") throw BAD;
        if (o.focus) out.focus = true;
      }
      return out;
    }
    // ring.status
    if (o.state !== "ringing" && o.state !== "stopped") throw BAD;
    if (present(o.reason) && typeof o.reason !== "string") throw BAD;
    const audible = present(o.audible) ? bool(o.audible) : true;
    if (o.state === "stopped") return typeof o.reason === "string" ? { state: "stopped", reason: o.reason } : { state: "stopped" };
    return { state: "ringing", audible };
  } catch (e) {
    if (e === BAD) return "bad";
    throw e;
  }
}

// ------------------------------------------------------------------ notifications (changelog 56)
const NID_RE = /^[0-9a-f]{32}$/;
const nidOfKey = (key: string) => createHash("sha256").update(key, "utf8").digest("hex").slice(0, 32);
const iconHashOf = (png: Uint8Array) => hex(sha256(png)).slice(0, 32);
const NOTIF_LIMITS = { app: 40, title: 150, text: 500, bigText: 1000, subText: 80, conversation: 100, messageSender: 60, messageText: 300, line: 150, hint: 60, label: 30, messages: 5, lines: 5, actions: 3, nids: 200, actionText: 2000, iconBytes: 32768, iconMin: 24, iconMax: 128 };
const BAD = Symbol("bad");
const isIdx = (v: any, badInts: Set<string>) => !badInts.has("i") && Number.isSafeInteger(v) && v >= 0 && v <= 31;
// the dimensions of a PNG (signature, then IHDR as the first chunk), or null
function pngSize(b: Uint8Array): { w: number; h: number } | null {
  const sig = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (b.length < 33 || !sig.every((x, i) => b[i] === x)) return null;
  if (String.fromCharCode(b[12], b[13], b[14], b[15]) !== "IHDR" || ((b[8] << 24) | (b[9] << 16) | (b[10] << 8) | b[11]) !== 13) return null;
  const u = (o: number) => ((b[o] << 24) | (b[o + 1] << 16) | (b[o + 2] << 8) | b[o + 3]) >>> 0;
  return { w: u(16), h: u(20) };
}
// "ok" / "bad" (bad_request) / "drop" (silently dropped and counted) and the normalized content, for one notif.* frame that passed steps 1 to 6b
function notifOf(type: string, o: any, badInts: Set<string>): any {
  const L = NOTIF_LIMITS;
  const optStr = (v: any, max: number): string | undefined => {
    if (v === undefined || v === null) return undefined;
    if (typeof v !== "string") throw BAD;
    return cleanLinkText(v, max) ?? undefined;
  };
  const optBool = (v: any): boolean => {
    if (v === undefined || v === null) return false;
    if (typeof v !== "boolean") throw BAD;
    return v;
  };
  const nid = (v: any) => {
    if (typeof v !== "string" || !NID_RE.test(v)) throw BAD;
    return v;
  };
  try {
    if (!validMsgId(o.id)) throw BAD;
    switch (type) {
      case "notif.status": {
        if (typeof o.enabled !== "boolean" || !["granted", "missing"].includes(o.access) || badInts.has("apps") || !Number.isSafeInteger(o.apps) || o.apps < 0) throw BAD;
        return { enabled: o.enabled, access: o.access, apps: o.apps, otpHidden: optBool(o.otpHidden) };
      }
      case "notif.icon": {
        if (typeof o.hash !== "string" || !NID_RE.test(o.hash) || typeof o.png !== "string") throw BAD;
        // a png that is not base64url, not a PNG, the wrong size or over 32,768 bytes, or a hash that doesn't match, is dropped silently
        if (!/^[A-Za-z0-9_-]*$/.test(o.png) || o.png.length % 4 === 1 || o.png.length > Math.ceil((L.iconBytes * 4) / 3) + 4) return "drop";
        const bytes = new Uint8Array(Buffer.from(o.png, "base64url"));
        if (bytes.length > L.iconBytes || iconHashOf(bytes) !== o.hash) return "drop";
        const dim = pngSize(bytes);
        if (!dim || dim.w !== dim.h || dim.w < L.iconMin || dim.w > L.iconMax) return "drop";
        return { hash: o.hash, bytes: bytes.length };
      }
      case "notif.remove":
      case "notif.dismiss":
        return { nid: nid(o.nid) };
      case "notif.sync": {
        if (!Array.isArray(o.nids) || o.nids.length > L.nids) throw BAD;
        return { nids: o.nids.map(nid) };
      }
      case "notif.action": {
        nid(o.nid);
        if (!isIdx(o.i, badInts) || typeof o.label !== "string") throw BAD;
        const out: any = { nid: o.nid, i: o.i, label: cleanLinkText(o.label, L.label) ?? "" };
        if (o.text !== undefined && o.text !== null) {
          if (typeof o.text !== "string") throw BAD;
          const t = trimWhiteSpace(o.text);
          if (cps(t).length < 1 || cps(t).length > L.actionText) throw BAD;
          out.text = t;
        }
        return out;
      }
      case "notif.post": {
        const app = optStr(nid(o.nid) && o.app, L.app);
        if (typeof o.app !== "string" || app === undefined) throw BAD;
        const post: any = { nid: o.nid, app };
        if (o.iconHash !== undefined && o.iconHash !== null) {
          if (typeof o.iconHash !== "string" || !NID_RE.test(o.iconHash)) throw BAD;
          post.iconHash = o.iconHash;
        }
        for (const [k, max] of [["title", L.title], ["text", L.text], ["bigText", L.bigText], ["subText", L.subText], ["conversation", L.conversation]] as const) {
          const v = optStr(o[k], max);
          if (v !== undefined) post[k] = v;
        }
        post.group = optBool(o.group);
        const arr = (v: any) => {
          if (v === undefined || v === null) return [];
          if (!Array.isArray(v)) throw BAD;
          return v;
        };
        const messages: any[] = [];
        for (const m of arr(o.messages)) {
          if (!isObj(m) || typeof m.text !== "string") throw BAD;
          const sender = optStr(m.sender, L.messageSender);
          const text = cleanLinkText(m.text, L.messageText);
          if (text !== null) messages.push({ ...(sender !== undefined ? { sender } : {}), text });
        }
        if (messages.length > 0) post.messages = messages.slice(-L.messages);
        const lines: string[] = [];
        for (const l of arr(o.lines)) {
          if (typeof l !== "string") throw BAD;
          const c = cleanLinkText(l, L.line);
          if (c !== null) lines.push(c);
        }
        if (lines.length > 0) post.lines = lines.slice(-L.lines);
        if (o.importance === undefined || o.importance === null) post.importance = 3;
        else if (badInts.has("importance") || !Number.isSafeInteger(o.importance) || o.importance < 2 || o.importance > 5) throw BAD;
        else post.importance = o.importance;
        post.silent = optBool(o.silent);
        post.sensitive = optBool(o.sensitive);
        post.redacted = optBool(o.redacted);
        if (validCode(o.code)) post.code = o.code;
        if (o.reply !== undefined && o.reply !== null) {
          if (!isObj(o.reply) || !isIdx(o.reply.i, badInts)) throw BAD;
          const hint = optStr(o.reply.hint, L.hint);
          post.reply = { i: o.reply.i, ...(hint !== undefined ? { hint } : {}) };
        }
        const actions: any[] = [];
        for (const a of arr(o.actions)) {
          if (!isObj(a) || !isIdx(a.i, badInts) || typeof a.label !== "string") throw BAD;
          const label = cleanLinkText(a.label, L.label);
          if (label !== null) actions.push({ i: a.i, label });
        }
        if (actions.length > 0) post.actions = actions.slice(0, L.actions);
        if (post.title === undefined && post.text === undefined && post.bigText === undefined && !post.messages && !post.lines && !post.redacted) throw BAD;
        return post;
      }
    }
  } catch (e) {
    if (e === BAD) return "bad";
    throw e;
  }
  return "bad";
}

// ------------------------------------------------------------------ media controls (changelog 57)
const MEDIA_LIMITS = {
  app: 40, title: 150, artist: 100, album: 100, actions: 8, durationMs: 86400000, positionMs: 86400000, rateMilli: 4000, seekMs: 86400000, level: 100, delta: 100,
  stateFrameSend: 16384, stateFrameRecv: 32768, imageFrame: 49152, imageBytes: 24576, artPixels: 256, iconPixels: 128, imageMinPixels: 16,
  pausedExpiryMs: 1800000, clearDebounceMs: 1500, positionJumpMs: 1500, endGraceMs: 5000, throttleMs: 250,
};
// the limiters: a token bucket (capacity, tokens per second) or a sliding window (max in windowMs); an error answer at most once per errorSpacingMs
const MEDIA_LIMITERS: Record<string, any> = {
  sourceState: { type: "bucket", capacity: 10, perSecond: 4 },
  viewerState: { type: "bucket", capacity: 20, perSecond: 5, errorSpacingMs: 10000 },
  sourceCmd: { type: "bucket", capacity: 20, perSecond: 2, errorSpacingMs: 10000 },
  viewerCmd: { type: "window", max: 20, windowMs: 10000 },
  image: { type: "window", max: 20, windowMs: 60000 },
};
const MEDIA_STATES = ["playing", "paused", "buffering", "stopped"];
const MEDIA_ACTIONS = ["play", "pause", "toggle", "next", "prev", "seek"]; // the canonical order of `actions`
const MEDIA_CMDS = ["play", "pause", "toggle", "next", "prev", "seek", "volume"];
const HEX32 = /^[0-9a-f]{32}$/;
// the first 32 hex characters of SHA-256 of the UTF-8 bytes of the app's identity (package name or SourceAppUserModelId)
const sidOf = (identity: string) => createHash("sha256").update(identity, "utf8").digest("hex").slice(0, 32);
// PNG or JPEG: the format and the size from the headers (a JPEG: the first SOF marker), or null. Nothing is decoded.
function mediaImageInfo(b: Uint8Array): { format: "png" | "jpeg"; w: number; h: number } | null {
  const png = pngSize(b);
  if (png) return { format: "png", ...png };
  if (b.length < 4 || b[0] !== 0xff || b[1] !== 0xd8) return null;
  let pos = 2;
  while (pos + 4 <= b.length) {
    if (b[pos] !== 0xff) return null;
    while (pos < b.length && b[pos] === 0xff) pos++; // fill bytes
    const marker = b[pos];
    pos++;
    if (marker === 0xd9 || marker === 0xda) return null; // end or scan before any frame header
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd8)) continue; // no length
    if (pos + 2 > b.length) return null;
    const len = (b[pos] << 8) | b[pos + 1];
    if (len < 2) return null;
    const sof = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
    if (sof) {
      if (pos + 7 > b.length) return null;
      return { format: "jpeg", h: (b[pos + 3] << 8) | b[pos + 4], w: (b[pos + 5] << 8) | b[pos + 6] };
    }
    pos += len;
  }
  return null;
}
// the wire form of the image's validity: bytes, format, and the size rule of its kind (null = fine)
function mediaImageProblem(bytes: Uint8Array, kind: string): string | null {
  if (bytes.length > MEDIA_LIMITS.imageBytes) return "too_big";
  const info = mediaImageInfo(bytes);
  if (!info) return "not_png_or_jpeg";
  if (info.w < MEDIA_LIMITS.imageMinPixels || info.h < MEDIA_LIMITS.imageMinPixels) return "too_small";
  if (Math.max(info.w, info.h) > (kind === "art" ? MEDIA_LIMITS.artPixels : MEDIA_LIMITS.iconPixels)) return "too_large_pixels";
  return null;
}
// "ok" content, "bad" (bad_request), or "drop" (silently dropped and counted), for one media.* frame that passed steps 1 to 6c
function mediaOf(type: string, o: any, badInts: Set<string>): any {
  const L = MEDIA_LIMITS;
  const optStr = (v: any, max: number): string | undefined => {
    if (v === undefined || v === null) return undefined;
    if (typeof v !== "string") throw BAD;
    return cleanLinkText(v, max) ?? undefined;
  };
  const int = (v: any, field: string, lo: number, hi: number): number => {
    if (badInts.has(field) || typeof v !== "number" || !Number.isSafeInteger(v) || v < lo || v > hi) throw BAD;
    return v;
  };
  const optInt = (v: any, field: string, lo: number, hi: number): number | undefined => (v === undefined || v === null ? undefined : int(v, field, lo, hi));
  const bool = (v: any): boolean => {
    if (typeof v !== "boolean") throw BAD;
    return v;
  };
  const hex32 = (v: any): string => {
    if (typeof v !== "string" || !HEX32.test(v)) throw BAD;
    return v;
  };
  try {
    if (!validMsgId(o.id)) throw BAD;
    switch (type) {
      case "media.status": {
        const out: any = { show: bool(o.show), share: bool(o.share) };
        if (o.access !== undefined && o.access !== null) {
          if (o.access !== "granted" && o.access !== "missing") throw BAD;
          out.access = o.access;
        }
        return out;
      }
      case "media.clear":
        return {};
      case "media.volume":
        return { level: int(o.level, "level", 0, L.level), muted: bool(o.muted) };
      case "media.state": {
        const out: any = { sid: hex32(o.sid) };
        if (typeof o.app !== "string") throw BAD;
        const app = cleanLinkText(o.app, L.app);
        if (app === null) throw BAD;
        out.app = app;
        if (typeof o.state !== "string" || !MEDIA_STATES.includes(o.state)) throw BAD;
        out.state = o.state;
        for (const [k, max] of [["title", L.title], ["artist", L.artist], ["album", L.album]] as const) {
          const v = optStr(o[k], max);
          if (v !== undefined) out[k] = v;
        }
        const duration = optInt(o.durationMs, "durationMs", 1, L.durationMs);
        if (duration !== undefined) out.durationMs = duration;
        let position = optInt(o.positionMs, "positionMs", 0, L.positionMs);
        if (position !== undefined) out.positionMs = duration !== undefined ? Math.min(position, duration) : position;
        const rate = o.rateMilli === undefined || o.rateMilli === null ? undefined : int(o.rateMilli, "rateMilli", 0, Number.MAX_SAFE_INTEGER);
        out.rateMilli = rate === undefined ? 1000 : Math.min(rate, L.rateMilli);
        // actions: only the first 8 entries are looked at, each a string; unknown ones are ignored; the result is in the canonical order; seek needs a duration
        let actions: string[] = [];
        if (o.actions !== undefined && o.actions !== null) {
          if (!Array.isArray(o.actions)) throw BAD;
          const first = o.actions.slice(0, L.actions);
          if (!first.every((a: any) => typeof a === "string")) throw BAD;
          actions = MEDIA_ACTIONS.filter((a) => first.includes(a));
        }
        if (duration === undefined) actions = actions.filter((a) => a !== "seek");
        out.actions = actions;
        if (o.artHash !== undefined && o.artHash !== null) out.artHash = hex32(o.artHash);
        if (o.iconHash !== undefined && o.iconHash !== null) out.iconHash = hex32(o.iconHash);
        if (o.volume !== undefined && o.volume !== null) {
          if (!isObj(o.volume)) throw BAD;
          out.volume = { level: int(o.volume.level, "level", 0, L.level), muted: bool(o.volume.muted) };
        }
        return out;
      }
      case "media.image": {
        const hash = hex32(o.hash);
        if (o.kind !== "art" && o.kind !== "icon") throw BAD;
        if (typeof o.data !== "string") throw BAD;
        // data that is not base64url, over 24,576 bytes, not a PNG or JPEG, the wrong size, or with a hash that doesn't match: dropped silently
        if (!/^[A-Za-z0-9_-]*$/.test(o.data) || o.data.length % 4 === 1 || o.data.length > Math.ceil((L.imageBytes * 4) / 3) + 4) return "drop";
        const bytes = new Uint8Array(Buffer.from(o.data, "base64url"));
        if (hex(sha256(bytes)).slice(0, 32) !== hash || mediaImageProblem(bytes, o.kind) !== null) return "drop";
        return { hash, kind: o.kind, bytes: bytes.length };
      }
      case "media.cmd": {
        if (typeof o.cmd !== "string" || !MEDIA_CMDS.includes(o.cmd)) throw BAD;
        const out: any = { cmd: o.cmd };
        const hasSid = o.sid !== undefined && o.sid !== null;
        if (hasSid) out.sid = hex32(o.sid);
        else if (o.cmd !== "volume") throw BAD;
        const present = (k: string) => o[k] !== undefined && o[k] !== null;
        if (o.cmd === "seek") {
          if (!present("seekMs")) throw BAD;
          out.seekMs = int(o.seekMs, "seekMs", 0, L.seekMs);
        } else if (present("seekMs")) throw BAD;
        if (o.cmd === "volume") {
          const given = ["level", "delta", "mute"].filter(present);
          if (given.length !== 1) throw BAD;
          if (given[0] === "level") out.level = int(o.level, "level", 0, L.level);
          if (given[0] === "delta") {
            out.delta = int(o.delta, "delta", -L.delta, L.delta);
            if (out.delta === 0) throw BAD;
          }
          if (given[0] === "mute") out.mute = bool(o.mute);
        } else if (present("level") || present("delta") || present("mute")) throw BAD;
        return out;
      }
    }
  } catch (e) {
    if (e === BAD) return "bad";
    throw e;
  }
  return "bad";
}

// ---- media: position (3.4), the active session (3.5), the maps (3.3, 5.2, 4.2), volume steps and the limiters
// what the viewer shows `elapsedMs` after it received a state: position, and the state it shows (a playing state past the end by more than 5 s is shown paused at the end; rate 0 is paused)
function mediaShown(c: { state: string; positionMs?: number; rateMilli?: number; durationMs?: number; elapsedMs: number }): { positionMs: number | null; state: string } {
  const rate = Math.min(c.rateMilli ?? 1000, MEDIA_LIMITS.rateMilli);
  if (c.positionMs === undefined) return { positionMs: null, state: c.state === "playing" && rate === 0 ? "paused" : c.state };
  const cap = (p: number) => (c.durationMs !== undefined ? Math.min(c.durationMs, p) : p);
  if (c.state !== "playing") return { positionMs: cap(c.positionMs), state: c.state };
  if (rate === 0) return { positionMs: cap(c.positionMs), state: "paused" };
  const raw = c.positionMs + Math.floor((c.elapsedMs * rate) / 1000);
  if (c.durationMs !== undefined && raw > c.durationMs + MEDIA_LIMITS.endGraceMs) return { positionMs: c.durationMs, state: "paused" };
  return { positionMs: cap(raw), state: "playing" };
}
// the source's own position for "now": Android (PlaybackState position and lastPositionUpdateTime, both elapsedRealtime ms; speed as a number) and Windows (timeline Position and LastUpdatedTime in 100 ns ticks)
function mediaSourcePosition(c: any): number | null {
  const clamp = (p: number) => Math.max(0, c.durationMs !== undefined && c.durationMs !== null ? Math.min(c.durationMs, p) : p);
  if (c.platform === "android") {
    if (c.positionMs < 0) return null;
    const speedMilli = Math.round(c.speed * 1000);
    const extra = c.state === "playing" ? Math.floor(((c.nowMs - c.updateTimeMs) * speedMilli) / 1000) : 0;
    return clamp(c.positionMs + Math.max(0, extra));
  }
  // ticks are decimal strings in the vectors: a FILETIME today is over 2^53
  const rate = BigInt(c.rateMilli ?? 1000);
  const base = BigInt(c.positionTicks) / 10000n;
  const last = BigInt(c.lastUpdatedTicks);
  const diff = BigInt(c.nowTicks) - last;
  const extra = c.state === "playing" && last > 0n && diff > 0n ? (diff * rate) / 10000000n : 0n;
  return clamp(Number(base + extra));
}
// 3.5: the session to show, or null (= clear). sessions are in the platform's priority order.
function mediaPick(sessions: { sid: string; own?: boolean; title: boolean; state: string; pausedMs?: number }[], current: string | null): string | null {
  const candidates = sessions.filter((s) => !s.own && s.title && ["playing", "paused", "buffering"].includes(s.state) && !(s.state === "paused" && (s.pausedMs ?? 0) >= MEDIA_LIMITS.pausedExpiryMs));
  const cur = candidates.find((s) => s.sid === current);
  if (cur && cur.state === "playing") return cur.sid;
  const playing = candidates.find((s) => s.state === "playing");
  if (playing) return playing.sid;
  if (cur) return cur.sid;
  return candidates[0]?.sid ?? null;
}
// the 1.5 s debounce of a clear: events in time order, present = a candidate exists after the event; returns the times at which a clear goes out
function mediaClearTimes(events: { t: number; present: boolean }[]): number[] {
  const out: number[] = [];
  let pending: number | null = null;
  let shown = true; // a session is on the viewer when the run starts
  for (const e of events) {
    if (pending !== null && e.t - pending >= MEDIA_LIMITS.clearDebounceMs) {
      out.push(pending + MEDIA_LIMITS.clearDebounceMs);
      pending = null;
      shown = false;
    }
    if (e.present) {
      pending = null;
      shown = true;
    } else if (shown && pending === null) pending = e.t;
  }
  return out;
}
// throttle with a trailing edge: the first event goes at once if the last send was 250 ms ago or more, else the latest value is held and goes at last + 250; a value equal to the last sent one is not sent
function mediaThrottle(events: { t: number; value: string }[]): { t: number; value: string }[] {
  const out: { t: number; value: string }[] = [];
  let lastT = -Infinity;
  let lastValue: string | null = null;
  let pending: string | null = null;
  const flushUntil = (t: number) => {
    if (pending !== null && lastT + MEDIA_LIMITS.throttleMs <= t) {
      const at = lastT + MEDIA_LIMITS.throttleMs;
      out.push({ t: at, value: pending });
      lastT = at;
      lastValue = pending;
      pending = null;
    }
  };
  for (const e of events) {
    flushUntil(e.t);
    if (e.value === lastValue) {
      pending = null;
      continue;
    }
    if (e.t - lastT >= MEDIA_LIMITS.throttleMs) {
      out.push({ t: e.t, value: e.value });
      lastT = e.t;
      lastValue = e.value;
      pending = null;
    } else pending = e.value;
  }
  flushUntil(Infinity);
  return out;
}
// the limiters of 3.6: outcomes per event, and the times an error answer goes out
function mediaLimiter(name: string, events: { t: number }[]): { outcomes: string[]; errorsAt: number[] } {
  const p = MEDIA_LIMITERS[name];
  const outcomes: string[] = [];
  const errorsAt: number[] = [];
  let lastError: number | null = null;
  const times: number[] = [];
  let tokens = p.type === "bucket" ? p.capacity * 1000 : 0;
  let last = 0;
  for (const e of events) {
    let ok: boolean;
    if (p.type === "bucket") {
      tokens = Math.min(p.capacity * 1000, tokens + (e.t - last) * p.perSecond);
      last = e.t;
      ok = tokens >= 1000;
      if (ok) tokens -= 1000;
    } else {
      while (times.length > 0 && e.t - times[0] >= p.windowMs) times.shift();
      ok = times.length < p.max;
      if (ok) times.push(e.t);
    }
    outcomes.push(ok ? "accepted" : "dropped");
    if (!ok && p.errorSpacingMs !== undefined && (lastError === null || e.t - lastError >= p.errorSpacingMs)) {
      errorsAt.push(e.t);
      lastError = e.t;
    }
  }
  return { outcomes, errorsAt };
}
const ANDROID_ACTION_BITS: Record<string, number> = { play: 4, pause: 2, toggle: 512, next: 32, prev: 16, seek: 256 };
const ANDROID_STATE: Record<number, string | null> = { 0: null, 1: null, 2: "paused", 3: "playing", 4: "playing", 5: "playing", 6: "buffering", 7: null, 8: "buffering", 9: "buffering", 10: "buffering", 11: "buffering" };
const ANDROID_STATE_OUT: Record<string, number> = { playing: 3, paused: 2, buffering: 6, stopped: 1 };
// PlaybackState (state int, actions mask, speed) -> the wire; `hasDuration`: seek is only offered with a duration
function androidToWire(c: { state: number; actions: number; speed: number; hasDuration: boolean }) {
  const state = ANDROID_STATE[c.state] ?? null;
  const actions = MEDIA_ACTIONS.filter((a) => (c.actions & ANDROID_ACTION_BITS[a]) !== 0 && (a !== "seek" || c.hasDuration));
  return { candidate: state !== null, ...(state !== null ? { state } : {}), actions, rateMilli: Math.min(Math.max(Math.round(c.speed * 1000), 0), MEDIA_LIMITS.rateMilli) };
}
const wireToAndroid = (c: { state: string; actions: string[] }) => ({ state: ANDROID_STATE_OUT[c.state], actions: c.actions.reduce((m, a) => m | (ANDROID_ACTION_BITS[a] ?? 0), 0) });
// GSMTC PlaybackStatus (Closed 0, Opened 1, Changing 2, Stopped 3, Playing 4, Paused 5) and Controls -> the wire
function windowsToWire(c: { status: number; controls: Record<string, boolean>; durationMs: number }) {
  const state = ({ 2: "buffering", 4: "playing", 5: "paused" } as Record<number, string>)[c.status] ?? null;
  const flags: Record<string, boolean> = { play: c.controls.play, pause: c.controls.pause, toggle: c.controls.toggle, next: c.controls.next, prev: c.controls.prev, seek: c.controls.position && c.durationMs > 0 };
  return { candidate: state !== null, ...(state !== null ? { state } : {}), actions: MEDIA_ACTIONS.filter((a) => flags[a] === true) };
}
// the wire -> SystemMediaTransportControls; a test run never shows Playing
function wireToSmtc(c: { state: string; actions: string[]; testMode: boolean }) {
  const status = c.state === "playing" ? (c.testMode ? "Paused" : "Playing") : c.state === "buffering" ? "Changing" : c.state === "paused" ? "Paused" : "Stopped";
  const has = (a: string) => c.actions.includes(a);
  return { status, playEnabled: has("play") || has("toggle"), pauseEnabled: has("pause") || has("toggle"), nextEnabled: has("next"), prevEnabled: has("prev") };
}
// native volume: Windows works on the scalar (0 to 1) of an endpoint with n steps, Android on an index 0..max
const volumeLevelOf = (c: any) => (c.platform === "windows" ? Math.round(c.scalar * 100) : Math.round((c.index * 100) / c.max));
function volumeSet(c: any): { level?: number; index?: number; muted?: boolean } {
  const op = c.op;
  if (op.mute !== undefined) return { muted: op.mute };
  if (c.platform === "windows") {
    const n = c.steps - 1;
    if (op.level !== undefined) return { level: op.level };
    const step = Math.round((c.level * n) / 100);
    const move = Math.max(1, Math.round((Math.abs(op.delta) * n) / 100));
    const next = Math.min(n, Math.max(0, step + Math.sign(op.delta) * move));
    return { level: Math.round((next * 100) / n) };
  }
  // Android: never index 0 by level or delta (mute is its own command)
  const n = c.max;
  const target = op.level !== undefined ? Math.round((op.level * n) / 100) : c.index + Math.sign(op.delta) * Math.max(1, Math.round((Math.abs(op.delta) * n) / 100));
  return { index: Math.min(n, Math.max(1, target)) };
}
// 3.1: may a source send media.state/image/clear/volume, or a viewer send media.cmd? `hellos`: the capability lists of the peer's accepted hellos, `status`: the peer's latest media.status
function mediaMaySend(c: { kind: string; ownShare: boolean; hellos: string[][]; status: { show?: boolean; share?: boolean } | null; shareJustTurnedOff?: boolean }): boolean {
  const listed = c.hellos.length > 0 && c.hellos[c.hellos.length - 1].includes("media");
  if (!listed) return false;
  if (c.kind === "cmd") return c.status?.share === true;
  if (c.kind === "status") return true;
  if (c.kind === "clear" && c.shareJustTurnedOff) return c.status?.show === true;
  return c.ownShare && c.status?.show === true;
}
// what a source answers to a media.cmd (3.6), in this order
function mediaCmdResult(c: { ownShare: boolean; rateOk: boolean; active: { sid: string; actions: string[] } | null; cmd: { sid?: string; cmd: string }; systemOk: boolean }): string {
  if (!c.ownShare) return "disabled";
  if (!c.rateOk) return "rate_limited";
  if (c.cmd.cmd !== "volume") {
    if (!c.active) return "not_found";
    const needs: Record<string, string[]> = { play: ["play", "toggle"], pause: ["pause", "toggle"], toggle: ["toggle", "play", "pause"], next: ["next"], prev: ["prev"], seek: ["seek"] };
    if (c.cmd.sid !== c.active.sid || !needs[c.cmd.cmd].some((a) => c.active!.actions.includes(a))) return "stale";
  }
  return c.systemOk ? "ok" : "action_failed";
}

// ---- rate limiters (integers only: millitokens, so there is no rounding to argue about)
function pcLimiter(events: { t: number; nid: string; content: string }[]) {
  const CAP = 20000, COST = 1000; // 1 token per 1000 ms = 1 millitoken per ms
  let tokens = CAP, last = 0;
  const lastAccepted = new Map<string, { t: number; content: string }>();
  let lastErrorAt: number | null = null;
  const outcomes: string[] = [];
  const errorsAt: number[] = [];
  for (const e of events) {
    tokens = Math.min(CAP, tokens + (e.t - last));
    last = e.t;
    const prev = lastAccepted.get(e.nid);
    if (prev && prev.content === e.content && e.t - prev.t < 2000) {
      outcomes.push("ignored_duplicate");
    } else if (tokens >= COST) {
      tokens -= COST;
      lastAccepted.set(e.nid, { t: e.t, content: e.content });
      outcomes.push("accepted");
    } else {
      outcomes.push("dropped_rate");
      if (lastErrorAt === null || e.t - lastErrorAt >= 10000) {
        errorsAt.push(e.t);
        lastErrorAt = e.t;
      }
    }
  }
  return { outcomes, errorsAt };
}

function phoneLimiter(events: { t: number; op: string; package?: string }[]) {
  const CAP = 10000, COST = 1000; // 1 token per 500 ms = 2 millitokens per ms
  let tokens = CAP, last = 0;
  const pkg = new Map<string, number[]>();
  const removes: number[] = [];
  const outcomes: string[] = [];
  for (const e of events) {
    tokens = Math.min(CAP, tokens + 2 * (e.t - last));
    last = e.t;
    if (e.op === "remove") {
      if (removes.filter((t) => t > e.t - 10000).length >= 50) outcomes.push("remove_dropped");
      else {
        removes.push(e.t);
        outcomes.push("remove_sent");
      }
      continue;
    }
    const times = pkg.get(e.package!) ?? [];
    if (times.filter((t) => t > e.t - 60000).length >= 20) outcomes.push("dropped_package");
    else if (tokens < COST) outcomes.push("dropped_budget");
    else {
      tokens -= COST;
      times.push(e.t);
      pkg.set(e.package!, times);
      outcomes.push("sent");
    }
  }
  return { outcomes };
}

// ---- the phone's per-nid sender
function coalesce(events: { t: number; op: string; nid: string; content?: string }[]) {
  type S = { lastSentAt: number | null; lastContent: string | null; pending: string | null };
  const st = new Map<string, S>();
  const sent: any[] = [];
  const get = (nid: string) => st.get(nid) ?? (st.set(nid, { lastSentAt: null, lastContent: null, pending: null }), st.get(nid)!);
  const fireDue = (upTo: number | null) => {
    for (;;) {
      let best: { nid: string; due: number } | null = null;
      for (const [nid, s] of st) {
        if (s.pending !== null && s.lastSentAt !== null) {
          const due = s.lastSentAt + 1000;
          if ((upTo === null || due <= upTo) && (best === null || due < best.due)) best = { nid, due };
        }
      }
      if (!best) return;
      const s = st.get(best.nid)!;
      const content = s.pending!;
      s.pending = null;
      if (content !== s.lastContent) {
        sent.push({ t: best.due, op: "post", nid: best.nid, content });
        s.lastSentAt = best.due;
        s.lastContent = content;
      }
    }
  };
  for (const e of events) {
    fireDue(e.t);
    const s = get(e.nid);
    if (e.op === "remove") {
      s.pending = null;
      sent.push({ t: e.t, op: "remove", nid: e.nid });
      s.lastSentAt = null;
      s.lastContent = null;
      continue;
    }
    if (s.lastSentAt !== null && e.t < s.lastSentAt + 1000) {
      s.pending = e.content!;
    } else if (e.content === s.lastContent) {
      // identical to what the PC already has: nothing goes out
    } else {
      sent.push({ t: e.t, op: "post", nid: e.nid, content: e.content });
      s.lastSentAt = e.t;
      s.lastContent = e.content!;
    }
  }
  fireDue(null);
  return { sent };
}

// ---- toast arguments
function parseToastArgs(s: string): { d: string; n: string; a: string; i?: number } | null {
  if (s.length > 200) return null;
  const parts = s.split("&");
  if (parts.length !== 4 && parts.length !== 5) return null;
  const keys = ["kin", "d", "n", "a", "i"];
  const v: string[] = [];
  for (let k = 0; k < parts.length; k++) {
    const at = parts[k].indexOf("=");
    if (at < 0 || parts[k].slice(0, at) !== keys[k]) return null;
    v.push(parts[k].slice(at + 1));
  }
  if (v[0] !== "1" || !/^[A-Z2-7]{8}$/.test(v[1]) || !/^[0-9a-f]{32}$/.test(v[2]) || !["tap", "reply", "copy", "btn"].includes(v[3])) return null;
  const needsIndex = v[3] === "reply" || v[3] === "btn";
  if (needsIndex !== (parts.length === 5)) return null;
  const out: any = { d: v[1], n: v[2], a: v[3] };
  if (needsIndex) {
    if (!/^(0|[1-9][0-9]?)$/.test(v[4]) || Number(v[4]) > 31) return null;
    out.i = Number(v[4]);
  }
  return out;
}
const validCode = (s: unknown) => typeof s === "string" && /^[0-9A-Z-]{4,10}$/.test(s);
const syncRemove = (shown: string[], nids: string[]) => shown.filter((n) => !nids.includes(n));
// A hint or a button label as it is on the toast (and so in notif.action.label): U+2028, U+2029, U+FFFE and U+FFFF become a space, the ends are trimmed.
const toastLabel = (s: string) => s.replace(/[\u2028\u2029\ufffe\uffff]/g, " ").replace(/^ +| +$/g, "");


class Receiver {
  state: "before_hello" | "open";
  // processed message IDs, per paired PC (`device|id`), as the database remembers them
  seen = new Set<string>();
  // offers by transfer ID, shared by all paired PCs: who made it, how its download stands, and its files (`fileId:size`, sorted)
  offers = new Map<string, { device: string; how: "unfinished" | "done" | "canceled" | "failed"; files: string }>();
  // the paired PC on this connection
  device = "first";
  // the capabilities of this side's own latest hello and of the peer's latest accepted hello (changelog 56: a notif.* message is dropped
  // silently unless both list "notif")
  ownCaps = new Set<string>();
  peerCaps = new Set<string>();
  // changelog 58: the phone's ring budget is spent (3 starts in the last 60 s, or inside the lock-out): a ring.start is error rate_limited
  ringBudgetSpent = false;
  // changelog 59: the phone's alert bucket is empty (10 per 60 s): an alert is error rate_limited
  alertBudgetSpent = false;
  constructor(public role: Role, state: "before_hello" | "open", caps?: { own?: string[]; peer?: string[]; ringBudgetSpent?: boolean; alertBudgetSpent?: boolean }) {
    this.state = state;
    this.ringBudgetSpent = caps?.ringBudgetSpent ?? false;
    this.alertBudgetSpent = caps?.alertBudgetSpent ?? false;
    this.ownCaps = new Set(caps?.own ?? []);
    this.peerCaps = new Set(caps?.peer ?? []);
  }
  // a new connection: the state starts again, the memory of IDs stays (it is stored in the database). `device`: the connection is from
  // another paired PC.
  reconnect(device?: string) {
    this.state = "before_hello";
    this.ownCaps = new Set();
    this.peerCaps = new Set();
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
        if (!(SIGNED_INT_FIELDS.has(f) ? /^-?(0|[1-9][0-9]*)$/ : /^(0|[1-9][0-9]*)$/).test(m[1])) badInts.add(f);
      }
    }
    // step 6: for hello, protocol is checked before every other field
    if (type === "hello" && !badInts.has("protocol") && typeof o.protocol === "number" && Number.isSafeInteger(o.protocol) && o.protocol >= 0 && o.protocol !== 1)
      return bad(ref, "unsupported_version", 4002);
    const isNotif = type.startsWith("notif.");
    const isMedia = type.startsWith("media.");
    const isPresence = type === "presence" || type === "ring.status";
    const isRingCmd = type === "ring.start" || type === "ring.stop";
    // step 6d (changelog 58): presence and ring.status are dropped silently unless this side's own latest hello and the sender's list "presence"; then the 4 KiB
    // frame rule (2 KiB for ring.status); then the fields. Neither is acked, de-duplicated or stored.
    if (isPresence) {
      if (!(this.ownCaps.has("presence") && this.peerCaps.has("presence"))) return { reply: "none", action: "presence_dropped", close: null };
      if (fb.bytes.length > (type === "presence" ? MAX_PRESENCE_FRAME : MAX_RING_FRAME)) return bad(ref, "too_large");
      const r = presenceOf(type, o, badInts);
      if (r === "bad") return bad(ref);
      return { reply: "none", action: ACTION[type], close: null, presence: r };
    }
    // ring.start and ring.stop are acked, so never dropped: the 2 KiB frame rule, the fields, the duplicate check, then the phone's own answers
    if (isRingCmd && fb.bytes.length > MAX_RING_FRAME) return bad(ref, "too_large");
    // an alert (changelog 59) is acked, so never dropped: the 4 KiB frame rule comes before the fields
    if (type === "alert" && fb.bytes.length > MAX_ALERT_FRAME) return bad(ref, "too_large");
    // step 7
    if (!isNotif && !isMedia && !validateFields(type, o, badInts)) return bad(ref);
    if (isRingCmd) {
      const seenKey = `${this.device}|${o.id}`;
      if (this.seen.has(seenKey)) return { reply: "ack", ref: o.id, action: "ignore_duplicate", close: null };
      // an error answer to a ring.start is not remembered; a ring.stop is always acked, also with the switch off
      if (type === "ring.start") {
        if (!this.ownCaps.has("ring")) return bad(ref, "disabled");
        if (this.ringBudgetSpent) return bad(ref, "rate_limited");
      }
      this.seen.add(seenKey);
      return { reply: "ack", ref: o.id, action: ACTION[type], close: null };
    }
    // step 6c (changelog 57): a media.* message is dropped silently unless this side's own latest hello and the sender's latest hello list "media"; then the 32 KiB
    // frame rule (48 KiB for a media.image); then the fields. None is acked, de-duplicated or stored.
    if (isMedia) {
      if (!(this.ownCaps.has("media") && this.peerCaps.has("media"))) return { reply: "none", action: "media_dropped", close: null };
      if (fb.bytes.length > (type === "media.image" ? MAX_MEDIA_IMAGE_FRAME : MAX_MEDIA_FRAME)) return bad(ref, "too_large");
      const r = mediaOf(type, o, badInts);
      if (r === "bad") return bad(ref);
      return r === "drop" ? { reply: "none", action: "media_dropped", close: null } : { reply: "none", action: ACTION[type], close: null, media: r };
    }
    // a valid offer with more than 800 files is rejected (an error, no ack, nothing remembered)
    if (type === "offer" && o.files.length > MAX_OFFER_FILES) return bad(ref, "too_large");
    // step 6b (changelog 56): a notif.* message is dropped silently unless this side's own latest hello and the sender's latest hello list "notif".
    // It is the receiver that keeps both lists; a case or sequence sets them (a hello of a sequence sets the sender's list on the other side).
    if (isNotif && !(this.ownCaps.has("notif") && this.peerCaps.has("notif"))) return { reply: "none", action: "notif_dropped", close: null };
    if (isNotif && fb.bytes.length > (type === "notif.icon" ? MAX_NOTIF_ICON_FRAME : MAX_NOTIF_FRAME)) return bad(ref, "too_large");
    // step 7
    if (isNotif) {
      const r = notifOf(type, o, badInts);
      if (r === "bad") return bad(ref);
      // ephemeral messages skip step 8; notif.action is acked and de-duplicated like the others
      if (type !== "notif.action") return r === "drop" ? { reply: "none", action: "notif_dropped", close: null } : { reply: "none", action: ACTION[type], close: null, notif: r };
      const seenKey = `${this.device}|${o.id}`;
      if (this.seen.has(seenKey)) return { reply: "ack", ref: o.id, action: "ignore_duplicate", close: null };
      this.seen.add(seenKey);
      return { reply: "ack", ref: o.id, action: ACTION[type], close: null, notif: r };
    }
    if (type === "hello") {
      this.state = "open";
      this.peerCaps = capabilitiesOf(o);
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
    // changelog 59: after the fields and the duplicate rule; an alert that is turned away is not remembered
    if (type === "alert" && this.alertBudgetSpent) return bad(ref, "rate_limited");
    this.seen.add(seenKey);
    return { reply: "ack", ref: o.id, action: ACTION[type], close: type === "unpair" ? 1000 : null, ...(type === "text" ? { sensitive: o.sensitive === true } : {}), ...(type === "link" ? { link: linkOf(o) } : {}), ...(type === "alert" ? { alert: alertOf(o) } : {}) };
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
  // a deliver_link case says what the receiver shows and opens: the url as sent, the cleaned title and sourceApp
  if (actual.action === "deliver_link" && expect.link !== undefined && !eq(actual.link, expect.link)) return false;
  // a notif.* case says what the receiver got after cleaning (expect.notif)
  if (expect.notif !== undefined && !eq(actual.notif, expect.notif)) return false;
  // a media.* case says what the receiver got after cleaning (expect.media)
  if (expect.media !== undefined && !eq(actual.media, expect.media)) return false;
  // a presence or ring.status case says what the receiver got after validation (expect.presence)
  if (expect.presence !== undefined && !eq(actual.presence, expect.presence)) return false;
  // an alert case says what the receiver got after cleaning (expect.alert)
  if (expect.alert !== undefined && !eq(actual.alert, expect.alert)) return false;
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
    const out = new Receiver(c.to, c.state, { own: c.own, peer: c.peer, ringBudgetSpent: c.ringBudgetSpent, alertBudgetSpent: c.alertBudgetSpent }).receive(c.from, c);
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
    const out = new Receiver(c.to, c.state, { own: c.own, peer: c.peer, ringBudgetSpent: c.ringBudgetSpent, alertBudgetSpent: c.alertBudgetSpent }).receive(c.from, c);
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
      // changelog 59: a step may say whether the receiver's alert bucket is empty at that moment
      if (st.alertBudgetSpent !== undefined) rx[to].alertBudgetSpent = st.alertBudgetSpent;
      // each role has its own receiver state, and the first message a role receives is its peer's hello
      const out = rx[to].receive(st.from, st);
      // a hello that is accepted also sets the capabilities of its own sender's side (what that side lists when it is the receiver)
      if (st.frame?.type === "hello" && out.action === "handshake_ok") rx[st.from as Role].ownCaps = capabilitiesOf(st.frame);
      check(g, `sequence ${s.name} step ${i + 1}`, sameOutcome(out, st.expect), () => `expected ${j(st.expect)}\n       got      ${j(out)}`);
    });
  }
  // every message type has at least one valid example, and every type that is acked is in the acked list
  const types = new Set(d.valid.map((c: any) => c.frame?.type));
  for (const t of Object.keys(DIRECTION)) check(g, `has a valid example of ${t}`, types.has(t));
  for (const t of ACKED) {
    const ok = d.valid.filter((c: any) => c.frame?.type === t).every((c: any) => c.expect.reply === "ack" || c.expect.action === "notif_dropped");
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

// ---- milestone 1.1 scope: every name listed in protocol/README.md ("What each milestone needs")
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

// ---- milestone 1.4 scope (queue, and the rule changes of 1.3). Same rule as for the other milestones: every name in
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

// ---- links.json (changelog 54) and the 2B.1 scope: the link-* cases of messages.json
{
  const g = "links";
  const d = load("links.json");
  const synth = (c: any) => c.synthesize.prefix + c.synthesize.fill.repeat(c.synthesize.fillCount) + (c.synthesize.suffix ?? "");
  check(g, "limits", d.maxUrlBytes === MAX_URL_BYTES && d.maxTitleCodePoints === MAX_TITLE && d.maxSourceAppCodePoints === MAX_SOURCE_APP);
  for (const k of ["urls", "titles", "sourceApps", "sendAs", "textFallback"]) uniqueNames(g + "." + k, d[k]);
  for (const c of d.urls) {
    const text: string = c.synthesize ? synth(c) : c.input;
    if (c.synthesize) check(g, `url ${c.name}: synthesized size matches bytes`, utf8Len(text) === c.synthesize.bytes, () => `${utf8Len(text)} vs ${c.synthesize.bytes}`);
    check(g, `url ${c.name}`, isLinkUrl(text) === c.valid, () => `input ${j(text.slice(0, 100))} expected ${c.valid}, model says ${isLinkUrl(text)}`);
    // the same link as a link in a frame goes through the receiver, and an unchanged URL comes out as the same string
    const frame = { type: "link", id: "u1", url: text };
    const out = new Receiver("phone", "open").receive("pc", { frame });
    check(g, `url ${c.name}: the receiver agrees`, c.valid ? out.reply === "ack" && out.link?.url === text : out.reply === "error" && out.code === "bad_request" && out.ref === "u1");
    if (c.unchanged) check(g, `url ${c.name}: delivered unchanged`, c.valid && out.link?.url === c.input);
  }
  check(g, "urls has accepted and rejected cases", d.urls.some((c: any) => c.valid) && d.urls.some((c: any) => !c.valid));
  // a link.url that is valid is also an openable URL as a text would see it; the reverse needs the url to be trimmed first
  for (const c of d.urls.filter((c: any) => c.valid && c.input !== undefined)) check(g, `url ${c.name}: valid as a link means valid as a text`, isOpenableUrl(c.input));
  // display[] (changelog 55): the address a receiver shows and whether it is marked
  uniqueNames(g + ".display", d.display);
  for (const c of d.display) {
    check(g, `display ${c.name}: the URL is valid as a link`, isLinkUrl(c.url));
    const got = displayAddress(c.url);
    check(g, `display ${c.name}`, got.display === c.display && got.warn === c.warn, () => `expected ${j(c.display)} warn ${c.warn}, model says ${j(got.display)} warn ${got.warn}`);
    // apart from a host a browser refuses, the shown address is what a browser reports for the whole URL
    if (c.parseFails) {
      check(g, `display ${c.name}: a browser refuses the URL`, (() => { try { new URL(c.url); return false; } catch { return true; } })());
      check(g, `display ${c.name}: refused hosts warn`, c.warn === true);
    } else {
      check(g, `display ${c.name}: equals the browser's host`, new URL(c.url).host === c.display, () => `${new URL(c.url).host}`);
    }
    // a plain ASCII host that isn't an IP address and has no punycode label never warns
    if (!c.warn) check(g, `display ${c.name}: no warning means plain ASCII`, /^[a-z0-9.:_-]+$/.test(c.display) && !c.display.split(".").some((l: string) => l.startsWith("xn--")));
  }
  check(g, "display has warned and plain cases", d.display.some((c: any) => c.warn) && d.display.some((c: any) => !c.warn) && d.display.length >= 40);
  for (const c of d.urls.filter((c: any) => c.valid && c.input !== undefined)) check(g, `url ${c.name}: the address can be shown`, typeof displayAddress(c.input).display === "string");
  for (const [key, max] of [["titles", MAX_TITLE], ["sourceApps", MAX_SOURCE_APP]] as const) {
    for (const c of d[key]) {
      const got = cleanLinkText(c.input, max);
      check(g, `${key} ${c.name}`, got === c.expected, () => `expected ${j(c.expected)}, model says ${j(got)}`);
      // cleaning is idempotent and never leaves a bidi or control character
      if (got !== null) check(g, `${key} ${c.name}: result is stable and clean`, cleanLinkText(got, max) === got && !Array.from(got).some((ch) => isControlCp(ch.codePointAt(0)!) || isBidiControlCp(ch.codePointAt(0)!)));
    }
  }
  for (const c of d.sendAs) {
    const hellos = c.hellos.map((h: any) => ({ type: "hello", id: "h", protocol: 1, name: "X", platform: "windows", appVersion: "0.2.0", ...h }));
    for (const h of hellos) check(g, `sendAs ${c.name}: the hello is accepted`, new Receiver("phone", "before_hello").receive("pc", { frame: h }).action === "handshake_ok");
    check(g, `sendAs ${c.name}`, sendLinkAs(hellos) === c.expected, () => `expected ${c.expected}, model says ${sendLinkAs(hellos)}`);
  }
  for (const c of d.textFallback) {
    check(g, `textFallback ${c.name}: the link is valid`, isLinkUrl(c.link.url));
    check(g, `textFallback ${c.name}: the text frame is a valid text with isUrl true`, c.expected.isUrl === true && c.expected.text === c.link.url && isOpenableUrl(c.expected.text));
    const frame = { type: "text", id: "t1", ...c.expected };
    check(g, `textFallback ${c.name}: an old peer delivers it as a text`, new Receiver("pc", "open").receive("phone", { frame }).action === "deliver_text");
  }
}

{
  const g = "scope-2b.1";
  const messages = load("messages.json");
  const matches = (name: string, p: string) => (p.endsWith("*") ? name.startsWith(p.slice(0, -1)) : name === p);
  const SCOPE: [string, any[], string[], number][] = [
    ["messages.valid", messages.valid, ["link-*"], 18],
    ["messages.invalid", messages.invalid, ["link-*"], 28],
    ["messages.sequences", messages.sequences, ["link-*"], 4],
  ];
  let inScope = 0;
  for (const [label, cases, patterns, atLeast] of SCOPE) {
    for (const p of patterns) check(g, `${label}: "${p}" matches a case`, cases.some((c) => matches(c.name, p)));
    const hits = cases.filter((c) => patterns.some((p) => matches(c.name, p)));
    inScope += hits.length;
    check(g, `${label}: at least ${atLeast} cases in scope`, hits.length >= atLeast, () => String(hits.length));
  }
  // every link case that is valid is acked as deliver_link and carries the expected link; every invalid one is bad_request with the id as ref
  for (const c of messages.valid.filter((c: any) => c.name.startsWith("link-"))) check(g, `${c.name}: delivers a link`, c.expect.action === "deliver_link" && typeof c.expect.link?.url === "string");
  for (const c of messages.invalid.filter((c: any) => c.name.startsWith("link-"))) check(g, `${c.name}: bad_request, connection stays open (unless before hello)`, c.expect.code === "bad_request" && (c.state === "before_hello" ? c.expect.close === 1002 : c.expect.close === null));
  // both directions are exercised
  check(g, "links go both ways", messages.valid.some((c: any) => c.name.startsWith("link-") && c.from === "pc") && messages.valid.some((c: any) => c.name.startsWith("link-") && c.from === "phone"));
  console.log(`milestone 2B.1 scope: ${inScope} link cases in messages.json, plus links.json`);
}


// ---- notif.json (changelog 56)
{
  const g = "notif";
  const d = load("notif.json");
  for (const k of ["nid", "iconHash", "icon", "clean", "post", "code", "toastIds", "toastArguments", "rate", "coalesce", "sendNotif", "syncDiff", "toastLabel"]) uniqueNames(g + "." + k, d[k]);
  check(g, "limits", eq(d.limits, { ...NOTIF_LIMITS, frameBytes: MAX_NOTIF_FRAME, iconFrameBytes: MAX_NOTIF_ICON_FRAME, codeMin: 4, codeMax: 10 }), () => j(d.limits));
  // nid: the first 32 hex characters of SHA-256 of the UTF-8 key
  for (const c of d.nid) check(g, `nid ${c.name}`, nidOfKey(c.key) === c.nid && NID_RE.test(c.nid), () => nidOfKey(c.key));
  check(g, "the nid examples of the design", d.nid.some((c: any) => c.key === "0|com.example.chat|42|null|10123" && c.nid === "54892d1f759b11f6762ff1bd98cda7df") && d.nid.some((c: any) => c.key === "0|com.whatsapp|1|null|10123" && c.nid === "36460390475e793c9a9e06dd5e2d421f"));
  check(g, "nid has non-ASCII, empty-tag and long keys", d.nid.some((c: any) => Array.from(c.key as string).some((ch) => ch.codePointAt(0)! > 127)) && d.nid.some((c: any) => c.key.includes("||")) && d.nid.some((c: any) => c.key.length >= 1000));
  for (const c of d.iconHash) {
    const bytes = new Uint8Array(Buffer.from(c.png, "base64url"));
    check(g, `iconHash ${c.name}`, iconHashOf(bytes) === c.hash && bytes.length === c.bytes && base64url(bytes) === c.png, () => iconHashOf(bytes));
  }
  // icon[]: a notif.icon frame through the receiver
  for (const c of d.icon) {
    const png = c.pngFile !== undefined ? base64url(new Uint8Array(readFileSync(join(dir, c.pngFile)))) : c.png;
    const frame: any = { type: "notif.icon", id: "icon-case" };
    if (c.hash !== undefined) frame.hash = c.hash;
    if (png !== undefined) frame.png = png;
    const out = new Receiver("pc", "open", { own: ["notif"], peer: ["notif"] }).receive("phone", { frame });
    const got = out.reply === "error" ? (out.code === "bad_request" ? "bad_request" : out.code) : out.action === "notif_icon" ? "accepted" : out.action === "notif_dropped" ? "dropped" : "?";
    check(g, `icon ${c.name}`, got === c.expect && out.reply !== "ack", () => `expected ${c.expect}, model says ${got}`);
  }
  check(g, "icon fixtures are exactly 32,768 and 32,769 bytes", readFileSync(join(dir, "fixtures/notif-icon-32768.png")).length === 32768 && readFileSync(join(dir, "fixtures/notif-icon-32769.png")).length === 32769);
  const innerLimit: Record<string, number> = { app: 40, title: 150, text: 500, bigText: 1000, subText: 80, conversation: 100, messageSender: 60, messageText: 300, line: 150, hint: 60, label: 30 };
  for (const c of d.clean) {
    const max = innerLimit[c.field];
    check(g, `clean ${c.name}: a known field`, max !== undefined && max === d.limits[c.field]);
    const got = cleanLinkText(c.input, max);
    check(g, `clean ${c.name}`, got === c.expected, () => `expected ${j(c.expected)}, model says ${j(got)}`);
    if (got !== null) check(g, `clean ${c.name}: stable and within the limit`, cleanLinkText(got, max) === got && cps(got).length <= max);
  }
  for (const c of d.post) {
    const out = new Receiver(c.to, c.state, { own: c.own, peer: c.peer }).receive(c.from, c);
    check(g, `post ${c.name}`, sameOutcome(out, c.expect), () => `expected ${j(c.expect)}\n       got      ${j(out)}`);
    if (c.synthesize) check(g, `post ${c.name}: synthesized size matches totalBytes`, frameBytes(c).bytes.length === c.synthesize.totalBytes, () => `${frameBytes(c).bytes.length} vs ${c.synthesize.totalBytes}`);
  }
  // a post that is accepted never comes out with more than the limits allow
  for (const c of d.post.filter((c: any) => c.expect.notif)) {
    const n = c.expect.notif;
    check(g, `post ${c.name}: the expected post is within the limits`, cps(n.app).length <= 40 && (n.title === undefined || cps(n.title).length <= 150) && (n.messages ?? []).length <= 5 && (n.lines ?? []).length <= 5 && (n.actions ?? []).length <= 3);
  }
  for (const c of d.code) check(g, `code ${c.name}`, validCode(c.input) === c.valid, () => j(c.input));
  for (const c of d.toastIds) {
    check(g, `toastIds ${c.name}: tag is the nid, group is kin- and the short ID`, c.tag === c.nid && c.group === `kin-${c.shortId}` && /^[A-Z2-7]{8}$/.test(c.shortId));
    const expectArgs = (a: string, i?: number) => `kin=1&d=${c.shortId}&n=${c.nid}&a=${a}` + (i !== undefined ? `&i=${i}` : "");
    check(g, `toastIds ${c.name}: launch and arguments`, c.launch === expectArgs("tap") && c.copy === expectArgs("copy") && c.reply.arguments === expectArgs("reply", c.reply.i) && c.button.arguments === expectArgs("btn", c.button.i));
    for (const a of [c.launch, c.copy, c.reply.arguments, c.button.arguments]) check(g, `toastIds ${c.name}: ${a.slice(-12)} parses`, parseToastArgs(a)?.n === c.nid && parseToastArgs(a)?.d === c.shortId);
    check(g, `toastIds ${c.name}: the arguments hold no content`, [c.launch, c.copy, c.reply.arguments, c.button.arguments].every((a) => a.length <= 200 && /^[A-Za-z0-9=&-]+$/.test(a)));
  }
  for (const c of d.toastArguments) {
    const got = parseToastArgs(c.input);
    check(g, `toastArguments ${c.name}`, c.valid ? eq(got, c.parsed) : got === null, () => `model says ${j(got)}`);
  }
  check(g, "toastArguments has accepted and refused cases", d.toastArguments.some((c: any) => c.valid) && d.toastArguments.filter((c: any) => !c.valid).length >= 30);
  for (const c of d.rate) {
    check(g, `rate ${c.name}: events are in time order`, c.events.every((e: any, k: number) => k === 0 || c.events[k - 1].t <= e.t));
    if (c.limiter === "pc") {
      const r = pcLimiter(c.events);
      check(g, `rate ${c.name}`, eq(r.outcomes, c.expect) && eq(r.errorsAt, c.errorsAt), () => `errors ${j(r.errorsAt)} expected ${j(c.errorsAt)}`);
      // the bucket never gives more than its capacity plus the refill, and errors are at least 10 s apart
      const accepted = r.outcomes.filter((o) => o === "accepted").length;
      const span = c.events.length ? c.events[c.events.length - 1].t : 0;
      check(g, `rate ${c.name}: accepted <= 20 + span in seconds`, accepted <= 20 + Math.floor(span / 1000));
      check(g, `rate ${c.name}: errors are at least 10 s apart`, r.errorsAt.every((t: number, k: number) => k === 0 || t - r.errorsAt[k - 1] >= 10000));
    } else {
      check(g, `rate ${c.name}`, eq(phoneLimiter(c.events).outcomes, c.expect));
    }
    check(g, `rate ${c.name}: one outcome per event`, c.expect.length === c.events.length);
  }
  for (const c of d.coalesce) {
    const r = coalesce(c.events);
    check(g, `coalesce ${c.name}`, eq(r.sent, c.expect.sent), () => `model says ${j(r.sent)}`);
    // a post is never sent twice in a row with the same content, and never twice within 1 s of the last for the same nid
    const lastPost = new Map<string, { t: number; content: string }>();
    for (const x of r.sent) {
      if (x.op === "remove") {
        lastPost.delete(x.nid);
        continue;
      }
      const prev = lastPost.get(x.nid);
      check(g, `coalesce ${c.name}: ${x.t} ms is not a repeat or inside the window`, !prev || (prev.content !== x.content && x.t - prev.t >= 1000));
      lastPost.set(x.nid, { t: x.t, content: x.content });
    }
  }
  for (const c of d.sendNotif) {
    const hellos = c.hellos.map((h: any) => ({ type: "hello", id: "h", protocol: 1, name: "X", platform: "android", appVersion: "0.2.0", ...h }));
    const mayRelay = hellos.length > 0 && capabilitiesOf(hellos[hellos.length - 1]).has("notif");
    check(g, `sendNotif ${c.name}`, (mayRelay ? "send" : "nothing") === c.expected);
  }
  for (const c of d.syncDiff) check(g, `syncDiff ${c.name}`, eq(syncRemove(c.shown, c.nids), c.remove));
  for (const c of d.toastLabel) {
    check(g, `toastLabel ${c.name}`, toastLabel(c.input) === c.expected, () => `model says ${j(toastLabel(c.input))}`);
    check(g, `toastLabel ${c.name} is idempotent`, toastLabel(toastLabel(c.input)) === toastLabel(c.input));
  }
}

// ---- notif.json, the arrays that only the phone runs (design 3.11): otpDetect[], redactedText[], skip[], pickActions[]. Their reference models are written from design
// 4.4, 4.7, 4.9 and 4.10; where the design is ambiguous the reading is the one the Android code settled on (see protocol/README.md, "notif.json").

// ---- one-time code detector (design 4.10)
const OTP_KEYWORDS: string[] = [
  // English
  "code", "otp", "passcode", "pass code", "verification", "verify", "one-time", "one time", "security code", "login code", "sign-in code", "sign in code", "2fa", "two-factor", "two factor", "pin", "confirmation code",
  // Spanish / Portuguese
  "código", "codigo", "verificación", "verificacion", "verificação", "clave", "contraseña de un solo uso", "senha", "confirmación", "confirmação",
  // French
  "code", "vérification", "verification", "confirmation", "mot de passe à usage unique",
  // German
  "code", "bestätigungscode", "verifizierung", "sicherheitscode", "einmalpasswort", "tan",
  // Italian / Dutch
  "codice", "verifica", "conferma", "password monouso", "verificatie", "verificatiecode", "bevestigingscode",
  // Nordic / Polish
  "kod", "kode", "bekräftelsekod", "verifieringskod", "engangskode", "weryfikacyjny", "jednorazowy",
  // Turkish / Russian / Ukrainian
  "kod", "doğrulama", "onay kodu", "tek kullanımlık", "код", "подтверждения", "проверочный", "одноразовый", "підтвердження",
  // Arabic / Persian / Hindi
  "رمز", "رمز التحقق", "كلمة مرور لمرة واحدة", "کد", "کد تایید", "कोड", "ओटीपी", "सत्यापन",
  // Indonesian / Vietnamese / Thai
  "kode", "verifikasi", "sandi sekali pakai", "mã", "xác minh", "mã xác thực", "รหัส", "ยืนยัน",
  // Japanese / Korean
  "認証コード", "確認コード", "認証番号", "確認番号", "ワンタイム", "コード", "인증번호", "인증 코드", "확인 코드", "일회용",
  // Chinese
  "验证码", "校验码", "动态码", "确认码", "驗證碼", "校驗碼", "動態密碼",
];
const OTP_WHOLE_WORD = new Set(["otp", "2fa", "pin"]);
const OTP_PROMO = ["save", "off", "discount", "promo", "coupon", "sale"];
const UNSPACED = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}\p{Script=Thai}]/u;
const isWordCh = (c: string) => /[\p{L}\p{N}\p{M}]/u.test(c);
function foldDigitsAscii(s: string): string {
  let out = "";
  for (const ch of s) {
    const cp = ch.codePointAt(0)!;
    if (cp > 0x7f && /\p{Nd}/u.test(ch)) {
      let run = cp;
      while (/\p{Nd}/u.test(String.fromCodePoint(run - 1))) run--;
      out += String((cp - run) % 10);
    } else out += ch;
  }
  return out;
}
// lower-case one character at a time (Locale.ROOT): a character whose lower case has another length keeps its place
function lowerPerChar(s: string): string {
  let out = "";
  for (const ch of s) {
    const l = ch.toLowerCase();
    out += l.length === ch.length ? l : ch;
  }
  return out;
}
function otpDetect(title: string | undefined, text: string | undefined, bigText: string | undefined, hasContact: boolean): string | null {
  if (hasContact) return null;
  const joined = [title, text, bigText].filter((x): x is string => typeof x === "string" && x.length > 0).join("\n");
  if (joined.length === 0 || joined.length > 400) return null;
  const folded = foldDigitsAscii(joined);
  const lower = lowerPerChar(folded);
  // gate 1: a keyword (the CJK, Thai and Hangul ones anywhere, the rest on word boundaries; otp, 2fa and pin always whole words)
  const spans: [number, number][] = [];
  for (const kw0 of OTP_KEYWORDS) {
    const kw = lowerPerChar(kw0);
    const substring = !OTP_WHOLE_WORD.has(kw) && UNSPACED.test(kw);
    for (let at = lower.indexOf(kw); at >= 0; at = lower.indexOf(kw, at + 1)) {
      const end = at + kw.length;
      if (substring || ((at === 0 || !isWordCh(lower[at - 1])) && (end === lower.length || !isWordCh(lower[end])))) spans.push([at, end - 1]);
    }
  }
  if (spans.length === 0) return null;
  type Cand = { value: string; start: number; end: number; year: boolean };
  const isDigit = (c: string) => c >= "0" && c <= "9";
  const touches = (c: string) => (isDigit(c) ? true : /\p{L}/u.test(c) ? !UNSPACED.test(c) : false);
  const UNIT_AFTER = /^ ?(?:usd|eur|gbp|jpy|cny|inr|rub|krw|chf|cad|aud|mxn|brl|pln|sek|nok|dkk|km|kg|min|mins|h|hr|hrs|m|cm|mm|mi|lb|lbs|mph|kph)(?![\p{L}\p{N}])/iu;
  const standsAlone = (start: number, end: number): boolean => {
    if (start > 0) {
      const b = folded[start - 1];
      if (touches(b) || "$€£¥₹₽₩+(".includes(b)) return false;
      if (".,:/-".includes(b) && start >= 2 && isDigit(folded[start - 2])) return false;
    }
    if (end < folded.length) {
      const a = folded[end];
      if (touches(a) || a === "%") return false;
      if (".,:/".includes(a) && end + 1 < folded.length && isDigit(folded[end + 1])) return false;
    }
    return !UNIT_AFTER.test(folded.slice(end, Math.min(folded.length, end + 8)));
  };
  const cands: Cand[] = [];
  const add = (value: string, start: number, end: number) => {
    if (standsAlone(start, end)) cands.push({ value, start, end, year: value.length === 4 && Number(value) >= 1900 && Number(value) <= 2099 });
  };
  for (const m of folded.matchAll(/[0-9]+(?:[ \-][0-9]+)*/g)) {
    const groups: [string, number, number][] = [];
    let at = m.index!;
    for (const part of m[0].split(/[ \-]/)) {
      groups.push([part, at, at + part.length]);
      at += part.length + 1;
    }
    if (groups.length >= 3) continue; // "1234 5678 9012", a date, a phone number
    if (groups.length === 2 && ((groups[0][0].length === 3 && groups[1][0].length === 3) || (groups[0][0].length === 4 && groups[1][0].length === 4))) add(groups[0][0] + groups[1][0], groups[0][1], groups[1][2]);
    else for (const [d, s, e] of groups) if (d.length >= 4 && d.length <= 8) add(d, s, e);
  }
  const insideUrl = (idx: number) => {
    let a = idx;
    while (a > 0 && !/\s/.test(folded[a - 1])) a--;
    let b = idx;
    while (b < folded.length && !/\s/.test(folded[b])) b++;
    const w = folded.slice(a, b).toLowerCase();
    return w.includes("://") || w.startsWith("www.");
  };
  for (const m of folded.matchAll(/(?<![A-Za-z0-9])[A-Z0-9]{5,8}(?![A-Za-z0-9])/g)) {
    const tok = m[0], start = m.index!, end = start + tok.length;
    if (/[A-Z]/.test(tok) && /[0-9]/.test(tok) && !insideUrl(start) && (start === 0 || !touches(folded[start - 1])) && (end === folded.length || !touches(folded[end]))) cands.push({ value: tok, start, end, year: false });
  }
  const promoRe = new RegExp(`(?<![\\p{L}\\p{N}])(?:${OTP_PROMO.join("|")})(?![\\p{L}\\p{N}])`, "u");
  const isPromo = (c: Cand) => {
    if (OTP_PROMO.some((w) => lower.slice(c.start, c.end).includes(w))) return true;
    return promoRe.test(lower.slice(Math.max(0, c.start - 30), c.start)) || promoRe.test(lower.slice(c.end, Math.min(lower.length, c.end + 30)));
  };
  const live = cands.filter((c) => !isPromo(c));
  if (live.length === 0) return null;
  const usable = live.filter((c) => !c.year).length > 0 ? live.filter((c) => !c.year) : live; // a year is a code only when nothing else is there
  const dist = (c: Cand) => Math.min(...spans.map(([a, b]) => (b < c.start ? c.start - b - 1 : a >= c.end ? a - c.end : 0)));
  const byValue = new Map<string, number>();
  for (const c of usable) byValue.set(c.value, Math.min(byValue.get(c.value) ?? Infinity, dist(c)));
  if (byValue.size === 1) return [...byValue.keys()][0];
  const ranked = [...byValue.entries()].sort((a, b) => a[1] - b[1]);
  // "at least 10 characters closer" is read as MORE than 10: the design's own example, "Verification code 123456 or 654321", must give no code
  return ranked[1][1] - ranked[0][1] > 10 ? ranked[0][0] : null;
}

// ---- Android 15+ redaction (design 4.9)
const redactedMatch = (sdk: number, texts: (string | null)[], known: string[]): boolean => {
  const fold = (t: string) => lowerPerChar(t.trim());
  const set = new Set(known.map(fold));
  return sdk >= 35 && texts.some((t) => t !== null && set.has(fold(t)));
};

// ---- skip rules (design 4.4)
const SKIPPED_CATEGORIES = new Set(["progress", "service", "sys", "status", "alarm", "call", "navigation"]);
function skipDecide(c: any): string {
  const s = { packageName: "com.example.chat", channelId: null, currentUser: true, ongoing: false, foregroundService: false, notClearable: false, groupSummary: false, localOnly: false, category: null, media: false, progressMax: 0, progressIndeterminate: false, importance: 3, suspended: false, visibility: 0, lockscreenOverride: -1000, title: "Alice", text: "hi", bigText: null, messages: 0, lines: 0, redacted: false, ...c.snapshot };
  const policy = { enabled: true, allowed: ["com.example.chat"], hiddenAllowed: [], ...c.policy };
  const flagged: string[] = c.flagged ?? [];
  if (s.packageName === c.ownPackage && !(c.debugChannel != null && typeof s.channelId === "string" && s.channelId.startsWith(c.debugChannel))) return "own";
  if (!(policy.enabled && policy.allowed.includes(s.packageName))) return "not_allowed";
  if (!s.currentUser) return "other_profile";
  if (s.ongoing || s.foregroundService || s.notClearable) return "ongoing";
  if (s.groupSummary) return "summary";
  if (s.localOnly) return "local_only";
  if (s.media || s.category === "transport") return "media";
  if ((s.category !== null && SKIPPED_CATEGORIES.has(s.category)) || (s.progressMax > 0 && !s.progressIndeterminate)) return "category";
  if (s.importance <= 1 || s.suspended) return "importance";
  const visibility = s.lockscreenOverride !== -1000 ? s.lockscreenOverride : s.visibility;
  const secret = visibility === -1;
  if (secret && !policy.hiddenAllowed.includes(s.packageName)) return "secret";
  const hasContent = (typeof s.title === "string" && s.title.trim() !== "") || (typeof s.text === "string" && s.text.trim() !== "") || (typeof s.bigText === "string" && s.bigText.trim() !== "") || s.messages > 0 || s.lines > 0;
  if (!hasContent && !s.redacted) return "empty";
  return flagged.includes(s.packageName) || secret ? "send_sensitive" : "send";
}

// ---- reply and buttons (design 4.7)
function pickActionsModel(actions: any[], clean: (s: string | null | undefined, max: number) => string | null) {
  const usable = actions.filter((a) => (a.hasIntent ?? true) && a.index >= 0 && a.index <= 31 && !(a.contextual ?? false));
  const free = (a: any) => (a.remoteInputs ?? []).some((r: any) => r.freeForm ?? true);
  const replyCands = usable.filter((a) => a.isActivity !== true && free(a));
  const replyAction = replyCands.find((a) => (a.semantic ?? 0) === 1) ?? replyCands[0];
  let reply: any = null;
  if (replyAction) {
    const input = replyAction.remoteInputs.find((r: any) => r.freeForm ?? true);
    const hint = clean(input.label, 60);
    reply = { i: replyAction.index, ...(hint !== null ? { hint } : {}) };
  }
  const buttons: any[] = [];
  for (const a of usable) {
    if (a === replyAction || (a.remoteInputs ?? []).length > 0 || a.isActivity === true) continue;
    const label = clean(a.title, 30);
    if (label !== null) buttons.push({ i: a.index, label });
    if (buttons.length === 3) break;
  }
  return { reply, actions: buttons };
}

{
  const g = "notif-phone";
  const d = load("notif.json");
  for (const k of ["otpDetect", "skip", "pickActions"]) uniqueNames(g + "." + k, d[k]);
  uniqueNames(g + ".redactedText", d.redactedText.cases);
  for (const c of d.otpDetect) {
    const got = otpDetect(c.title, c.text, c.bigText, c.personHasContactUri === true);
    check(g, `otpDetect ${c.name}`, got === c.expect, () => `expected ${j(c.expect)}, model says ${j(got)}`);
    // a code that is found is always one the PC accepts
    if (got !== null) check(g, `otpDetect ${c.name}: a code the PC accepts`, validCode(got));
  }
  // every language of the keyword table has a positive, and every negative of design 4.10 is there
  const langs = ["english", "spanish", "portuguese", "french", "german", "italian", "dutch", "swedish", "norwegian", "polish", "turkish", "russian", "ukrainian", "arabic", "persian-digits", "hindi-devanagari-digits", "indonesian", "vietnamese", "thai", "japanese", "korean", "chinese-simplified", "chinese-traditional"];
  for (const l of langs) check(g, `otpDetect has a positive for ${l}`, d.otpDetect.some((c: any) => c.name === `positive-${l}` && c.expect !== null));
  for (const n of ["order-number-has-no-keyword", "promo-code", "time-and-room-no-keyword", "amount", "phone-number-three-groups", "three-groups", "two-equal-candidates"]) check(g, `otpDetect has the negative ${n}`, d.otpDetect.some((c: any) => c.name === `negative-${n}` && c.expect === null));
  check(g, "otpDetect: the year and the code", d.otpDetect.some((c: any) => c.name === "positive-year-and-code" && c.expect === "482913"));
  for (const c of d.redactedText.cases) check(g, `redactedText ${c.name}`, redactedMatch(c.sdk, c.texts, d.redactedText.known) === c.expect);
  for (const c of d.skip) {
    const got = skipDecide(c);
    check(g, `skip ${c.name}`, got === c.expect, () => `expected ${c.expect}, model says ${got}`);
  }
  // the first rule that matches decides: every reason has a case, and every case that sends says whether it is sensitive
  const reasons = ["own", "not_allowed", "other_profile", "ongoing", "summary", "local_only", "media", "category", "importance", "secret", "empty", "send", "send_sensitive"];
  for (const r of reasons) check(g, `skip has a case for ${r}`, d.skip.some((c: any) => c.expect === r));
  check(g, "skipDefaults are the defaults of the model", eq(d.skipDefaults, { packageName: "com.example.chat", channelId: null, currentUser: true, ongoing: false, foregroundService: false, notClearable: false, groupSummary: false, localOnly: false, category: null, media: false, progressMax: 0, progressIndeterminate: false, importance: 3, suspended: false, visibility: 0, lockscreenOverride: -1000, title: "Alice", text: "hi", bigText: null, messages: 0, lines: 0, redacted: false }));
  for (const c of d.pickActions) {
    const got = pickActionsModel(c.actions, (x, max) => (typeof x === "string" ? cleanLinkText(x, max) : null));
    check(g, `pickActions ${c.name}`, eq(got, c.expect), () => `expected ${j(c.expect)}, model says ${j(got)}`);
    // never more than the PC allows: 3 buttons, indexes 0 to 31, a label of at most 30 and a hint of at most 60 code points
    check(g, `pickActions ${c.name}: within the limits`, c.expect.actions.length <= 3 && c.expect.actions.every((a: any) => a.i >= 0 && a.i <= 31 && cps(a.label).length <= 30) && (c.expect.reply === null || (c.expect.reply.i >= 0 && c.expect.reply.i <= 31 && (c.expect.reply.hint === undefined || cps(c.expect.reply.hint).length <= 60))));
  }
}

{
  const g = "scope-2b.2";
  const messages = load("messages.json");
  const matches = (name: string, p: string) => (p.endsWith("*") ? name.startsWith(p.slice(0, -1)) : name === p);
  const SCOPE: [string, any[], string[], number][] = [
    ["messages.valid", messages.valid, ["notif-*"], 30],
    ["messages.invalid", messages.invalid, ["notif-*"], 55],
    ["messages.sequences", messages.sequences, ["notif-dropped-when-feature-off-is-silent", "notif-capability-changes-with-a-new-hello", "notif-is-not-in-the-dedup-memory", "notif-action-is-acked-and-a-repeat-is-not-run-twice", "notif-action-is-not-resent-after-reconnect", "notif-sync-removes-the-rest", "notif-invalid-post-keeps-the-connection-open", "notif-status-then-icon-then-post"], 8],
  ];
  let inScope = 0;
  for (const [label, cases, patterns, atLeast] of SCOPE) {
    for (const p of patterns) check(g, `${label}: "${p}" matches a case`, cases.some((c) => matches(c.name, p)));
    const hits = cases.filter((c) => patterns.some((p) => matches(c.name, p)));
    inScope += hits.length;
    check(g, `${label}: at least ${atLeast} cases in scope`, hits.length >= atLeast, () => String(hits.length));
  }
  // every notif type has a valid example; ephemeral ones reply nothing, notif.action is acked; a dropped message never has a reply
  for (const t of Object.keys(DIRECTION).filter((t) => t.startsWith("notif."))) {
    const ex = messages.valid.filter((c: any) => c.frame?.type === t && c.expect.action !== "notif_dropped");
    check(g, `${t}: has a valid example`, ex.length > 0);
    check(g, `${t}: ${t === "notif.action" ? "is acked" : "is never acked"}`, ex.every((c: any) => c.expect.reply === (t === "notif.action" ? "ack" : "none")));
  }
  check(g, "every dropped case replies with nothing", messages.valid.filter((c: any) => c.expect.action === "notif_dropped").every((c: any) => c.expect.reply === "none" && c.expect.close === null));
  check(g, "every notif case lists the capabilities of both sides", [...messages.valid, ...messages.invalid].filter((c: any) => c.name.startsWith("notif-")).every((c: any) => Array.isArray(c.own) && Array.isArray(c.peer)));
  check(g, "wrong-direction notif messages are unsupported_type", messages.invalid.filter((c: any) => /^notif-.*wrong-direction/.test(c.name)).length >= 7);
  const d = load("notif.json");
  for (const k of ["nid", "iconHash", "icon", "clean", "post", "code", "toastIds", "toastArguments", "rate", "coalesce", "sendNotif", "syncDiff", "toastLabel", "otpDetect", "skip", "pickActions"]) check(g, `notif.json ${k}[] exists`, Array.isArray(d[k]) && d[k].length > 0);
  console.log(`milestone 2B.2 scope: ${inScope} notif cases in messages.json, plus notif.json`);
}

// ---- media.json (changelog 57)
{
  const g = "media";
  const d = load("media.json");
  for (const k of ["sid", "clean", "image", "position", "sourcePosition", "pick", "clearDebounce", "throttle", "rate", "androidMap", "toAndroid", "windowsMap", "toSmtc", "volumeLevel", "volumeSet", "sendRule", "viewerRule", "cmdResult"]) uniqueNames(g + "." + k, d[k]);
  check(g, "limits", eq(d.limits, MEDIA_LIMITS), () => j(d.limits));
  check(g, "limiters", eq(d.limiters, MEDIA_LIMITERS), () => j(d.limiters));
  for (const c of d.sid) check(g, `sid ${c.name}`, sidOf(c.identity) === c.sid && HEX32.test(c.sid), () => sidOf(c.identity));
  check(g, "sid has a package name, an exe name, a packaged app and a non-ASCII identity", d.sid.some((c: any) => c.identity.includes(".") && !c.identity.endsWith(".exe")) && d.sid.some((c: any) => c.identity.endsWith(".exe")) && d.sid.some((c: any) => c.identity.includes("!")) && d.sid.some((c: any) => /[^\x00-\x7f]/.test(c.identity)));
  const parts = (list: (string | [string, number])[]) => list.map((p) => (typeof p === "string" ? p : p[0].repeat(p[1]))).join("");
  for (const c of d.clean) {
    const max = (MEDIA_LIMITS as any)[c.field];
    check(g, `clean ${c.name}: a known field`, ["app", "title", "artist", "album"].includes(c.field) && max !== undefined);
    const input: string = c.inputParts !== undefined ? parts(c.inputParts) : c.input;
    const expected: string | null = c.expectedParts !== undefined ? parts(c.expectedParts) : c.expected;
    const got = cleanLinkText(input, max);
    check(g, `clean ${c.name}`, got === expected, () => `expected ${j(expected)}, model says ${j(got)}`);
    if (got !== null) check(g, `clean ${c.name}: stable and within the limit`, cleanLinkText(got, max) === got && cps(got).length <= max);
  }
  // image[]: a media.image frame through the receiver
  for (const c of d.image) {
    const bytes = new Uint8Array(readFileSync(join(dir, c.dataFile)));
    const frame: any = { type: "media.image", id: "image-case", hash: c.hash ?? hex(sha256(bytes)).slice(0, 32), kind: c.kind, data: base64url(bytes) };
    const out = new Receiver("pc", "open", { own: ["media"], peer: ["media"] }).receive("phone", { frame });
    const got = out.reply === "error" ? (out.code === "bad_request" ? "bad_request" : out.code) : out.action === "media_image" ? "accepted" : out.action === "media_dropped" ? "dropped" : "?";
    check(g, `image ${c.name}`, got === c.expect && out.reply !== "ack", () => `expected ${c.expect}, model says ${got}`);
    if (got === "accepted") check(g, `image ${c.name}: the frame is within 48 KiB`, frameBytes({ frame }).bytes.length <= MAX_MEDIA_IMAGE_FRAME);
  }
  check(g, "image fixtures: 24,576 and 24,577 bytes", readFileSync(join(dir, "fixtures/media-art-24576.png")).length === 24576 && readFileSync(join(dir, "fixtures/media-art-24577.png")).length === 24577);
  check(g, "image has accepted, dropped and bad_request cases", ["accepted", "dropped", "bad_request"].every((e) => d.image.some((c: any) => c.expect === e)));
  for (const c of d.position) {
    const got = mediaShown(c);
    check(g, `position ${c.name}`, eq(got, c.expect), () => `model says ${j(got)}`);
  }
  for (const c of d.sourcePosition) {
    const got = mediaSourcePosition(c);
    check(g, `sourcePosition ${c.name}`, got === c.expect, () => `model says ${j(got)}`);
    check(g, `sourcePosition ${c.name}: a known platform`, c.platform === "android" || c.platform === "windows");
  }
  for (const c of d.pick) {
    const got = mediaPick(c.sessions, c.current);
    check(g, `pick ${c.name}`, got === c.expect, () => `model says ${j(got)}`);
    // a session of Kin's own, one without a title and one that is stopped is never the answer
    const chosen = c.sessions.find((s: any) => s.sid === got);
    check(g, `pick ${c.name}: the answer is a candidate`, got === null || (chosen && !chosen.own && chosen.title && chosen.state !== "stopped"));
  }
  for (const c of d.clearDebounce) {
    const got = mediaClearTimes(c.events);
    check(g, `clearDebounce ${c.name}`, eq(got, c.expect), () => `model says ${j(got)}`);
    check(g, `clearDebounce ${c.name}: events are in time order`, c.events.every((e: any, k: number) => k === 0 || c.events[k - 1].t <= e.t));
  }
  for (const c of d.throttle) {
    const got = mediaThrottle(c.events);
    check(g, `throttle ${c.name}`, eq(got, c.expect), () => `model says ${j(got)}`);
    // never two sends less than 250 ms apart, and never the same value twice in a row
    check(g, `throttle ${c.name}: at least 250 ms apart and never a repeat`, got.every((x, k) => k === 0 || (x.t - got[k - 1].t >= MEDIA_LIMITS.throttleMs && x.value !== got[k - 1].value)));
  }
  // runs of [outcome, count]: adjacent equal outcomes are one run
  const runs = (outcomes: string[]) => {
    const out: [string, number][] = [];
    for (const o of outcomes) {
      if (out.length > 0 && out[out.length - 1][0] === o) out[out.length - 1][1]++;
      else out.push([o, 1]);
    }
    return out;
  };
  for (const c of d.rate) {
    check(g, `rate ${c.name}: a known limiter`, MEDIA_LIMITERS[c.limiter] !== undefined);
    const events = c.events.flatMap(([t, n]: [number, number]) => Array.from({ length: n }, () => ({ t })));
    check(g, `rate ${c.name}: events are in time order`, events.every((e: any, k: number) => k === 0 || events[k - 1].t <= e.t));
    const r = mediaLimiter(c.limiter, events);
    check(g, `rate ${c.name}`, eq(runs(r.outcomes), c.expect) && eq(r.errorsAt, c.errorsAt), () => `runs ${j(runs(r.outcomes))}, errors ${j(r.errorsAt)}`);
    check(g, `rate ${c.name}: the runs are canonical (no empty run, no two equal neighbours)`, c.expect.every(([o, n]: [string, number], k: number) => n > 0 && (k === 0 || c.expect[k - 1][0] !== o)));
  }
  for (const c of d.androidMap) {
    const got = androidToWire(c);
    check(g, `androidMap ${c.name}`, eq(got, c.expect), () => `model says ${j(got)}`);
  }
  for (const c of d.toAndroid) check(g, `toAndroid ${c.name}`, eq(wireToAndroid(c), c.expect), () => j(wireToAndroid(c)));
  for (const c of d.windowsMap) check(g, `windowsMap ${c.name}`, eq(windowsToWire(c), c.expect), () => j(windowsToWire(c)));
  for (const c of d.toSmtc) check(g, `toSmtc ${c.name}`, eq(wireToSmtc(c), c.expect), () => j(wireToSmtc(c)));
  check(g, "toSmtc: no testMode case ever says Playing", d.toSmtc.filter((c: any) => c.testMode).every((c: any) => c.expect.status !== "Playing"));
  for (const c of d.volumeLevel) check(g, `volumeLevel ${c.name}`, volumeLevelOf(c) === c.level, () => String(volumeLevelOf(c)));
  for (const c of d.volumeSet) check(g, `volumeSet ${c.name}`, eq(volumeSet(c), c.expect), () => j(volumeSet(c)));
  // a delta always moves at least one native step in its direction, unless it is already at the end
  for (const c of d.volumeSet.filter((c: any) => c.op.delta !== undefined && c.platform === "android")) {
    const idx = volumeSet(c).index as number;
    const atEnd = (c.op.delta > 0 && c.index === c.max) || (c.op.delta < 0 && c.index === 1);
    check(g, `volumeSet ${c.name}: moves at least one step`, atEnd ? idx === c.index : Math.sign(idx - c.index) === Math.sign(c.op.delta));
  }
  for (const c of d.sendRule) check(g, `sendRule ${c.name}`, (mediaMaySend(c) ? "send" : "nothing") === c.expected);
  for (const c of d.viewerRule) {
    const got = ["state", "image", "clear", "volume"].includes(c.kind) && !c.ownShow ? "drop" : "accept";
    check(g, `viewerRule ${c.name}`, got === c.expected);
  }
  for (const c of d.cmdResult) check(g, `cmdResult ${c.name}`, mediaCmdResult(c) === c.expected, () => mediaCmdResult(c));
}

{
  const g = "scope-2b.3";
  const messages = load("messages.json");
  const matches = (name: string, p: string) => (p.endsWith("*") ? name.startsWith(p.slice(0, -1)) : name === p);
  const SEQ = ["media-is-not-in-the-dedup-memory", "media-status-change-needs-no-new-hello", "media-capability-changes-with-a-new-hello", "media-cmd-is-not-resent-after-reconnect", "media-image-before-state", "media-invalid-state-keeps-the-connection-open"];
  const SCOPE: [string, any[], string[], number][] = [
    ["messages.valid", messages.valid, ["media-*"], 35],
    ["messages.invalid", messages.invalid, ["media-*"], 50],
    ["messages.sequences", messages.sequences, SEQ, 6],
  ];
  let inScope = 0;
  for (const [label, cases, patterns, atLeast] of SCOPE) {
    for (const p of patterns) check(g, `${label}: "${p}" matches a case`, cases.some((c) => matches(c.name, p)));
    const hits = cases.filter((c) => patterns.some((p) => matches(c.name, p)));
    inScope += hits.length;
    check(g, `${label}: at least ${atLeast} cases in scope`, hits.length >= atLeast, () => String(hits.length));
  }
  // every media type has a valid example; none is ever acked; a dropped message never has a reply; every case lists the capabilities of both sides
  for (const t of Object.keys(DIRECTION).filter((t) => t.startsWith("media."))) {
    const ex = messages.valid.filter((c: any) => c.frame?.type === t && c.expect.action !== "media_dropped");
    check(g, `${t}: has a valid example`, ex.length > 0);
    check(g, `${t}: is never acked`, ex.every((c: any) => c.expect.reply === "none"));
    check(g, `${t}: has a bad_request example`, messages.invalid.some((c: any) => c.name.startsWith("media-") && (c.frame?.type === t || c.raw?.includes(`"${t}"`)) && c.expect.code === "bad_request"));
  }
  check(g, "every media dropped case replies with nothing", messages.valid.filter((c: any) => c.expect.action === "media_dropped").every((c: any) => c.expect.reply === "none" && c.expect.close === null));
  check(g, "every media case lists the capabilities of both sides", [...messages.valid, ...messages.invalid].filter((c: any) => c.name.startsWith("media-")).every((c: any) => Array.isArray(c.own) && Array.isArray(c.peer)));
  check(g, "too_large cases for state, image and a small type", ["media-state-over-32-KiB-is-too-large", "media-image-over-48-KiB-is-too-large", "media-status-padding-over-32-KiB-is-too-large"].every((n) => messages.invalid.some((c: any) => c.name === n && c.expect.code === "too_large")));
  // the policy names of design 3.12 are pinned in media.json
  const d = load("media.json");
  for (const n of ["media-not-sent-before-peer-status"]) check(g, `sendRule ${n}`, d.sendRule.some((c: any) => c.name === n));
  check(g, "viewerRule media-dropped-when-viewer-off-is-silent", d.viewerRule.some((c: any) => c.name.startsWith("media-dropped-when-viewer-off-is-silent")));
  for (const n of ["media-cmd-to-non-sharing-source-is-disabled", "media-cmd-stale-sid-is-stale"]) check(g, `cmdResult ${n}`, d.cmdResult.some((c: any) => c.name === n));
  for (const k of ["sid", "clean", "image", "position", "sourcePosition", "pick", "clearDebounce", "throttle", "rate", "androidMap", "toAndroid", "windowsMap", "toSmtc", "volumeLevel", "volumeSet", "sendRule", "viewerRule", "cmdResult"]) check(g, `media.json ${k}[] exists`, Array.isArray(d[k]) && d[k].length > 0);
  console.log(`milestone 2B.3 scope: ${inScope} media cases in messages.json, plus media.json`);
}

// ---- presence.json (changelog 58)
// time zones: the offset of an IANA zone at an instant, from Intl (no table of our own), to the second
function zoneOffsetMs(zone: string, instantMs: number): number {
  const whole = Math.floor(instantMs / 1000) * 1000;
  const f = new Intl.DateTimeFormat("en-US", { timeZone: zone, hourCycle: "h23", year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric" });
  const p: Record<string, number> = {};
  for (const part of f.formatToParts(new Date(whole))) if (part.type !== "literal") p[part.type] = Number(part.value);
  return Date.UTC(p.year, p.month - 1, p.day, p.hour % 24, p.minute, p.second) - whole;
}
// a wall-clock time of a zone as an instant (08:00 always exists and is never ambiguous in the zones of the vectors)
function localToInstant(zone: string, y: number, m: number, d: number, hh: number, mm: number): number {
  const guess = Date.UTC(y, m - 1, d, hh, mm);
  let t = guess - zoneOffsetMs(zone, guess);
  t = guess - zoneOffsetMs(zone, t);
  return t;
}
function localParts(zone: string, instantMs: number) {
  const o = new Date(instantMs + zoneOffsetMs(zone, instantMs));
  return { y: o.getUTCFullYear(), m: o.getUTCMonth() + 1, d: o.getUTCDate() };
}
const pad2 = (n: number) => String(n).padStart(2, "0");
function isoLocal(zone: string, instantMs: number): string {
  const off = zoneOffsetMs(zone, instantMs);
  const o = new Date(instantMs + off);
  const sign = off < 0 ? "-" : "+";
  const a = Math.abs(off) / 60000;
  return `${o.getUTCFullYear()}-${pad2(o.getUTCMonth() + 1)}-${pad2(o.getUTCDate())}T${pad2(o.getUTCHours())}:${pad2(o.getUTCMinutes())}:${pad2(o.getUTCSeconds())}${sign}${pad2(Math.floor(a / 60))}:${pad2(a % 60)}`;
}
// "1h" = one hour of elapsed time; "tomorrow" = the next 08:00 of the zone's wall clock that is at least 2 hours away
function pauseUntilModel(c: { zone: string; now: string; choice: string }): string {
  const now = Date.parse(c.now);
  if (c.choice === "1h") return isoLocal(c.zone, now + 3600000);
  const lp = localParts(c.zone, now);
  let cand = localToInstant(c.zone, lp.y, lp.m, lp.d, PRESENCE_LIMITS.tomorrowHour, 0);
  if (cand - now < PRESENCE_LIMITS.tomorrowMinAheadMs) {
    const next = new Date(Date.UTC(lp.y, lp.m - 1, lp.d + 1));
    cand = localToInstant(c.zone, next.getUTCFullYear(), next.getUTCMonth() + 1, next.getUTCDate(), PRESENCE_LIMITS.tomorrowHour, 0);
  }
  return isoLocal(c.zone, cand);
}
// the gate of the pause table (protocol.md): send / drop on the sending side, flows / drop / quiet on the receiving side
const GATE_AUTO = new Set(["clipboard", "notif.post", "media.state", "media.image", "media.volume", "screenshot"]);
const GATE_MANUAL = new Set(["text", "link", "offer", "file"]);
function pauseGateModel(c: { at: string; kind: string; selfPaused: boolean; peerPaused: boolean }): string {
  if (c.at === "send") {
    if (GATE_AUTO.has(c.kind)) return c.selfPaused || c.peerPaused ? "drop" : "send";
    return GATE_MANUAL.has(c.kind) ? "send" : "flows";
  }
  // receiving: only the receiver's own pause counts. An auto-sent screenshot is an upload: it is saved, quietly.
  if (c.kind === "screenshot") return c.selfPaused ? "quiet" : "flows";
  if (GATE_AUTO.has(c.kind)) return c.selfPaused ? "drop" : "flows";
  if (GATE_MANUAL.has(c.kind)) return c.selfPaused ? "quiet" : "flows";
  return "flows";
}
function pauseExpiryModel(c: any): boolean {
  if (c.side === "peer") {
    if (c.laterPresenceWithoutPauseAt !== undefined && c.laterPresenceWithoutPauseAt <= c.nowAt) return false;
    if (c.untilResumed === true) return true;
    return c.nowAt < c.arrivalAt + Math.min(c.remainingMs, PRESENCE_LIMITS.maxPauseMs);
  }
  if (c.untilResumed === true) return true;
  if (c.pausedUntil === undefined) return false;
  return c.now < c.pausedUntil && c.pausedUntil - c.now <= PRESENCE_LIMITS.maxPauseMs;
}
function lowBatteryModel(steps: any[]): number[] {
  let armed = true;
  const out: number[] = [];
  for (const s of steps) {
    if (s.none) continue;
    if (s.charging || s.level >= PRESENCE_LIMITS.nudgeRearmAt) armed = true;
    else if (s.level <= PRESENCE_LIMITS.nudgeAt && armed && !s.paused) {
      out.push(s.t);
      armed = false;
    }
  }
  return out;
}
function batteryMapModel(c: any): { level: number; charging: boolean } | null {
  if (c.platform === "android") {
    if (c.scale <= 0 || c.level < 0) return null;
    const pct = Math.floor((c.level * 200 + c.scale) / (2 * c.scale)); // to the nearest, halves up
    return { level: Math.min(100, pct), charging: c.plugged !== 0 };
  }
  if (c.BatteryFlag === 255 || (c.BatteryFlag & 128) !== 0 || c.BatteryLifePercent === 255) return null;
  return { level: Math.min(100, c.BatteryLifePercent), charging: c.ACLineStatus === 1 };
}
// the sender of presence (3.4): a snapshot after the hello, a change at once or trailing, the battery at most every 30 s, a pause within 1 s, a one-shot at the expiry
function presenceSenderModel(events: any[]): any[] {
  const L = PRESENCE_LIMITS;
  const out: any[] = [];
  let battery: any | undefined;
  let pause: { until?: number; untilResumed?: boolean } | null = null;
  let connected = false;
  let last: { battery: any | undefined; pauseKey: string } | null = null;
  let lastT = -Infinity;
  let dueBattery: number | null = null;
  let duePause: number | null = null;
  let dueExpire: number | null = null;
  const pauseKey = (p: typeof pause, t: number) => (!p ? "none" : p.untilResumed ? "ur" : p.until! - t >= 1 ? `until:${p.until}` : "none");
  const pauseWire = (p: typeof pause, t: number) => (!p ? undefined : p.untilResumed ? { untilResumed: true } : p.until! - t >= 1 ? { remainingMs: Math.min(p.until! - t, L.maxPauseMs) } : undefined);
  const crosses = (a: number, b: number) => L.batteryCrossings.some((x) => a >= x !== b >= x);
  const significant = (a: any, b: any) => {
    if (!a && !b) return false;
    if (!a || !b) return true;
    return Math.abs(a.level - b.level) >= L.batteryMinStep || crosses(a.level, b.level) || a.charging !== b.charging || (a.saver ?? false) !== (b.saver ?? false);
  };
  const send = (t: number) => {
    const entry: any = { t };
    if (battery) entry.battery = battery;
    const pw = pauseWire(pause, t);
    if (pw) entry.pause = pw;
    out.push(entry);
    last = { battery, pauseKey: pauseKey(pause, t) };
    lastT = t;
    dueBattery = null;
    duePause = null;
  };
  const advance = (T: number) => {
    for (;;) {
      const dues = ([["expire", dueExpire], ["pause", duePause], ["battery", dueBattery]] as [string, number | null][]).filter((x): x is [string, number] => x[1] !== null && x[1] <= T).sort((a, b) => a[1] - b[1]);
      if (dues.length === 0) return;
      const [k, d] = dues[0];
      if (k === "expire") {
        dueExpire = null;
        if (connected && pauseKey(pause, d) !== last!.pauseKey) send(d);
      } else if (k === "pause") {
        duePause = null;
        if (connected && pauseKey(pause, d) !== last!.pauseKey) send(d);
      } else {
        dueBattery = null;
        if (connected && significant(last!.battery, battery)) send(d);
      }
    }
  };
  for (const ev of events) {
    advance(ev.t);
    if (ev.op === "connected") {
      connected = true;
      send(ev.t);
    } else if (ev.op === "battery") {
      battery = ev.none ? undefined : { level: ev.level, charging: ev.charging, ...(ev.saver !== undefined ? { saver: ev.saver } : {}) };
      if (connected && significant(last!.battery, battery)) {
        if (ev.t - lastT >= L.batteryEveryMs) send(ev.t);
        else if (dueBattery === null) dueBattery = lastT + L.batteryEveryMs;
      }
    } else if (ev.op === "pause") {
      pause = ev.until !== undefined && ev.until !== null ? { until: ev.until } : ev.untilResumed ? { untilResumed: true } : null;
      dueExpire = pause?.until ?? null;
      if (connected && duePause === null) duePause = ev.t + L.pauseTrailingMs;
    }
  }
  return out;
}
// the receiver's bucket for presence: 10, one token back per 3 s; one error answer per 10 s. Counted in 1/3000 of a token so there is no rounding.
function presenceReceiverModel(events: { t: number }[]): { outcomes: string[]; errorsAt: number[] } {
  const L = PRESENCE_LIMITS;
  const cap = L.receiverCapacity * L.receiverRefillMs;
  let tokens = cap;
  let last = 0;
  let lastError: number | null = null;
  const outcomes: string[] = [];
  const errorsAt: number[] = [];
  for (const e of events) {
    tokens = Math.min(cap, tokens + (e.t - last));
    last = e.t;
    const ok = tokens >= L.receiverRefillMs;
    if (ok) tokens -= L.receiverRefillMs;
    outcomes.push(ok ? "accepted" : "dropped");
    if (!ok && (lastError === null || e.t - lastError >= L.receiverErrorSpacingMs)) {
      errorsAt.push(e.t);
      lastError = e.t;
    }
  }
  return { outcomes, errorsAt };
}
// the phone's ring controller: what it answers and says, in time order
function ringModel(events: any[]): any[] {
  const L = PRESENCE_LIMITS;
  const out: any[] = [];
  let allow = true;
  let ringing = false;
  let contStart = 0;
  let deadline = 0;
  let deadlineReason = "timeout";
  let starts: number[] = [];
  let lockoutUntil = -Infinity;
  const stop = (t: number, reason: string) => {
    ringing = false;
    out.push([t, "status", "stopped", reason]);
  };
  const advance = (T: number) => {
    if (ringing && deadline <= T) {
      stop(deadline, deadlineReason);
      if (deadlineReason === "limit") lockoutUntil = deadline + L.ringLockoutMs;
    }
  };
  for (const ev of events) {
    advance(ev.t);
    if (ev.ev === "start") {
      if (!allow) out.push([ev.t, "error", "disabled"]);
      else if (ev.t < lockoutUntil) out.push([ev.t, "error", "rate_limited"]);
      else {
        starts = starts.filter((s) => ev.t - s < L.ringWindowMs);
        if (starts.length >= L.ringStartsPerWindow) out.push([ev.t, "error", "rate_limited"]);
        else {
          starts.push(ev.t);
          if (!ringing) {
            ringing = true;
            contStart = ev.t;
          }
          const timeoutAt = ev.t + L.ringTimeoutMs;
          const capAt = contStart + L.ringCapMs;
          if (timeoutAt <= capAt) [deadline, deadlineReason] = [timeoutAt, "timeout"];
          else [deadline, deadlineReason] = [capAt, "limit"];
          out.push([ev.t, "ack"]);
          out.push([ev.t, "status", "ringing", ev.audible ?? true]);
        }
      }
    } else if (ev.ev === "stop") {
      out.push([ev.t, "ack"]);
      if (ringing) stop(ev.t, "pc");
    } else if (ev.ev === "user_stop") {
      if (ringing) stop(ev.t, "user");
    } else if (ev.ev === "allow") {
      allow = ev.value;
      if (!allow && ringing) stop(ev.t, "user");
    }
  }
  return out;
}
// the picker of the phone's screenshot watcher (design 5.6): the first rule that applies decides; the baseline moves only for a paused candidate
function shotModel(c: any): { send: string | null; reason?: string; baselineAfter: number } {
  const r = c.row;
  const keep = c.baselineId;
  const skip = (reason: string, baselineAfter = keep) => ({ send: null, reason, baselineAfter });
  if (r.id <= c.baselineId || c.handled.includes(r.id)) return skip("old");
  if (r.isPending) return skip("pending");
  if (!/^image\//.test(r.mime) || r.size === 0) return skip("not_image");
  if (c.now - r.dateAdded > PRESENCE_LIMITS.shotStaleSec) return skip("stale");
  if (r.relativePath.startsWith("Pictures/Kin/") || r.relativePath.startsWith("Download/Kin/")) return skip("own");
  let kind: string;
  if (r.relativePath.toLowerCase().includes("screenshot") || r.ownerPackage === "com.android.systemui") {
    if (!c.screenshots) return skip("off");
    kind = "screenshot";
  } else if (r.relativePath.startsWith("DCIM/Camera")) {
    if (!c.camera) return skip("off");
    kind = "camera";
  } else return skip("other");
  if (c.paused) return skip("paused", Math.max(c.baselineId, r.id));
  if (!c.rateOk) return skip("rate");
  return { send: kind, baselineAfter: keep };
}
// what a 0.2.0 PC does with the image of a finished transfer (design 3.7)
function originModel(c: any) {
  const origin = c.origin === "screenshot" || c.origin === "camera" ? c.origin : null;
  if (c.paused) return { toast: "none", clipboard: false, sensitiveFormats: false };
  if (origin === "screenshot") return { toast: "screenshot", clipboard: c.imagesToClipboard, sensitiveFormats: c.imagesToClipboard };
  if (origin === "camera") return { toast: "photo", clipboard: false, sensitiveFormats: false };
  return { toast: "files", clipboard: c.imagesToClipboard, sensitiveFormats: false };
}
function featuresModel(c: any): boolean {
  const o = c.peer === null ? undefined : c.overrides[c.peer]?.[c.feature];
  if (typeof o === "boolean") return o;
  return c.defaults[c.feature] === true;
}

{
  const g = "presence";
  const d = load("presence.json");
  for (const k of ["presence", "presenceDnd", "pauseUntil", "pauseGate", "pauseExpiry", "lowBattery", "batteryMap", "presenceRate", "ring", "shot", "origin", "features"]) uniqueNames(g + "." + k, d[k]);
  // changelog 59: dnd and focus are two more optional fields of the same snapshot (presenceDnd[], same shape as presence[])
  for (const c of d.presenceDnd) {
    const out = new Receiver("pc", "open", { own: ["presence"], peer: ["presence"] }).receive("phone", c);
    const got = out.reply === "error" ? out.code : out.action === "presence" ? "accepted" : "?";
    check(g, `presenceDnd ${c.name}`, got === c.expect && out.reply !== "ack", () => `expected ${c.expect}, model says ${got}`);
    if (c.expect === "accepted") check(g, `presenceDnd ${c.name}: the content`, eq(out.presence, c.content), () => j(out.presence));
  }
  check(g, "presenceDnd[] has accepted and bad_request cases for dnd and for focus", ["dnd", "focus"].every((f) => ["accepted", "bad_request"].every((e) => d.presenceDnd.some((c: any) => c.name.startsWith(f) && c.expect === e))));
  check(g, "limits", eq(d.limits, PRESENCE_LIMITS), () => j(d.limits));
  const runs = (outcomes: string[]) => {
    const out: [string, number][] = [];
    for (const o of outcomes) {
      if (out.length > 0 && out[out.length - 1][0] === o) out[out.length - 1][1]++;
      else out.push([o, 1]);
    }
    return out;
  };
  for (const c of d.presence) {
    const out = new Receiver("pc", "open", { own: ["presence"], peer: ["presence"] }).receive("phone", c);
    const got = out.reply === "error" ? out.code : out.action === "presence" ? "accepted" : "?";
    check(g, `presence ${c.name}`, got === c.expect && out.reply !== "ack", () => `expected ${c.expect}, model says ${got}`);
    if (c.expect === "accepted") check(g, `presence ${c.name}: the content`, eq(out.presence, c.content), () => j(out.presence));
  }
  check(g, "presence[] has accepted and bad_request cases, and a case for each field", ["accepted", "bad_request"].every((e) => d.presence.some((c: any) => c.expect === e)) && ["battery", "pause", "level", "charging", "saver", "remaining", "resumed", "id"].every((f) => d.presence.some((c: any) => c.name.includes(f))));
  for (const c of d.pauseUntil) {
    check(g, `pauseUntil ${c.name}: a known choice`, c.choice === "1h" || c.choice === "tomorrow");
    const got = pauseUntilModel(c);
    check(g, `pauseUntil ${c.name}`, got === c.expect, () => `expected ${c.expect}, model says ${got}`);
    // tomorrow is always 08:00 of the wall clock
    if (c.choice === "tomorrow") check(g, `pauseUntil ${c.name}: 08:00 local and 2 hours or more away`, /T08:00:00[+-]/.test(c.expect) && Date.parse(c.expect) - Date.parse(c.now) >= PRESENCE_LIMITS.tomorrowMinAheadMs);
  }
  for (const z of ["UTC", "Europe/Berlin", "America/New_York", "Asia/Kolkata"]) check(g, `pauseUntil has cases in ${z}`, d.pauseUntil.some((c: any) => c.zone === z));
  for (const c of d.pauseGate) {
    const got = pauseGateModel(c);
    check(g, `pauseGate ${c.name}`, got === c.expected, () => `model says ${got}`);
    check(g, `pauseGate ${c.name}: a known outcome`, ["send", "drop", "quiet", "flows"].includes(c.expected) && (c.at === "send" || c.at === "receive"));
  }
  for (const c of d.pauseExpiry) check(g, `pauseExpiry ${c.name}`, pauseExpiryModel(c) === c.expected, () => String(pauseExpiryModel(c)));
  for (const c of d.lowBattery) {
    const got = lowBatteryModel(c.steps);
    check(g, `lowBattery ${c.name}`, eq(got, c.expect), () => `model says ${j(got)}`);
    check(g, `lowBattery ${c.name}: steps are in time order`, c.steps.every((s: any, k: number) => k === 0 || c.steps[k - 1].t < s.t));
  }
  for (const c of d.batteryMap) {
    const got = batteryMapModel(c);
    check(g, `batteryMap ${c.name}`, eq(got, c.expect), () => `model says ${j(got)}`);
  }
  check(g, "batteryMap has both platforms, an unknown and a no-battery case", ["android", "windows"].every((p) => d.batteryMap.some((c: any) => c.platform === p)) && d.batteryMap.some((c: any) => c.expect === null && c.BatteryFlag === 128));
  for (const c of d.presenceRate) {
    if (c.side === "sender") {
      check(g, `presenceRate ${c.name}: events are in time order and end with end`, c.events.every((e: any, k: number) => k === 0 || c.events[k - 1].t <= e.t) && c.events[c.events.length - 1].op === "end");
      const got = presenceSenderModel(c.events);
      check(g, `presenceRate ${c.name}`, eq(got, c.expect), () => `model says ${j(got)}`);
      // never two battery-driven sends less than 30 s apart unless a pause send came between (the cases that have none)
      if (!c.events.some((e: any) => e.op === "pause")) check(g, `presenceRate ${c.name}: sends are at least 30 s apart (after the first)`, got.every((x: any, k: number) => k <= 1 || x.t - got[k - 1].t >= PRESENCE_LIMITS.batteryEveryMs));
    } else {
      check(g, `presenceRate ${c.name}: a known side`, c.side === "receiver");
      const events = c.events.flatMap(([t, n]: [number, number]) => Array.from({ length: n }, () => ({ t })));
      const r = presenceReceiverModel(events);
      check(g, `presenceRate ${c.name}`, eq(runs(r.outcomes), c.expect) && eq(r.errorsAt, c.errorsAt), () => `runs ${j(runs(r.outcomes))}, errors ${j(r.errorsAt)}`);
    }
  }
  for (const c of d.ring) {
    check(g, `ring ${c.name}: events are in time order`, c.events.every((e: any, k: number) => k === 0 || c.events[k - 1].t <= e.t));
    const got = ringModel(c.events);
    check(g, `ring ${c.name}`, eq(got, c.expect), () => `model says ${j(got)}`);
    // a ring.stop is always acked, and the output is in time order
    check(g, `ring ${c.name}: every stop is acked and the answers are in time order`, c.events.filter((e: any) => e.ev === "stop").every((e: any) => got.some((o: any) => o[0] === e.t && o[1] === "ack")) && got.every((o: any, k: number) => k === 0 || got[k - 1][0] <= o[0]));
  }
  for (const c of d.shot) {
    const got = shotModel(c);
    check(g, `shot ${c.name}`, eq(got, c.expect.send === null ? { send: null, reason: c.expect.reason, baselineAfter: c.expect.baselineAfter } : { send: c.expect.send, baselineAfter: c.expect.baselineAfter }), () => `model says ${j(got)}`);
    check(g, `shot ${c.name}: the baseline never goes down`, c.expect.baselineAfter >= c.baselineId);
  }
  for (const r of ["old", "pending", "not_image", "stale", "own", "off", "other", "paused", "rate"]) check(g, `shot has a case for ${r}`, d.shot.some((c: any) => c.expect.reason === r));
  check(g, "shot has a screenshot and a camera case that are sent", ["screenshot", "camera"].every((k) => d.shot.some((c: any) => c.expect.send === k)));
  for (const c of d.origin) check(g, `origin ${c.name}`, eq(originModel(c), c.expect), () => j(originModel(c)));
  check(g, "origin: a camera photo never reaches the clipboard", d.origin.filter((c: any) => c.origin === "camera").every((c: any) => c.expect.clipboard === false));
  check(g, "origin: a paused PC does nothing visible", d.origin.filter((c: any) => c.paused).every((c: any) => c.expect.toast === "none" && c.expect.clipboard === false));
  for (const c of d.features) check(g, `features ${c.name}`, featuresModel(c) === c.expected, () => String(featuresModel(c)));
}

{
  const g = "scope-2b.4";
  const messages = load("messages.json");
  const matches = (name: string, p: string) => (p.endsWith("*") ? name.startsWith(p.slice(0, -1)) : name === p);
  const SEQ = ["presence-not-sent-without-capability", "presence-dropped-when-feature-off-is-silent", "presence-is-not-acked-or-deduplicated", "presence-snapshot-replaces-pause", "presence-forgotten-on-close", "ring-not-sent-without-capability", "ring-capability-follows-the-switch", "ring-start-is-not-resent-after-reconnect", "ring-start-is-acked-once", "ring-status-is-ephemeral", "ring-disabled-error-is-not-remembered"];
  const SCOPE: [string, any[], string[], number][] = [
    ["messages.valid", messages.valid, ["presence-*", "ring-*"], 15],
    ["messages.invalid", messages.invalid, ["presence-*", "ring-*"], 15],
    ["messages.sequences", messages.sequences, SEQ, 11],
  ];
  let inScope = 0;
  for (const [label, cases, patterns, atLeast] of SCOPE) {
    for (const p of patterns) check(g, `${label}: "${p}" matches a case`, cases.some((c) => matches(c.name, p)));
    const hits = cases.filter((c) => patterns.some((p) => matches(c.name, p)));
    inScope += hits.length;
    check(g, `${label}: at least ${atLeast} cases in scope`, hits.length >= atLeast, () => String(hits.length));
  }
  for (const n of ["presence-minimal", "presence-full", "ring-start-ack", "ring-start-disabled", "ring-start-rate-limited", "ring-stop-idempotent"]) check(g, `messages has ${n}`, [...messages.valid, ...messages.invalid].some((c: any) => c.name === n));
  // every new type has a valid example; presence and ring.status are never acked, ring.start and ring.stop always are
  for (const t of ["presence", "ring.start", "ring.stop", "ring.status"]) {
    const ex = messages.valid.filter((c: any) => c.frame?.type === t && !String(c.expect.action).endsWith("_dropped"));
    check(g, `${t}: has a valid example`, ex.length > 0);
    check(g, `${t}: ${t === "ring.start" || t === "ring.stop" ? "is always acked" : "is never acked"}`, ex.every((c: any) => c.expect.reply === (t === "ring.start" || t === "ring.stop" ? "ack" : "none")));
    check(g, `${t}: has a bad_request or error example`, messages.invalid.some((c: any) => c.name.startsWith(t === "presence" ? "presence-" : "ring-") && (c.frame?.type === t || c.raw?.includes(`"${t}"`) || c.synthesize?.base?.type === t) && c.expect.reply === "error"));
  }
  check(g, "every presence dropped case replies with nothing", messages.valid.filter((c: any) => c.expect.action === "presence_dropped").every((c: any) => c.expect.reply === "none" && c.expect.close === null));
  check(g, "every presence and ring case lists the capabilities of both sides", [...messages.valid, ...messages.invalid].filter((c: any) => c.name.startsWith("presence-") || c.name.startsWith("ring-")).every((c: any) => Array.isArray(c.own) && Array.isArray(c.peer)));
  check(g, "too_large cases for presence, ring.status and ring.start", ["presence-over-4-KiB-is-too-large", "ring-status-over-2-KiB-is-too-large", "ring-start-over-2-KiB-is-too-large"].every((n) => messages.invalid.some((c: any) => c.name === n && c.expect.code === "too_large")));
  console.log(`milestone 2B.4 scope: ${inScope} presence and ring cases in messages.json, plus presence.json`);
}

// ---- alerts.json (changelog 59): Do Not Disturb, Focus and alerts
const ALERT_LIMITS = {
  frame: 4096, textMax: 300, titleMax: 60, waitedMaxMs: 604800000, waitedOmitBelowMs: 1000,
  phoneCapacity: 10, phoneRefillMs: 6000,
  ttlDefaultSec: 86400, ttlMinSec: 60, ttlMaxSec: 604800,
  focusStableMs: 3000, focusPollMs: 15000, dndGraceMs: 60000,
  pipeSendsPerWindow: 30, pipeWindowMs: 10000,
  downloadNameMax: 120, downloadMinSeconds: 30, downloadTtlSec: 1800, downloadCoalesceMs: 3000,
};
const buildParts = (parts: [string, number][]): string => parts.map(([s, n]) => s.repeat(n)).join("");
// the PC's decision for an alert item about to go on the wire (3.2)
function alertSendModel(c: any): any {
  if (c.nowMs >= c.createdMs + c.ttlMs) return { result: "expired" };
  if (!c.connected) return { result: "queued" };
  if (!c.peerCaps.includes("alert")) return { result: "peer_unsupported" };
  const waited = Math.min(c.nowMs - c.createdMs, ALERT_LIMITS.waitedMaxMs);
  return waited >= ALERT_LIMITS.waitedOmitBelowMs ? { result: "sent", waitedMs: waited } : { result: "sent" };
}
// the phone's bucket, in 1/6000 of a token so there is no rounding: capacity 10, one token back per 6000 ms
function alertRateModel(times: number[]): string[] {
  const cost = ALERT_LIMITS.phoneRefillMs;
  const cap = ALERT_LIMITS.phoneCapacity * cost;
  let units = cap;
  let last = times.length > 0 ? times[0] : 0;
  return times.map((t) => {
    units = Math.min(cap, units + (t - last));
    last = t;
    if (units >= cost) {
      units -= cost;
      return "accepted";
    }
    return "rate_limited";
  });
}
// 3.5: what Windows says now -> is the PC in Focus
const PRESENTING = new Set(["busy", "fullscreen", "presentation"]);
const focusRule = (s: { focusSession: boolean; quiet: string }, alsoPresenting: boolean): boolean =>
  s.focusSession || s.quiet === "quiet_time" || (alsoPresenting && PRESENTING.has(s.quiet));
// the 3 s stability filter: a changed value is sent when it has stayed for 3000 ms
function focusTimelineModel(c: any): { t: number; focus: boolean }[] {
  const out: { t: number; focus: boolean }[] = [];
  let stable = false;
  let candidate = false;
  let since = 0;
  const flush = (upTo: number) => {
    if (candidate !== stable && since + ALERT_LIMITS.focusStableMs <= upTo) {
      stable = candidate;
      out.push({ t: since + ALERT_LIMITS.focusStableMs, focus: stable });
    }
  };
  for (const s of c.samples) {
    flush(s.t);
    const v = focusRule(s, c.alsoPresenting);
    if (v !== candidate) {
      candidate = v;
      since = s.t;
    }
  }
  flush(c.endAt);
  return out;
}
const dndWireOf = (filter: number): string | null => (filter === 2 ? "priority" : filter === 4 ? "alarms" : filter === 3 ? "total" : null);
const dndRequestOf = (setting: string | null): number => (setting === "alarms" ? 4 : 2);
// 4.2: the phone's follower. Events are described in alerts.json howToRead.dndFollow.
function dndFollowModel(c: any): { do: string[]; weHold: boolean }[] {
  let { switch: sw, access, paused, filter, follow, weHold } = c.init;
  weHold = weHold ?? false;
  let presence = false;
  let peerFocus = false;
  let grace: number | null = null;
  let held = follow;
  const want = () => sw && access && !paused && presence && peerFocus;
  let wantPrev = want();
  const results: { do: string[]; weHold: boolean }[] = [];
  const evaluate = (out: string[]) => {
    const now = want();
    if (!wantPrev && now && !weHold) {
      if (filter === "all") {
        out.push("request:" + follow);
        weHold = true;
        held = follow;
      } else out.push("leave");
    }
    if (weHold && !now) {
      out.push("release");
      weHold = false;
    }
    wantPrev = now;
  };
  for (const s of c.steps) {
    const out: string[] = [];
    if (grace !== null && s.t >= grace) {
      grace = null;
      peerFocus = false;
      evaluate(out);
    }
    switch (s.ev) {
      case "connect": presence = s.presence; break;
      case "snapshot": peerFocus = s.focus; grace = null; break;
      case "close": if (weHold) grace = s.t + ALERT_LIMITS.dndGraceMs; else peerFocus = false; break;
      case "pause": paused = s.on; break;
      case "switch": sw = s.on; break;
      case "access": access = s.granted; break;
      case "filter":
        filter = s.value;
        if (weHold && s.value !== held) {
          out.push("override");
          weHold = false;
        }
        break;
      case "unlink": peerFocus = false; presence = false; grace = null; break;
      case "start":
        if (weHold) {
          out.push("release");
          weHold = false;
        }
        peerFocus = false;
        presence = false;
        grace = null;
        wantPrev = false;
        break;
      case "tick": break;
      default: throw new Error("unknown dndFollow event " + s.ev);
    }
    evaluate(out);
    results.push({ do: out, weHold });
  }
  return results;
}
const holdModel = (c: any): string => (c.setting && c.dnd !== null && (c.kind === "low_battery" || c.kind === "info") ? "held" : "shown");
// 5.3: `Kin.exe --notify` arguments. The pipe request it makes has source cli and auto false; the text is checked, never cut.
function notifyCliModel(c: any): any {
  const args: string[] = c.args.map((a: any) => (typeof a === "string" ? a : buildParts(a.parts)));
  const usage = (reason: string, exit = 2, count?: number) => ({ kind: "usage", exit, reason, ...(count !== undefined ? { count } : {}) });
  let title = "";
  let ttlSec: number = ALERT_LIMITS.ttlDefaultSec;
  let device: string | undefined;
  let noStart = false;
  const pos: string[] = [];
  let rest = false;
  for (let i = 0; i < args.length; ) {
    const a = args[i];
    if (rest || a === "-" || !a.startsWith("-")) {
      pos.push(a);
      i++;
      continue;
    }
    if (a === "--") {
      rest = true;
      i++;
      continue;
    }
    const eqAt = a.startsWith("--") ? a.indexOf("=") : -1;
    const name = eqAt >= 0 ? a.slice(0, eqAt) : a;
    if (name === "--no-start" && eqAt < 0) {
      noStart = true;
      i++;
      continue;
    }
    if (name !== "--title" && name !== "--ttl" && name !== "--device") return usage("unknown_flag");
    let value: string;
    if (eqAt >= 0) {
      value = a.slice(eqAt + 1);
      i++;
    } else {
      if (i + 1 >= args.length) return usage("missing_value");
      value = args[i + 1];
      i += 2;
    }
    if (name === "--title") title = value.trim();
    else if (name === "--device") device = value;
    else {
      const m = /^([0-9]+)([smhd])$/.exec(value);
      const sec = m ? Number(m[1]) * { s: 1, m: 60, h: 3600, d: 86400 }[m[2] as "s" | "m" | "h" | "d"] : NaN;
      if (!(sec >= ALERT_LIMITS.ttlMinSec && sec <= ALERT_LIMITS.ttlMaxSec)) return usage("bad_ttl");
      ttlSec = sec;
    }
  }
  let text: string;
  if (pos.length === 1 && pos[0] === "-") {
    const bytes = c.stdinParts ? new TextEncoder().encode(buildParts(c.stdinParts)) : unhex(c.stdinHex ?? "");
    try {
      text = new TextDecoder("utf-8", { fatal: true }).decode(bytes).trim();
    } catch {
      return usage("stdin_not_utf8");
    }
  } else text = pos.join(" ").trim();
  if (text === "") return usage("missing_text");
  if (cps(text).length > ALERT_LIMITS.textMax) return usage("text_too_long", 5, cps(text).length);
  if (cps(title).length > ALERT_LIMITS.titleMax) return usage("title_too_long", 5, cps(title).length);
  return { kind: "send", text, ttlSec, ...(title !== "" ? { title } : {}), ...(device !== undefined ? { device } : {}), ...(noStart ? { noStart: true } : {}) };
}
function notifyDeviceModel(c: any): any {
  const byId = c.devices.filter((d: any) => d.id === c.selector);
  const byName = c.devices.filter((d: any) => d.name.toLowerCase() === c.selector.toLowerCase());
  const byPrefix = c.selector.length >= 4 ? c.devices.filter((d: any) => d.id.startsWith(c.selector)) : [];
  for (const found of [byId, byName, byPrefix]) {
    if (found.length === 1) return { result: "match", id: found[0].id };
    if (found.length > 1) return { result: "ambiguous" };
  }
  return { result: "none" };
}
function notifyExitModel(c: any): any {
  const r = c.response;
  if (r === null) return { exit: 3, word: "not_running" };
  if (r.ok === true) return { exit: 0, word: r.queued === true && r.code === "DEVICE_OFFLINE_QUEUED" ? "queued" : r.code === "PAUSED_SKIPPED" ? "skipped" : "sent" };
  const exit = r.code === "KIN_NOT_RUNNING" ? 3 : r.code === "NOT_PAIRED" ? 4 : r.code === "TOO_LARGE" || (r.code === "INTERNAL" && r.rateLimited === true) ? 5 : 6;
  return { exit, word: "refused" };
}
// 5.4: the browser extension's download alert
function downloadAlertModel(c: any): any[] {
  // remembered by id AND state: interrupted-then-resumed is two alerts, the same state again is none
  const handled = new Set<string>();
  const sends: any[] = [];
  let burst: { start: number; items: string[] } | null = null;
  const nameOf = (path: string): string => {
    const base = cleanLinkText(path.split(/[\\/]/).pop() ?? "", Number.MAX_SAFE_INTEGER) ?? "";
    return base === "" ? "A file" : cps(base).slice(0, ALERT_LIMITS.downloadNameMax).join("");
  };
  const close = () => {
    if (!burst) return;
    const done = burst.items.filter((s) => s === "complete").length;
    const failed = burst.items.length - done;
    const names = (burst as any).names as string[];
    let s: any;
    if (burst.items.length === 1) s = { title: failed === 1 ? "Download failed" : "Download finished", text: names[0] };
    else if (failed === 0) s = { title: "Downloads finished", text: `${done} downloads finished` };
    else if (done === 0) s = { title: "Downloads failed", text: `${failed} downloads failed` };
    else s = { title: "Downloads finished", text: `${done} finished, ${failed} failed` };
    sends.push({ t: burst.start + ALERT_LIMITS.downloadCoalesceMs, ...s });
    burst = null;
  };
  for (const e of c.events) {
    if (burst && e.t >= burst.start + ALERT_LIMITS.downloadCoalesceMs) close();
    const key = `${e.id}:${e.state}`;
    if (!c.setting.enabled || e.incognito || handled.has(key)) continue;
    handled.add(key);
    if (e.state === "interrupted" && e.error === "USER_CANCELED") continue;
    if (c.setting.onlyLong && e.durationMs < ALERT_LIMITS.downloadMinSeconds * 1000) continue;
    if (!burst) burst = Object.assign({ start: e.t, items: [] as string[] }, { names: [] as string[] });
    burst.items.push(e.state);
    (burst as any).names.push(nameOf(e.path + (e.pathParts ? buildParts(e.pathParts) : "")));
  }
  close();
  return sends;
}

{
  const g = "alerts";
  const d = load("alerts.json");
  for (const k of ["alert", "alertSend", "alertRate", "focusState", "focusTimeline", "focusSnapshot", "dndMap", "dndRequest", "dndFollow", "holdToasts", "notifyCli", "notifyDevice", "notifyExit", "downloadAlert"]) uniqueNames(g + "." + k, d[k]);
  check(g, "limits", eq(d.limits, ALERT_LIMITS), () => j(d.limits));
  for (const c of d.alert) {
    const frame = c.frame !== undefined ? { ...c.frame } : undefined;
    if (frame && c.frameParts) for (const [f, parts] of Object.entries(c.frameParts)) frame[f] = buildParts(parts as [string, number][]);
    const input = frame !== undefined ? { frame } : { raw: c.raw };
    const out = new Receiver("phone", "open", { own: ["alert"], peer: [] }).receive("pc", input);
    const got = out.reply === "error" ? out.code : out.reply === "ack" ? "accepted" : "?";
    check(g, `alert ${c.name}`, got === c.expect, () => `expected ${c.expect}, model says ${got} ${j(out)}`);
    if (c.expect === "accepted") {
      const content: any = { ...(c.content ?? {}) };
      if (c.contentParts) for (const [f, parts] of Object.entries(c.contentParts)) content[f] = buildParts(parts as [string, number][]);
      check(g, `alert ${c.name}: the content`, eq(out.alert, content), () => j(out.alert));
    } else check(g, `alert ${c.name}: the error names the frame when it has a usable id`, (out.ref ?? null) === (frame?.id && validMsgId(frame.id) ? frame.id : c.raw ? "0193a5f0-7b1e-7c3a-9d4f-5e6a7b8c0800" : null));
  }
  for (const c of d.alertSend) check(g, `alertSend ${c.name}`, eq(alertSendModel(c), c.expect), () => j(alertSendModel(c)));
  for (const r of ["sent", "queued", "peer_unsupported", "expired"]) check(g, `alertSend[] has a ${r} case`, d.alertSend.some((c: any) => c.expect.result === r));
  for (const c of d.alertRate) {
    check(g, `alertRate ${c.name}: events are in time order`, c.events.every((e: any, k: number) => k === 0 || c.events[k - 1].t <= e.t));
    const got = alertRateModel(c.events.map((e: any) => e.t));
    check(g, `alertRate ${c.name}`, eq(got, c.expect), () => j(got));
  }
  for (const c of d.focusState) check(g, `focusState ${c.name}`, focusRule(c, c.alsoPresenting) === c.expect, () => `model says ${focusRule(c, c.alsoPresenting)}`);
  for (const c of d.focusTimeline) {
    check(g, `focusTimeline ${c.name}: samples are in time order`, c.samples.every((e: any, k: number) => k === 0 || c.samples[k - 1].t <= e.t));
    const got = focusTimelineModel(c);
    check(g, `focusTimeline ${c.name}`, eq(got, c.expect), () => j(got));
  }
  for (const c of d.focusSnapshot) {
    const got = c.shareOn && c.stable && !c.paused && c.peerHasPresence;
    check(g, `focusSnapshot ${c.name}`, got === c.expect, () => `model says ${got}`);
  }
  for (const c of d.dndMap) check(g, `dndMap ${c.name}`, dndWireOf(c.filter) === c.wire, () => `model says ${dndWireOf(c.filter)}`);
  for (const c of d.dndRequest) check(g, `dndRequest ${c.name}`, dndRequestOf(c.setting) === c.filter && c.filter !== 3, () => `model says ${dndRequestOf(c.setting)}`);
  for (const c of d.dndFollow) {
    const got = dndFollowModel(c);
    check(g, `dndFollow ${c.name}: steps are in time order`, c.steps.every((s: any, k: number) => k === 0 || c.steps[k - 1].t <= s.t));
    c.steps.forEach((s: any, k: number) => check(g, `dndFollow ${c.name} step ${k + 1} (${s.ev})`, eq(got[k], { do: s.do, weHold: s.weHold }), () => `expected ${j({ do: s.do, weHold: s.weHold })}, model says ${j(got[k])}`));
    // the invariants of the design: Kin never requests anything while the phone's filter is not "all" and never releases what it does not hold
    check(g, `dndFollow ${c.name}: the model never requests twice in a row without a release or override`, (() => {
      let holding = c.init.weHold ?? false;
      for (const r of got) for (const a of r.do) {
        if (a.startsWith("request:")) { if (holding) return false; holding = true; }
        else if (a === "release" || a === "override") { if (a === "release" && !holding) return false; holding = false; }
      }
      return true;
    })());
  }
  for (const c of d.holdToasts) check(g, `holdToasts ${c.name}`, holdModel(c) === c.expect, () => `model says ${holdModel(c)}`);
  for (const c of d.notifyCli) {
    const got = notifyCliModel(c);
    const expect = { ...c.expect };
    const shown = { ...got };
    if (expect.textLength !== undefined) {
      check(g, `notifyCli ${c.name}: the text length`, got.kind === "send" && cps(got.text).length === expect.textLength, () => j(got));
      delete expect.textLength;
      delete shown.text;
    }
    if (expect.titleLength !== undefined) {
      check(g, `notifyCli ${c.name}: the title length`, got.kind === "send" && cps(got.title ?? "").length === expect.titleLength, () => j(got));
      delete expect.titleLength;
      delete shown.title;
    }
    check(g, `notifyCli ${c.name}`, eq(shown, expect), () => j(got));
  }
  for (const c of d.notifyDevice) check(g, `notifyDevice ${c.name}`, eq(notifyDeviceModel(c), c.expect), () => j(notifyDeviceModel(c)));
  for (const c of d.notifyExit) check(g, `notifyExit ${c.name}`, eq(notifyExitModel(c), c.expect), () => j(notifyExitModel(c)));
  for (const e of [0, 2, 3, 4, 5, 6]) check(g, `exit code ${e} is covered by notifyCli[] or notifyExit[]`, d.notifyCli.some((c: any) => (c.expect.exit ?? 0) === e && (e !== 0 || c.expect.kind === "send")) || d.notifyExit.some((c: any) => c.expect.exit === e));
  for (const c of d.downloadAlert) {
    const expect = c.expect.map((x: any) => {
      const { textParts, ...rest } = x;
      return textParts ? { ...rest, text: buildParts(textParts) } : rest;
    });
    const got = downloadAlertModel(c);
    check(g, `downloadAlert ${c.name}`, eq(got, expect), () => j(got));
  }
  // the browser's alert is event-driven: auto true, a 30 minute ttl, source downloads (pinned in the spec, not per case)
  console.log(`alerts.json: ${d.alert.length} alert, ${d.dndFollow.length} dndFollow, ${d.notifyCli.length} notifyCli, ${d.downloadAlert.length} downloadAlert cases`);
}

{
  const g = "scope-2b.5";
  const messages = load("messages.json");
  const matches = (name: string, p: string) => (p.endsWith("*") ? name.startsWith(p.slice(0, -1)) : name === p);
  const SEQ = ["alert-is-queued-and-acked", "alert-repeat-gets-the-first-answer", "alert-rate-limited-fails-the-item", "alert-quiet-when-paused", "alert-capability-is-the-senders-rule", "alert-expires-locally"];
  const SCOPE: [string, any[], string[], number][] = [
    ["messages.valid", messages.valid, ["alert-*"], 8],
    ["messages.invalid", messages.invalid, ["alert-*"], 12],
    ["messages.sequences", messages.sequences, SEQ, 6],
  ];
  let inScope = 0;
  for (const [label, cases, patterns, atLeast] of SCOPE) {
    for (const p of patterns) check(g, `${label}: "${p}" matches a case`, cases.some((c) => matches(c.name, p)));
    const hits = cases.filter((c) => patterns.some((p) => matches(c.name, p)));
    inScope += hits.length;
    check(g, `${label}: at least ${atLeast} cases in scope`, hits.length >= atLeast, () => String(hits.length));
  }
  for (const n of ["alert-minimal", "alert-full", "alert-text-missing", "alert-over-4-KiB-is-too-large", "alert-rate-limited"]) check(g, `messages has ${n}`, [...messages.valid, ...messages.invalid].some((c: any) => c.name === n));
  check(g, "every valid alert is acked with deliver_alert", messages.valid.filter((c: any) => c.frame?.type === "alert" || c.synthesize?.base?.type === "alert").every((c: any) => c.expect.reply === "ack" && c.expect.action === "deliver_alert"));
  check(g, "every alert case lists the capabilities of both sides", [...messages.valid, ...messages.invalid].filter((c: any) => c.name.startsWith("alert-")).every((c: any) => Array.isArray(c.own) && Array.isArray(c.peer)));
  const presence = load("presence.json");
  check(g, "presenceDnd[] is in presence.json", Array.isArray(presence.presenceDnd) && presence.presenceDnd.length >= 20);
  console.log(`milestone 2B.5 scope: ${inScope} alert cases in messages.json, plus alerts.json and presenceDnd[]`);
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
