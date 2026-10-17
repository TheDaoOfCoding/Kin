// Generates the stable development key of the extension (run once; the public key is committed).
// The private key is thrown away: it is only needed to sign a .crx, and the stores sign their own.
import { createHash, generateKeyPairSync } from "node:crypto";
import { writeFileSync } from "node:fs";

const { publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const der = publicKey.export({ type: "spki", format: "der" });
const key = der.toString("base64");
const id = [...createHash("sha256").update(der).digest().subarray(0, 16)]
  .map((b) => String.fromCharCode(97 + (b >> 4)) + String.fromCharCode(97 + (b & 15)))
  .join("");
writeFileSync(new URL("../manifest-key.json", import.meta.url), JSON.stringify({ key, id }, null, 2) + "\n");
console.log("extension id:", id);
