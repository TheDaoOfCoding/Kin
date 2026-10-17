// Placeholder icons: an indigo rounded square with a white "K". Real artwork comes later.
// Writes assets/icons/icon-{16,32,48,128}.png (committed, so a build needs no generation).
import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

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
function chunk(type: string, data: Uint8Array): Buffer {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, "ascii");
  out.set(data, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}
function png(size: number, rgba: Uint8Array): Buffer {
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y++) {
    raw[y * (size * 4 + 1)] = 0;
    raw.set(rgba.subarray(y * size * 4, (y + 1) * size * 4), y * (size * 4 + 1) + 1);
  }
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw, { level: 9 })), chunk("IEND", new Uint8Array())]);
}

function distToSegment(px: number, py: number, ax: number, ay: number, bx: number, by: number): number {
  const dx = bx - ax;
  const dy = by - ay;
  const t = Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)));
  return Math.hypot(px - (ax + t * dx), py - (ay + t * dy));
}

/** Shape coverage in unit coordinates: 0 outside, 1 = background, 2 = the letter. */
function sample(x: number, y: number): 0 | 1 | 2 {
  const r = 0.22;
  const cx = Math.min(Math.max(x, r), 1 - r);
  const cy = Math.min(Math.max(y, r), 1 - r);
  if (Math.hypot(x - cx, y - cy) > r) return 0;
  const bar = x >= 0.29 && x <= 0.4 && y >= 0.22 && y <= 0.78;
  const upper = distToSegment(x, y, 0.4, 0.52, 0.7, 0.22) <= 0.055;
  const lower = distToSegment(x, y, 0.45, 0.46, 0.72, 0.78) <= 0.055;
  return bar || upper || lower ? 2 : 1;
}

function render(size: number): Buffer {
  const ss = 4;
  const px = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      let a = 0;
      let letter = 0;
      for (let sy = 0; sy < ss; sy++) {
        for (let sx = 0; sx < ss; sx++) {
          const s = sample((x + (sx + 0.5) / ss) / size, (y + (sy + 0.5) / ss) / size);
          if (s > 0) a++;
          if (s === 2) letter++;
        }
      }
      const n = ss * ss;
      const t = y / size;
      const bg = [79 + 45 * t, 70 + 20 * t, 229 - 10 * t]; // indigo to violet
      const mix = a === 0 ? 0 : letter / a;
      const o = (y * size + x) * 4;
      px[o] = Math.round(bg[0]! * (1 - mix) + 255 * mix);
      px[o + 1] = Math.round(bg[1]! * (1 - mix) + 255 * mix);
      px[o + 2] = Math.round(bg[2]! * (1 - mix) + 255 * mix);
      px[o + 3] = Math.round((a / n) * 255);
    }
  }
  return png(size, px);
}

const dir = new URL("../assets/icons/", import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1");
mkdirSync(dir, { recursive: true });
for (const size of [16, 32, 48, 128]) writeFileSync(join(dir, `icon-${size}.png`), render(size));
console.log("icons written to", dir);
