// Writes the image fixtures of protocol/media.json (changelog 57, `image[]`):
//
//   bun protocol/fixtures/make-media-fixtures.ts
//
// Everything is deterministic (fixed colours, fixed zlib level), so running it again writes the same bytes.
//
//   media-art-256.png          256 x 256 PNG, valid (the largest `art`)
//   media-art-257.png          257 x 256 PNG (one pixel too wide for `art`)
//   media-art-16.png           16 x 16 PNG (the smallest accepted)
//   media-art-15.png           15 x 16 PNG (one pixel too narrow)
//   media-icon-128.png         128 x 128 PNG (the largest `icon`)
//   media-icon-129.png         129 x 128 PNG (too wide for `icon`; fine as `art`)
//   media-art-24576.png        256 x 256 PNG padded with a tEXt chunk to exactly 24,576 bytes (the largest accepted)
//   media-art-24577.png        the same, 24,577 bytes (one over)
//   media-art-256.jpg          the HEADERS of a 256 x 256 JPEG (SOI, APP0, SOF0, EOI): enough for the wire checks, which read the magic and the size
//                              only; a real decoder refuses it, which is what the apps' own decoders do with a bad picture (they drop it)
//   media-art-257.jpg          the same for 257 x 256
//   media-art.gif              a GIF header (GIF89a, 256 x 256): not a PNG or JPEG
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));

const crcTable = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (b: Uint8Array) => {
  let c = 0xffffffff;
  for (const x of b) c = crcTable[(c ^ x) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const u32 = (n: number) => Uint8Array.of((n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255);
const cat = (...p: Uint8Array[]) => Uint8Array.from(p.flatMap((x) => [...x]));
const ascii = (s: string) => new TextEncoder().encode(s);
const chunk = (type: string, data: Uint8Array) => {
  const body = cat(ascii(type), data);
  return cat(u32(data.length), body, u32(crc32(body)));
};

function png(w: number, h: number, padTo?: number): Uint8Array {
  const row = cat(Uint8Array.of(0), new Uint8Array(w * 4).map((_, i) => (i % 4 === 0 ? 0x30 : i % 4 === 1 ? 0x60 : i % 4 === 2 ? 0xc0 : 0xff)));
  const raw = cat(...Array.from({ length: h }, () => row));
  const head = cat(Uint8Array.of(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a), chunk("IHDR", cat(u32(w), u32(h), Uint8Array.of(8, 6, 0, 0, 0))));
  const idat = chunk("IDAT", new Uint8Array(deflateSync(raw, { level: 9 })));
  const end = chunk("IEND", new Uint8Array(0));
  let out = cat(head, idat, end);
  if (padTo !== undefined) {
    const fill = padTo - out.length - 12;
    if (fill < 8) throw new Error(`the base image (${out.length} bytes) is too big to pad to ${padTo}`);
    const text = chunk("tEXt", cat(ascii("Comment\0"), new Uint8Array(fill - 8).fill(0x78)));
    out = cat(head, idat, text, end);
    if (out.length !== padTo) throw new Error(`padded to ${out.length}, wanted ${padTo}`);
  }
  return out;
}

// JPEG headers: SOI, an APP0 (JFIF) segment, SOF0 with the size, EOI.
function jpegHeaders(w: number, h: number): Uint8Array {
  const app0 = cat(Uint8Array.of(0xff, 0xe0, 0x00, 0x10), ascii("JFIF\0"), Uint8Array.of(1, 1, 0, 0, 1, 0, 1, 0, 0));
  const sof0 = cat(Uint8Array.of(0xff, 0xc0, 0x00, 0x0b, 8), Uint8Array.of(h >> 8, h & 255, w >> 8, w & 255), Uint8Array.of(1, 1, 0x11, 0));
  return cat(Uint8Array.of(0xff, 0xd8), app0, sof0, Uint8Array.of(0xff, 0xd9));
}

const gif = cat(ascii("GIF89a"), Uint8Array.of(0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00), Uint8Array.of(0x3b));

const files: Record<string, Uint8Array> = {
  "media-art-256.png": png(256, 256),
  "media-art-257.png": png(257, 256),
  "media-art-16.png": png(16, 16),
  "media-art-15.png": png(15, 16),
  "media-icon-128.png": png(128, 128),
  "media-icon-129.png": png(129, 128),
  "media-art-24576.png": png(256, 256, 24576),
  "media-art-24577.png": png(256, 256, 24577),
  "media-art-256.jpg": jpegHeaders(256, 256),
  "media-art-257.jpg": jpegHeaders(257, 256),
  "media-art.gif": gif,
};
for (const [name, bytes] of Object.entries(files)) {
  writeFileSync(join(dir, name), bytes);
  console.log(`${name}: ${bytes.length} bytes`);
}
