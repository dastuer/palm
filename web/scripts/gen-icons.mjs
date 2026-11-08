// 生成 PWA 图标（192/512）：深色圆角底 + 爪印。零依赖：手写 PNG 编码。
// 用法：node scripts/gen-icons.mjs
import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// ---- PNG 编码 ----
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}
function encodePNG(w, h, rgba) {
  const sig = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc(h * (1 + w * 4));
  for (let y = 0; y < h; y++) {
    raw[y * (1 + w * 4)] = 0; // filter none
    rgba.copy(raw, y * (1 + w * 4) + 1, y * w * 4, (y + 1) * w * 4);
  }
  return Buffer.concat([
    sig,
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ---- 绘制 ----
function drawIcon(size) {
  const px = Buffer.alloc(size * size * 4);
  const S = size;
  const set = (x, y, r, g, b, a) => {
    if (x < 0 || y < 0 || x >= S || y >= S) return;
    const i = (y * S + x) * 4;
    const na = a / 255;
    px[i] = Math.round(px[i] * (1 - na) + r * na);
    px[i + 1] = Math.round(px[i + 1] * (1 - na) + g * na);
    px[i + 2] = Math.round(px[i + 2] * (1 - na) + b * na);
    px[i + 3] = Math.min(255, px[i + 3] + a);
  };
  const inRounded = (x, y) => {
    const rad = S * 0.2;
    const x0 = Math.min(Math.max(x, rad), S - rad);
    const y0 = Math.min(Math.max(y, rad), S - rad);
    return (x - x0) ** 2 + (y - y0) ** 2 <= rad * rad;
  };
  const inCircle = (x, y, cx, cy, r) => (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
  const inEllipse = (x, y, cx, cy, rx, ry) =>
    ((x - cx) / rx) ** 2 + ((y - cy) / ry) ** 2 <= 1;

  const BG = [15, 17, 21];
  const FG = [91, 140, 255]; // accent
  const AA = 2; // 超采样
  for (let y = 0; y < S; y++) {
    for (let x = 0; x < S; x++) {
      let cov = 0;
      for (let sy = 0; sy < AA; sy++) {
        for (let sx = 0; sx < AA; sx++) {
          const fx = x + (sx + 0.5) / AA;
          const fy = y + (sy + 0.5) / AA;
          if (!inRounded(fx, fy)) continue;
          let hit = false;
          // 主肉垫
          if (inEllipse(fx, fy, S * 0.5, S * 0.66, S * 0.185, S * 0.15)) hit = true;
          // 四趾
          if (inCircle(fx, fy, S * 0.325, S * 0.44, S * 0.062)) hit = true;
          if (inCircle(fx, fy, S * 0.44, S * 0.365, S * 0.066)) hit = true;
          if (inCircle(fx, fy, S * 0.56, S * 0.365, S * 0.066)) hit = true;
          if (inCircle(fx, fy, S * 0.675, S * 0.44, S * 0.062)) hit = true;
          if (hit) cov++;
        }
      }
      const a = Math.round((cov / (AA * AA)) * 255);
      if (inRounded(x, y)) {
        const total = Math.round((cov / (AA * AA)) * 255);
        // 背景（不透明圆角矩形）
        set(x, y, BG[0], BG[1], BG[2], 255);
        // 前景按覆盖度叠加
        if (total > 0) set(x, y, FG[0], FG[1], FG[2], total);
      } else if (a > 0) {
        set(x, y, BG[0], BG[1], BG[2], a);
      }
    }
  }
  return encodePNG(S, S, px);
}

mkdirSync(join(root, "public", "icons"), { recursive: true });
for (const size of [192, 512]) {
  const buf = drawIcon(size);
  const out = join(root, "public", "icons", `icon-${size}.png`);
  writeFileSync(out, buf);
  console.log("wrote", out, buf.length, "bytes");
}
