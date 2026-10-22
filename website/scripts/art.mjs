// Horizon's pixel art for the website, drawn in code like the launcher's (launcher/src/art.rs):
// the stone tile behind the pages and the landscape behind the hero, in both appearances.
// Each texel is one pixel of the PNG; the CSS scales them up with `image-rendering: pixelated`.
// Written by `npm run assets` (or `node scripts/art.mjs`) into public/media/.
import { readFile } from "node:fs/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const repo = join(site, "..");

// The same small fixed random sequence as art.rs (xorshift), so the art is the same every time.
function rng(seed) {
  let x = seed >>> 0;
  return () => {
    x = (x ^ (x << 13)) >>> 0;
    x = (x ^ (x >>> 17)) >>> 0;
    x = (x ^ (x << 5)) >>> 0;
    return Math.fround((x >>> 8) / (1 << 24));
  };
}

const channels = (c) => [(c >> 16) & 255, (c >> 8) & 255, c & 255];
const shade = (c, by) => channels(c).reduce((out, v) => (out << 8) | Math.min(255, Math.max(0, v + by)), 0);
const mix = (a, b, t) => {
  const [x, y] = [channels(a), channels(b)];
  return x.reduce((out, v, i) => (out << 8) | Math.round(v + (y[i] - v) * t), 0);
};

// `n` by `n` texels of stone: the page colour in a few close shades, with darker flecks.
export function stone(page, shades, n) {
  const s = shades.map((by) => shade(page, by));
  const next = rng(0x9e3779b9);
  const out = Array.from({ length: n * n }, () => {
    const v = next();
    return s[v < 0.5 ? 0 : v < 0.78 ? 1 : v < 0.93 ? 2 : 3];
  });
  for (let k = 0; k < Math.floor((n * n) / 50); k++) {
    const x = Math.floor(next() * n);
    const y = Math.floor(next() * n);
    const len = 1 + Math.floor(next() * 3);
    for (let i = 0; i < len; i++) {
      out[y * n + ((x + i) % n)] = s[4];
      out[((y + 1) % n) * n + ((x + i + 1) % n)] = s[4];
    }
  }
  return out;
}

export const SUNSET = {
  sky: [0x0a0f2c, 0x26206a, 0x4b2a7e, 0x7c3a88, 0xc4527a, 0xee6e5a, 0xffb25e],
  sun: [0xfff6cf, 0xffd272, 0xff9a44],
  far: 0x4a2f7e, mid: 0x2b1f5e, near: 0x151a45, top: 0x3d2f80, trees: 0x0e1236,
};
export const DAWN = {
  sky: [0x8d7fd8, 0xa596e2, 0xbcaeea, 0xd2c4f0, 0xe7d2ee, 0xf6dce4, 0xffe6d6],
  sun: [0xffffff, 0xfff1d0, 0xffd9a8],
  far: 0xa898dc, mid: 0x8676c8, near: 0x6a5ab4, top: 0x9a8ae0, trees: 0x4c3e94,
};

