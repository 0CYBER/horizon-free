// A dependency-free static server for dist/, for local checks (`npm run dev`).
// Railway uses Caddy instead (see Dockerfile).
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { join, extname, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL(".", import.meta.url)), "dist");
const types = { ".html": "text/html; charset=utf-8", ".css": "text/css", ".js": "text/javascript", ".webp": "image/webp", ".png": "image/png", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".txt": "text/plain; charset=utf-8", ".md": "text/plain; charset=utf-8", ".json": "application/json", ".xml": "application/xml" };
const port = Number(process.env.PORT) || 4173;

createServer(async (req, res) => {
  let path = normalize(decodeURIComponent(new URL(req.url, "http://x").pathname)).replace(/^(\.\.[/\\])+/, "");
  if (path === "/docs/api/") {
    res.writeHead(301, { Location: "/docs/api" });
    return res.end();
  }
  if (path.endsWith("/")) path += "index.html";
  // The server guide is part of the docs now, as the Caddyfile redirects it.
  if (path === "/server" || path === "/server.html") {
    res.writeHead(301, { Location: "/docs/hosting" });
    return res.end();
  }
  if (path === "/docs") path = "/docs/index.html";
  // /privacy, /docs/<page> and /docs/api/<page> are .html files, as the Caddyfile rewrites them.
  if (!extname(path)) path += ".html";
  let file = join(root, path);
  let status = 200;
  try {
    if (!(await stat(file)).isFile()) throw 0;
  } catch {
    file = join(root, "404.html");
    status = 404;
  }
  res.writeHead(status, { "Content-Type": types[extname(file)] || "application/octet-stream" });
  res.end(await readFile(file));
}).listen(port, () => console.log(`http://localhost:${port}`));
