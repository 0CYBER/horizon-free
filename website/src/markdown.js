// A small Markdown renderer for release notes, the README and the docs: headings, paragraphs,
// lists (nested by indentation), fenced code, quotes, rules, tables, and inline code, bold,
// italic and links. Everything is escaped first, so its output never carries raw HTML.
// Shared by build.mjs (Node) and main.js (browser).

const REPO_BLOB = "https://github.com/ludovic111/horizon/blob/main/";

export function escape(text) {
  return text.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

function href(url, site) {
  url = url.trim();
  if (/^(https?:|mailto:)/i.test(url)) return url;
  // The docs pages (`site`) link each other and their own headings; the README's anchors have no target here.
  if (site && (/^\/(?!\/)/.test(url) || url.startsWith("#"))) return url;
  if (url.startsWith("#")) return null;
  if (/^[a-z]+:/i.test(url)) return null; // javascript: and the like
  return REPO_BLOB + url.replace(/^\.?\//, "");
}

export function inline(text, site = false) {
  const codes = [];
  let out = text.replace(/`([^`]+)`/g, (_, code) => {
    codes.push(code);
    return `\u0000${codes.length - 1}\u0000`;
  });
  const links = [];
  out = out.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (_, label, url) => {
    links.push([label, url]);
    return `\u0001${links.length - 1}\u0001`;
  });
  out = escape(out)
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^\w*])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>")
    .replace(/(^|[^\w])_([^_\s][^_]*)_(?!\w)/g, "$1<em>$2</em>")
    .replace(/&lt;(https?:\/\/[^\s&]+)&gt;/g, (_, u) => `<a href="${u}">${u}</a>`);
  out = out.replace(/\u0001(\d+)\u0001/g, (_, i) => {
    const [label, url] = links[i];
    const target = href(url, site);
    // The label's code spans are already placeholders: put them back for its own pass.
    const body = inline(label.replace(/\u0000(\d+)\u0000/g, (_, c) => `\`${codes[c]}\``), site);
    return target ? `<a href="${escape(target)}">${body}</a>` : body;
  });
  return out.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${escape(codes[i])}</code>`);
}

// A table: a row of cells, a rule row (`| --- | :-: |`), then body rows, until a line that isn't a row.
const ROW = /^\s*\|.*\|\s*$/;
const RULE = /^\s*\|(\s*:?-+:?\s*\|)+\s*$/;

/** A row's cells: split on `|` outside code spans; `\|` is a literal bar. */
function cells(row) {
  const s = row.trim().slice(1, -1);
  const out = [""];
  let code = false;
  for (let i = 0; i < s.length; i++) {
    if (s[i] === "\\" && s[i + 1] === "|") out[out.length - 1] += s[++i];
    else if (s[i] === "|" && !code) out.push("");
    else {
      if (s[i] === "`") code = !code;
      out[out.length - 1] += s[i];
    }
  }
  return out.map((c) => c.trim());
}

function table(lines, start, site) {
  const row = (line, tag) => `<tr>${cells(line).map((c) => `<${tag}>${inline(c, site)}</${tag}>`).join("")}</tr>`;
  let i = start + 2;
  const body = [];
  while (i < lines.length && ROW.test(lines[i])) body.push(row(lines[i++], "td"));
  return [`<div class="table-wrap"><table class="md-table"><thead>${row(lines[start], "th")}</thead><tbody>${body.join("")}</tbody></table></div>`, i];
}

const indentOf = (line) => line.match(/^ */)[0].length;
const ITEM = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/;

function list(lines, start, site) {
  // Collects the items at this list's indentation; deeper lines belong to the item above.
  const base = indentOf(lines[start]);
  const ordered = /\d/.test(lines[start].match(ITEM)[2]);
  const items = [];
  let i = start;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      const next = lines.slice(i + 1).find((l) => l.trim());
      if (next && ITEM.test(next) && indentOf(next) >= base) {
        i++;
        continue;
      }
      break;
    }
    const m = line.match(ITEM);
    if (m && indentOf(line) === base) {
      items.push({ text: [m[3]], children: [] });
    } else if (m && indentOf(line) > base) {
      const [html, end] = list(lines, i, site);
      items[items.length - 1].children.push(html);
      i = end;
      continue;
    } else if (indentOf(line) > base && items.length) {
      items[items.length - 1].text.push(line.trim());
    } else {
      break;
    }
    i++;
  }
  const tag = ordered ? "ol" : "ul";
  const body = items.map((it) => `<li>${inline(it.text.join(" "), site)}${it.children.join("")}</li>`).join("");
  return [`<${tag}>${body}</${tag}>`, i];
}

