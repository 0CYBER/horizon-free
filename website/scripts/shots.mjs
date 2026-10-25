// Full-page screenshots of the built site at every width that matters, plus the social
// preview image. Run `npm run build` first; `npm run shots` serves dist/ and writes
// shots/*.png (gitignored: full-page PNGs are too heavy for the repo), 390-menu.png (the open
// menu) and 1440-hero.png, and public/og.png (rebuild after). It also checks each width for
// horizontal scroll, small tap targets and line lengths. `SHOTS=dir` writes them elsewhere.
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const port = 4199;
const url = `http://localhost:${port}/`;
const server = spawn(process.execPath, [join(site, "serve.mjs")], { env: { ...process.env, PORT: String(port) }, stdio: "ignore" });
await new Promise((r) => setTimeout(r, 400));

const widths = [360, 390, 430, 768, 1024, 1180, 1250, 1280, 1366, 1440, 1680, 1920];
const sizes = [...widths.map((w) => [w, w < 900 ? 800 : 900]), [1250, 700]];
const only = process.argv.slice(2);
const out = process.env.SHOTS ? resolve(process.env.SHOTS) : join(site, "shots");

await mkdir(out, { recursive: true });
const browser = await chromium.launch();
let problems = 0;

for (const [width, height] of sizes) {
  const name = height === 700 ? `${width}x${height}` : `${width}`;
  if (only.length && !only.includes(name)) continue;
  const touch = width < 900;
  const page = await browser.newPage({ viewport: { width, height }, deviceScaleFactor: 1, hasTouch: touch, isMobile: touch });
  await page.goto(url, { waitUntil: "networkidle" });
  // Reveal every section, as scrolling would, and let the fades finish.
  await page.evaluate(async () => {
    for (let y = 0; y < document.body.scrollHeight; y += 400) {
      scrollTo({ top: y, behavior: "instant" });
      await new Promise((r) => setTimeout(r, 30));
    }
    scrollTo({ top: 0, behavior: "instant" });
  });
  await page.waitForTimeout(800);
  const report = await page.evaluate((touch) => {
    const out = [];
    const doc = document.documentElement;
    if (doc.scrollWidth > innerWidth) out.push(`horizontal scroll: ${doc.scrollWidth} > ${innerWidth}`);
    for (const el of document.querySelectorAll("body *")) {
      const r = el.getBoundingClientRect();
      if (r.width && r.right > innerWidth + 1 && !el.closest(".lightbox, .menu, .hero-media, .footer-word")) {
        out.push(`overflows: ${el.tagName.toLowerCase()}.${el.className} right ${Math.round(r.right)}`);
      }
    }
    if (touch) {
      for (const el of document.querySelectorAll("a, button, summary")) {
        if (el.closest(".menu, .lightbox, .skip") || el.closest(".prose, .footer-cols, .fact p, .system-detail, .download-meta, .meta")) continue;
        const r = el.getBoundingClientRect();
        if (r.width && r.height < 44) out.push(`small target: ${el.textContent.trim().slice(0, 30)} (${Math.round(r.height)}px)`);
      }
    }
    // Characters per line of the longest running-text paragraphs.
    const lengths = [...document.querySelectorAll(".prose p, .lede, .hero-line, .fact p, .features p, .system-detail, .count-label, .footer-cols p")]
      .filter((p) => p.textContent.length > 160)
      .map((p) => {
        const lh = parseFloat(getComputedStyle(p).lineHeight);
        const n = Math.round(p.textContent.length / Math.max(1, Math.round(p.getBoundingClientRect().height / lh)));
        return [n, `${p.parentElement.id || p.parentElement.className}>${p.className || p.tagName}`];
      })
      .sort((a, b) => a[0] - b[0]);
    const [lo, hi] = [lengths[0], lengths[lengths.length - 1]];
    out.push(`chars per line: ${lo[0]} (${lo[1]})–${hi[0]} (${hi[1]})`);
    return out.slice(0, 12);
  }, touch);
  const issues = report.filter((l) => !l.startsWith("chars"));
  problems += issues.length;
  console.log(`${name}: ${issues.length ? issues.join("; ") : "ok"} · ${report.find((l) => l.startsWith("chars"))}`);
  await page.screenshot({ path: join(out, `${name}.png`), fullPage: height !== 700 });
  if (width === 1440) await page.screenshot({ path: join(out, "1440-hero.png") });
  if (width === 390) {
    await page.click(".menu-button");
    await page.waitForTimeout(800);
    await page.screenshot({ path: join(out, "390-menu.png") });
  }
  await page.close();
}

if (!only.length || only.includes("og")) {
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
  await page.goto(`${url}og.html`, { waitUntil: "networkidle" });
  await page.waitForTimeout(700);
  await page.screenshot({ path: join(site, "public/og.png") });
  await page.close();
}

await browser.close();
server.kill();

if (problems) {
  console.log(`${problems} problems`);
  process.exitCode = 1;
}
