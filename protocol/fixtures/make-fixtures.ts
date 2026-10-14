// Writes the DER fixtures for the negative `tls[]` cases of http.json:
//
//   bun protocol/fixtures/make-fixtures.ts
//
// Neither rcgen nor OpenSSL 3 can make an X.509 v1 certificate, and neither can be told to write an unknown critical
// extension, so these are built by hand here: a small DER writer, and the test keys of device-id.json (or a fresh P-384
// key) to sign. Every certificate carries a *valid* signature over its own key. The only thing wrong with it is the one
// thing its name says, so a handshake that fails with it fails for that reason.
//
//   client-v1.der                            X.509 v1 (no version field). Key: device-id.json test key 2.
//   client-unknown-critical-extension.der    v3 with a critical extension nobody knows (1.3.6.1.4.1.99999.1). Key: test key 2.
//   client-not-p256.der                      v3, P-384 key, signed ecdsa-with-SHA256. Key: client-not-p256.key.der.
//   client-not-p256.key.der                  The P-384 private key, PKCS#8. A test-only key, public in this repo.
//
// ECDSA signatures are randomized, so running this again writes different bytes. Nothing depends on the exact bytes.
import { createPrivateKey, createPublicKey, generateKeyPairSync, randomBytes, sign, type KeyObject } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));

// ----- a minimal DER writer
const cat = (...parts: Uint8Array[]): Uint8Array => Uint8Array.from(parts.flatMap((p) => [...p]));
function len(n: number): Uint8Array {
  if (n < 128) return Uint8Array.of(n);
  const bytes: number[] = [];
  for (let v = n; v > 0; v = Math.floor(v / 256)) bytes.unshift(v % 256);
  return Uint8Array.of(0x80 | bytes.length, ...bytes);
}
const tlv = (tag: number, ...content: Uint8Array[]): Uint8Array => {
  const c = cat(...content);
  return cat(Uint8Array.of(tag), len(c.length), c);
};
const seq = (...c: Uint8Array[]) => tlv(0x30, ...c);
const set = (...c: Uint8Array[]) => tlv(0x31, ...c);
const integer = (bytes: Uint8Array) => tlv(0x02, bytes);
const bool = (v: boolean) => tlv(0x01, Uint8Array.of(v ? 0xff : 0x00));
const octets = (b: Uint8Array) => tlv(0x04, b);
const utf8 = (s: string) => tlv(0x0c, new TextEncoder().encode(s));
function oid(dotted: string): Uint8Array {
  const arcs = dotted.split(".").map(Number);
  const out: number[] = [arcs[0] * 40 + arcs[1]];
  for (const arc of arcs.slice(2)) {
    const groups: number[] = [arc & 0x7f];
    for (let v = arc >>> 7; v > 0; v >>>= 7) groups.unshift((v & 0x7f) | 0x80);
    out.push(...groups);
  }
  return tlv(0x06, Uint8Array.from(out));
}

// ----- the certificate pieces
const ECDSA_SHA256 = seq(oid("1.2.840.10045.4.3.2")); // no parameters, as in RFC 5758
const name = (cn: string) => seq(set(seq(oid("2.5.4.3"), utf8(cn))));
const validity = seq(tlv(0x17, new TextEncoder().encode("261014000000Z")), tlv(0x18, new TextEncoder().encode("21261014000000Z")));
const serial = () => {
  const b = randomBytes(16);
  b[0] &= 0x7f; // positive
  b[0] |= 0x01; // and no redundant leading zero
  return integer(b);
};

type Extra = { v1?: boolean; extensions?: Uint8Array };
function certificate(cn: string, publicKey: KeyObject, signer: KeyObject, extra: Extra = {}): Uint8Array {
  const spki = new Uint8Array(publicKey.export({ type: "spki", format: "der" }));
  const tbs = seq(
    ...(extra.v1 ? [] : [tlv(0xa0, integer(Uint8Array.of(2)))]), // [0] version, 2 = v3
    serial(),
    ECDSA_SHA256,
    name(cn),
    validity,
    name(cn),
    spki,
    ...(extra.extensions ? [tlv(0xa3, extra.extensions)] : []),
  );
  const signature = new Uint8Array(sign("sha256", tbs, { key: signer, dsaEncoding: "der" }));
  return seq(tbs, ECDSA_SHA256, tlv(0x03, Uint8Array.of(0), signature));
}

// ----- the fixtures
const keys = JSON.parse(readFileSync(join(dir, "..", "device-id.json"), "utf8")).keys;
const key2 = createPrivateKey(keys[1].privateKeyPkcs8Pem);
const pub2 = createPublicKey(key2);

writeFileSync(join(dir, "client-v1.der"), certificate("kin-fixture-v1", pub2, key2, { v1: true }));

// Extension ::= SEQUENCE { extnID, critical BOOLEAN, extnValue OCTET STRING }. An unknown OID that is marked critical.
const unknownCritical = seq(seq(oid("1.3.6.1.4.1.99999.1"), bool(true), octets(Uint8Array.of(0x05, 0x00))));
writeFileSync(join(dir, "client-unknown-critical-extension.der"), certificate("kin-fixture-critical", pub2, key2, { extensions: unknownCritical }));

const p384 = generateKeyPairSync("ec", { namedCurve: "P-384" });
writeFileSync(join(dir, "client-not-p256.der"), certificate("kin-fixture-p384", p384.publicKey, p384.privateKey));
writeFileSync(join(dir, "client-not-p256.key.der"), new Uint8Array(p384.privateKey.export({ type: "pkcs8", format: "der" })));

console.log("wrote client-v1.der, client-unknown-critical-extension.der, client-not-p256.der, client-not-p256.key.der");