const slug = (text) =>
  text
    .toLowerCase()
    .replace(/`/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

/**
 * Renders Markdown; `shift` moves headings down (a `#` becomes `h{1 + shift}`).
 * `site` is for the docs pages: links to the site's own pages and anchors stay as they are, `##` and `###`
 * headings get an id (and are pushed onto `headings`, for a table of contents), and a line holding only
 * `![caption](/media/…)` becomes a figure.
 */
export function markdown(source, { shift = 0, site = false, headings = null } = {}) {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim() || /^\s*<\/?[a-z][^>]*>\s*$/i.test(line)) {
      i++; // blank lines and lone HTML tags (the README's layout tables) are skipped
      continue;
    }
    let m;
    if ((m = line.match(/^(#{1,6})\s+(.*?)\s*#*$/))) {
      const level = Math.min(6, m[1].length + shift);
      let id = "";
      if (site && (m[1].length === 2 || m[1].length === 3)) {
        const base = slug(m[2]);
        id = base;
        for (let n = 2; headings && headings.some((h) => h.id === id); n++) id = `${base}-${n}`;
        headings?.push({ level: m[1].length, text: m[2].replace(/`/g, ""), id });
      }
      out.push(`<h${level}${id ? ` id="${id}"` : ""}>${inline(m[2], site)}</h${level}>`);
      i++;
    } else if (site && (m = line.match(/^!\[([^\]]*)\]\((\/media\/[\w./-]+)\)\s*$/))) {
      out.push(`<figure><img src="${escape(m[2])}" alt="${escape(m[1])}" loading="lazy" decoding="async"><figcaption>${inline(m[1], true)}</figcaption></figure>`);
      i++;
    } else if ((m = line.match(/^\s*(```|~~~)/))) {
      const fence = m[1];
      const code = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence)) code.push(lines[i++]);
      i++;
      out.push(`<pre><code>${escape(code.join("\n"))}</code></pre>`);
    } else if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      out.push("<hr>");
      i++;
    } else if (ROW.test(line) && RULE.test(lines[i + 1] || "")) {
      const [html, end] = table(lines, i, site);
      out.push(html);
      i = end;
    } else if (ITEM.test(line)) {
      const [html, end] = list(lines, i, site);
      out.push(html);
      i = end;
    } else if (line.startsWith(">")) {
      const quote = [];
      while (i < lines.length && lines[i].startsWith(">")) quote.push(lines[i++].replace(/^>\s?/, ""));
      out.push(`<blockquote>${markdown(quote.join("\n"), { shift, site })}</blockquote>`);
    } else {
      const para = [];
      while (
        i < lines.length &&
        lines[i].trim() &&
        !/^(#{1,6}\s|>|\s*```|\s*~~~)/.test(lines[i]) &&
        !ITEM.test(lines[i])
      ) {
        para.push(lines[i++].trim());
      }
      out.push(`<p>${inline(para.join(" "), site)}</p>`);
    }
  }
  return out.join("\n");
}

/** The body of the section titled `title` (any heading level), up to the next heading at its level or above. */
export function section(source, title) {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const level = (l) => (l.match(/^(#{1,6})\s/) || [, ""])[1].length;
  const start = lines.findIndex((l) => level(l) > 0 && l.replace(/^#+/, "").trim() === title);
  if (start < 0) return null;
  const depth = level(lines[start]);
  const end = lines.findIndex((l, i) => i > start && level(l) > 0 && level(l) <= depth);
  return lines
    .slice(start + 1, end < 0 ? undefined : end)
    .join("\n")
    .trim();
}