// A blocky landscape, `w` by `h` texels: banded sky, a sun half set behind the far hills and cut
// into slices like Horizon's mark, flat clouds, three ridges of hills and a few square trees.
export function scene(p, seed, w, h) {
  const next = rng((Math.imul(seed, 0x2545f491) | 1) >>> 0);
  const out = new Array(w * h).fill(0);
  const horizon = Math.floor((h * 62) / 100);
  const band = horizon / p.sky.length;
  for (let y = 0; y < h; y++) {
    const f = y / band;
    const i = Math.min(Math.floor(f), p.sky.length - 1);
    const dither = f - i > 0.88 && i + 1 < p.sky.length;
    for (let x = 0; x < w; x++) out[y * w + x] = dither && (x + y) % 2 === 1 ? p.sky[i + 1] : p.sky[i];
  }
  const rect = (x, y, rw, rh, c) => {
    for (let yy = Math.max(y, 0); yy < Math.min(y + rh, h); yy++)
      for (let xx = Math.max(x, 0); xx < Math.min(x + rw, w); xx++) out[yy * w + xx] = c;
  };
  for (let k = 0; k < 4; k++) {
    const cx = Math.floor(next() * w);
    const cy = Math.floor(horizon * (0.12 + next() * 0.4));
    const cw = 10 + Math.floor(next() * 14);
    const under = p.sky[Math.min(Math.floor(cy / band) + 1, p.sky.length - 1)];
    const c = mix(under, 0xffffff, 0.12);
    rect(cx, cy, cw, 2, c);
    rect(cx + 3, cy - 2, Math.max(cw - 6, 0), 2, c);
  }
  const r = Math.floor(h * 0.2);
  const [cx, cy] = [Math.floor(w * 0.68), horizon - Math.floor(h * 0.11)];
  for (let dy = -r; dy <= 0; dy++) {
    const half = Math.round(Math.sqrt(r * r - dy * dy));
    const t = (dy + r) / r;
    const c = p.sun[t < 0.4 ? 0 : t < 0.75 ? 1 : 2];
    if ((cy + dy) % 5 !== 0 || dy < -r * 0.6) rect(cx - half, cy + dy, half * 2, 1, c);
  }
  const ridge = (base, amp, rough) => {
    let [y, v] = [base, 0];
    return Array.from({ length: w }, (_, x) => {
      if (x % 2 === 0) {
        v = (v + (next() - 0.5) * rough) * 0.86;
        y += v + (base - y) * 0.06;
      }
      return Math.max(Math.round(Math.min(Math.max(y, base - amp), base + amp)), 0);
    });
  };
  const layers = [
    [p.far, horizon - h * 0.1, h * 0.12, 3.2, null],
    [p.mid, horizon + h * 0.02, h * 0.09, 2.6, null],
    [p.near, horizon + h * 0.18, h * 0.07, 2.2, p.top],
  ];
  let near = [];
  for (const [c, base, amp, rough, top] of layers) {
    const hs = ridge(base, amp, rough);
    hs.forEach((y, x) => {
      for (let yy = Math.min(y, h); yy < h; yy++) out[yy * w + x] = c;
      if (top !== null && y < h) out[y * w + x] = top;
    });
    near = hs;
  }
  for (let k = 0; k < Math.floor(w / 26); k++) {
    const tx = 4 + Math.floor(next() * (w - 10));
    const ty = near[tx];
    rect(tx - 2, ty - 8, 5, 5, p.trees);
    rect(tx - 1, ty - 10, 3, 2, p.trees);
    rect(tx, ty - 3, 1, 3, p.trees);
  }
  return out;
}

const png = (texels, w, h, file) =>
  sharp(Buffer.from(texels.flatMap(channels)), { raw: { width: w, height: h, channels: 3 } })
    .png({ compressionLevel: 9, palette: true })
    .toFile(file);

// The page colours and the stone's shades come from crates/horizon-theme, like tokens.css.
export async function writeArt(media) {
  const theme = await readFile(join(repo, "crates/horizon-theme/src/lib.rs"), "utf8");
  const page = (name) => parseInt(theme.match(new RegExp(`pub const ${name}: Blocks = Blocks \\{\\s*page: 0x([0-9a-f]{6})`))[1], 16);
  const shades = JSON.parse(`[${theme.match(/TILE_SHADES: \[i16; 5\] = \[([^\]]*)\]/)[1]}]`);
  const texels = Number(theme.match(/TILE_TEXELS: usize = (\d+)/)[1]) * 4;
  await png(stone(page("BLOCKS_DARK"), shades, texels), texels, texels, join(media, "stone-dark.png"));
  await png(stone(page("BLOCKS_LIGHT"), shades, texels), texels, texels, join(media, "stone-light.png"));
  const [w, h] = [240, 72];
  await png(scene(SUNSET, 3, w, h), w, h, join(media, "scene-dark.png"));
  await png(scene(DAWN, 3, w, h), w, h, join(media, "scene-light.png"));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await writeArt(join(site, "public", "media"));
  console.log("pixel art written to public/media/");
}
