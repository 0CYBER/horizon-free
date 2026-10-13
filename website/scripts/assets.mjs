// Copies the launcher's fonts, background and the repo's screenshots into public/,
// converted for the web (woff2, webp in two widths). Run it after any of them change:
// `npm run assets`. Its output is committed, because Railway builds only website/.
import { mkdir, readFile, writeFile, copyFile } from "node:fs/promises";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";
import wawoff2 from "wawoff2";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const repo = join(site, "..");
const pub = join(site, "public");

const fonts = [
  "Geist-Regular.ttf",
  "Geist-Medium.ttf",
  "Geist-SemiBold.ttf",
  "GeistMono-Regular.ttf",
  "InstrumentSerif-Regular.ttf",
];

// The launcher's screenshots are left out: they show its background, whose licence is
// unconfirmed (see LEGAL.md). [source, name]; every image becomes media/<name>.webp and media/<name>-800.webp.
export const images = [
  ["docs/media/sunset.jpg", "sunset"],
  ["docs/media/village.jpg", "village"],
  ["docs/media/survival.jpg", "survival"],
  ["docs/media/nether.jpg", "nether"],
  ["docs/media/night.jpg", "night"],
  ["docs/media/underwater.jpg", "underwater"],
  ["docs/media/menu-title.png", "menu-title"],
  ["docs/media/menu-worlds.png", "menu-worlds"],
  ["docs/media/menu-options.png", "menu-options"],
  ["docs/media/menu-pause.png", "menu-pause"],
];

await mkdir(join(pub, "fonts"), { recursive: true });
await mkdir(join(pub, "media"), { recursive: true });

for (const font of fonts) {
  const ttf = await readFile(join(repo, "crates/horizon-theme/fonts", font));
  await writeFile(join(pub, "fonts", font.replace(".ttf", ".woff2")), await wawoff2.compress(ttf));
}
for (const licence of ["OFL-Geist.txt", "OFL-InstrumentSerif.txt"]) {
  await copyFile(join(repo, "crates/horizon-theme/fonts", licence), join(pub, "fonts", licence));
}

for (const [source, name] of images) {
  const input = join(repo, source);
  await sharp(input).resize({ width: 1600, withoutEnlargement: true }).webp({ quality: 82 }).toFile(join(pub, "media", `${name}.webp`));
  await sharp(input).resize({ width: 800 }).webp({ quality: 78 }).toFile(join(pub, "media", `${name}-800.webp`));
}

// The atmosphere: the frozen landscape Horizon renders behind its own title screen, cut from
// the part of docs/media/menu-title.png that has no interface on it and blurred further.
// One file, so it can be replaced.
await sharp(join(repo, "docs/media/menu-title.png"))
  .extract({ left: 600, top: 545, width: 1000, height: 400 })
  .resize(1600, 900, { fit: "cover" })
  .blur(10)
  .webp({ quality: 78 })
  .toFile(join(pub, "background.webp"));

// Icons from the project's logo.
await copyFile(join(repo, "docs/media/logo.svg"), join(pub, "favicon.svg"));
await sharp(join(repo, "docs/media/logo.svg")).resize(180).png().toFile(join(pub, "apple-touch-icon.png"));
await sharp(join(repo, "docs/media/logo.svg")).resize(32).png().toFile(join(pub, "favicon-32.png"));

console.log("assets written to public/");
