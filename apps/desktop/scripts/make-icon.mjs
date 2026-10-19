// Draws the 1024px source icon (a slanted manhwa panel on a violet tile) as a PNG with no dependencies.
// Regenerate the platform icons with: node scripts/make-icon.mjs && npx tauri icon app-icon.png
import { writeFileSync } from "node:fs";
import { deflateSync } from "node:zlib";

const S = 1024;
const px = Buffer.alloc(S * S * 4);

const inRoundedSquare = (x, y, r, inset) => {
  const lo = inset + r;
  const hi = S - inset - r;
  const cx = Math.min(Math.max(x, lo), hi);
  const cy = Math.min(Math.max(y, lo), hi);
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
};

// Panel: a quadrilateral with a slanted bottom edge, like a manhwa action panel.
const panel = [
  [250, 260],
  [774, 260],
  [774, 650],
  [250, 770],
];
const inPolygon = (x, y, poly) => {
  let inside = false;
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i];
    const [xj, yj] = poly[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
};
const shrink = (poly, d) => poly.map(([x, y]) => [x + (x < 512 ? d : -d), y + (y < 512 ? d : -d)]);
const inner = shrink(panel, 26);

for (let y = 0; y < S; y++) {
  for (let x = 0; x < S; x++) {
    const i = (y * S + x) * 4;
    if (!inRoundedSquare(x, y, 200, 40)) continue;
    const t = (x + y) / (2 * S);
    let [r, g, b] = [Math.round(92 - 40 * t), Math.round(54 - 20 * t), Math.round(220 - 60 * t)];
    if (inPolygon(x, y, panel)) [r, g, b] = [250, 248, 255];
    if (inPolygon(x, y, inner)) {
      // Speed-line burst inside the panel.
      const a = Math.atan2(y - 500, x - 512);
      const ray = Math.floor(((a + Math.PI) / (2 * Math.PI)) * 36) % 2 === 0;
      [r, g, b] = ray ? [255, 72, 112] : [32, 18, 48];
    }
    px[i] = r;
    px[i + 1] = g;
    px[i + 2] = b;
    px[i + 3] = 255;
  }
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc32 = (buf) => {
  let c = 0xffffffff;
  for (const byte of buf) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
};

const raw = Buffer.alloc((S * 4 + 1) * S);
for (let y = 0; y < S; y++) px.copy(raw, y * (S * 4 + 1) + 1, y * S * 4, (y + 1) * S * 4);
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(S, 0);
ihdr.writeUInt32BE(S, 4);
ihdr.set([8, 6, 0, 0, 0], 8);
writeFileSync(
  new URL("../app-icon.png", import.meta.url),
  Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]),
);
console.log("wrote app-icon.png");
