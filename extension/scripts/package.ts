// bun scripts/package.ts  (after `bun scripts/build.ts`)
// Makes the store-ready zips: dist/kin-chromium-<version>.zip and dist/kin-firefox-<version>.zip, plus
// dist/kin-chromium-<version>-sideload.zip for loading unpacked before a store listing.
// The Chromium store zip leaves the manifest `key` out (the stores give their own ID); the side-load zip
// keeps it (the public key, ID cbioefpddjpaionhljhffgbedncfchmn). The zips are reproducible: files sorted, fixed timestamp.
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { deflateRawSync } from "node:zlib";
import pkg from "../package.json";
import { buildManifest, type Target } from "./manifest";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist");

const crcTable = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(buf: Uint8Array): number {
  let c = 0xffffffff;
  for (const b of buf) c = crcTable[(c ^ b) & 0xff]! ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...walk(p));
    else out.push(p);
  }
  return out;
}

export function zip(files: { name: string; data: Uint8Array }[]): Buffer {
  const DOS_TIME = 0;
  const DOS_DATE = (2026 - 1980) << 9 | (1 << 5) | 1; // 2026-01-01
  const parts: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const f of files) {
    const name = Buffer.from(f.name, "utf8");
    const raw = Buffer.from(f.data);
    const deflated = deflateRawSync(raw, { level: 9 });
    const useDeflate = deflated.length < raw.length;
    const body = useDeflate ? deflated : raw;
    const crc = crc32(raw);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0x0800, 6); // UTF-8 names
    local.writeUInt16LE(useDeflate ? 8 : 0, 8);
    local.writeUInt16LE(DOS_TIME, 10);
    local.writeUInt16LE(DOS_DATE, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(body.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(name.length, 26);
    local.writeUInt16LE(0, 28);
    parts.push(local, name, body);
    const c = Buffer.alloc(46);
    c.writeUInt32LE(0x02014b50, 0);
    c.writeUInt16LE(20, 4);
    c.writeUInt16LE(20, 6);
    c.writeUInt16LE(0x0800, 8);
    c.writeUInt16LE(useDeflate ? 8 : 0, 10);
    c.writeUInt16LE(DOS_TIME, 12);
    c.writeUInt16LE(DOS_DATE, 14);
    c.writeUInt32LE(crc, 16);
    c.writeUInt32LE(body.length, 20);
    c.writeUInt32LE(raw.length, 24);
    c.writeUInt16LE(name.length, 28);
    c.writeUInt32LE(offset, 42);
    central.push(c, name);
    offset += local.length + name.length + body.length;
  }
  const centralBuf = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(centralBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, centralBuf, end]);
}

function pack(target: Target, sideload = false): string {
  const dir = join(dist, target);
  const files = walk(dir).map((p) => {
    const name = relative(dir, p).split("\\").join("/");
    // The store build has no `key`; the side-load build has it (so the unpacked extension gets the ID the host's allowed_origins names).
    // Everything else is what was built.
    const data = name === "manifest.json" ? Buffer.from(JSON.stringify(buildManifest(target, "release", { withKey: sideload }), null, 2) + "\n") : readFileSync(p);
    return { name, data };
  });
  if (!files.some((f) => f.name === "manifest.json")) throw new Error(`${dir} is not built; run bun run build first`);
  const out = join(dist, `kin-${target}-${pkg.version}${sideload ? "-sideload" : ""}.zip`);
  writeFileSync(out, zip(files));
  console.log(`${relative(root, out)}  ${files.length} files  ${(statSync(out).size / 1024).toFixed(1)} KiB`);
  return out;
}

if (import.meta.main) {
  pack("chromium");
  pack("chromium", true);
  pack("firefox");
}
