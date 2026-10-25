// node --test src/: the release notes as the site shows them.
import { test } from "node:test";
import assert from "node:assert/strict";
import { notesText } from "./releases.js";

const players = '<h3>Want to play? Download the <a href="https://playhorizon.dev">Horizon launcher</a>, not the files below</h3><p>It installs this version.</p>';

test("the block sending people to the launcher is left out, the notes stay", () => {
  assert.equal(notesText(`${players}\n\n### Fixed\n- A fix (#1)\n`), "### Fixed\n- A fix (#1)\n");
  assert.equal(notesText(`${players}\r\n\r\nRebuilt from the latest main.\r\n`), "Rebuilt from the latest main.\r\n");
  assert.equal(notesText("<p>Other HTML</p>\n\n### New\n- A thing (#2)\n"), "<p>Other HTML</p>\n\n### New\n- A thing (#2)\n");
  assert.equal(notesText(undefined), "");
});
