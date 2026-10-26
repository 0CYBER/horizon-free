// node --test src/: the Markdown the docs pages and release notes are rendered from.
import { test } from "node:test";
import assert from "node:assert/strict";
import { markdown } from "./markdown.js";

test("a table renders with its header, cells and inline Markdown", () => {
  const html = markdown("| Way in | Status |\n| --- | :-: |\n| Rust | `cargo horizon-mod` |\n| Lua | planned \\| soon |\n\nAfter.");
  assert.equal(
    html,
    '<div class="table-wrap"><table class="md-table"><thead><tr><th>Way in</th><th>Status</th></tr></thead>' +
      "<tbody><tr><td>Rust</td><td><code>cargo horizon-mod</code></td></tr><tr><td>Lua</td><td>planned | soon</td></tr></tbody></table></div>\n<p>After.</p>",
  );
});

test("a bar inside a code span stays in its cell", () => {
  assert.match(markdown("| a | b |\n|---|---|\n| `x | y` | z |"), /<td><code>x \| y<\/code><\/td><td>z<\/td>/);
});

test("rows without a rule line are a paragraph", () => {
  assert.equal(markdown("| a | b |\n| c | d |"), "<p>| a | b | | c | d |</p>");
});

test("a link whose label is code keeps it", () => {
  assert.equal(markdown("See [`AGENTS.md`](/docs/agents).", { site: true }), '<p>See <a href="/docs/agents"><code>AGENTS.md</code></a>.</p>');
});
