// Copies the launcher's fonts and the repo's screenshots into public/, draws the pixel art,
// converted for the web (woff2, webp in two widths). Run it after any of them change:
// `npm run assets`. Its output is committed, because Railway builds only website/.
import { mkdir, readFile, writeFile, copyFile } from "node:fs/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";
import wawoff2 from "wawoff2";
import { writeArt } from "./art.mjs";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const repo = join(site, "..");
const pub = join(site, "public");

const fonts = [
  "Geist-Regular.ttf",
  "Geist-Medium.ttf",
  "Geist-SemiBold.ttf",
  "GeistMono-Regular.ttf",
  "PixelifySans-Medium.ttf",
  "PixelifySans-SemiBold.ttf",
  "PixelifySans-Bold.ttf",
];

// [source, name]; every image becomes media/<name>.webp and media/<name>-800.webp.
export const images = [
  ["docs/media/sunset.jpg", "sunset"],
  ["docs/media/village.jpg", "village"],
  ["docs/media/survival.jpg", "survival"],
  ["docs/media/nether.jpg", "nether"],
  ["docs/media/night.jpg", "night"],
  ["docs/media/underwater.jpg", "underwater"],
  ["docs/media/settings-simple.png", "settings"],
  ["docs/media/launcher-play.png", "launcher"],
];

await mkdir(join(pub, "fonts"), { recursive: true });
await mkdir(join(pub, "media"), { recursive: true });

for (const font of fonts) {
  const ttf = await readFile(join(repo, "crates/horizon-theme/fonts", font));
  await writeFile(join(pub, "fonts", font.replace(".ttf", ".woff2")), await wawoff2.compress(ttf));
}
for (const licence of ["OFL-Geist.txt", "OFL-PixelifySans.txt"]) {
  await copyFile(join(repo, "crates/horizon-theme/fonts", licence), join(pub, "fonts", licence));
}

for (const [source, name] of images) {
  const input = join(repo, source);
  await sharp(input).resize({ width: 1600, withoutEnlargement: true }).webp({ quality: 82 }).toFile(join(pub, "media", `${name}.webp`));
  await sharp(input).resize({ width: 800 }).webp({ quality: 78 }).toFile(join(pub, "media", `${name}-800.webp`));
}

// The stone tile and the hero's landscape, drawn in code (scripts/art.mjs).
await writeArt(join(pub, "media"));

// The icons (favicon.svg, favicon-32.png, apple-touch-icon.png) come from Horizon's mark:
// `cargo run -p xtask -- icons` writes them with the rest of the brand (docs/media/brand).

console.log("assets written to public/");
