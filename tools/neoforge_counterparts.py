#!/usr/bin/env python3
"""mdBook preprocessor: render declared NeoForge counterparts without a source snapshot."""
import json
from pathlib import Path
import re
import sys

from modding_hook_notes import expand

PAGE = "reference/neoforge-counterparts.md"


def string_end(text, i):
    """The index just past the string that starts at `i` (`"`)."""
    i += 1
    while i < len(text):
        if text[i] == "\\":
            i += 2
            continue
        if text[i] == '"':
            return i + 1
        i += 1
    raise AssertionError("unterminated string")


def raw_string_end(text, i):
    """The index just past the raw string at `i` (`r"..."`, `r#"..."#`), or None if it is not one."""
    j = i + 1
    while j < len(text) and text[j] == "#":
        j += 1
    if j >= len(text) or text[j] != '"':
        return None
    close = '"' + "#" * (j - i - 1)
    k = text.find(close, j + 1)
    assert k != -1, "unterminated raw string"
    return k + len(close)


def comment_end(text, i):
    """The index just past the `//` or `/* ... */` comment at `i`, or None if it is not a comment."""
    if text.startswith("//", i):
        k = text.find("\n", i)
        return len(text) if k < 0 else k + 1
    if text.startswith("/*", i):
        depth = 1
        j = i + 2
        while j < len(text) and depth:
            if text.startswith("/*", j):
                depth += 1
                j += 2
            elif text.startswith("*/", j):
                depth -= 1
                j += 2
            else:
                j += 1
        assert depth == 0, "unterminated block comment"
        return j
    return None


def attribute_end(text, start):
    """The index just past the `]` closing the attribute at `start` (`#[...]`)."""
    i = start + 2
    depth = 1
    while i < len(text):
        end = comment_end(text, i)
        if end is not None:
            i = end
            continue
        c = text[i]
        if c == '"':
            i = string_end(text, i)
            continue
        if c == "r":
            end = raw_string_end(text, i)
            if end is not None:
                i = end
                continue
        if c == "[":
            depth += 1
        elif c == "]":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    raise AssertionError(f"unterminated attribute at {start}")


def skip_trivia(text, i):
    """Skip whitespace, attributes and comments, the way Rust allows before an item."""
    while True:
        while i < len(text) and text[i].isspace():
            i += 1
        if text.startswith("#[", i):
            i = attribute_end(text, i)
            continue
        end = comment_end(text, i)
        if end is not None:
            i = end
            continue
        return i


def without_comments(text):
    """`text` with `//` and `/* ... */` comments dropped; strings are kept."""
    out = []
    i = 0
    while i < len(text):
        end = comment_end(text, i)
        if end is not None:
            out.append(" ")
            i = end
            continue
        if text[i] == '"':
            end = string_end(text, i)
            out.append(text[i:end])
            i = end
            continue
        if text[i] == "r":
            end = raw_string_end(text, i)
            if end is not None:
                out.append(text[i:end])
                i = end
                continue
        out.append(text[i])
        i += 1
    return "".join(out)


def hook_spans(text):
    """Yield `(start, end)` for each `#[hook(...)]` at item level, never inside another attribute."""
    i = 0
    while i < len(text):
        if text.startswith("#[", i):
            end = attribute_end(text, i)
            if text.startswith("#[hook(", i):
                yield i, end
            i = end
            continue
        end = comment_end(text, i)
        if end is not None:
            i = end
            continue
        if text[i] == '"':
            i = string_end(text, i)
            continue
        if text[i] == "r":
            end = raw_string_end(text, i)
            if end is not None:
                i = end
                continue
        i += 1


def hooks(root):
    rows = []
    for path in sorted((root / "crates/horizon-mod-vocab/src/hooks").glob("*.rs")):
        if path.name == "mod.rs":
            continue
        text = path.read_text(encoding="utf8")
        for start, end in hook_spans(text):
            attr = without_comments(text[start + len("#[hook("):end - 2])
            j = skip_trivia(text, end)
            name = re.match(r"fn\s+(\w+)", text[j:])

            def key(k):
                v = re.search(k + r'\s*=\s*"([^"]*)"', attr)
                return v.group(1) if v else None

            neo = key("neoforge")
            if neo is None:
                lst = re.search(r"neoforge\s*=\s*\[(.*?)\]", attr, re.S)
                if lst:
                    neo = ", ".join(re.findall(r'"([^"]*)"', lst.group(1)))
            if name:
                rows.append((name.group(1), key("side") or "", neo))
    return sorted(rows)


def table(rows, head):
    out = [f"| Hook | Side | {head} |", "| --- | --- | --- |"]
    out += [f"| `{n}` | {s} | {c} |" for n, s, c in rows]
    return "\n".join(out)


def page(root):
    rows = hooks(root)
    mapped = [(n, s, f"`{c}`") for n, s, c in rows if c and not c.lower().startswith("none")]
    none = [
        (n, s, re.sub(r"^none\s*\(?(.*?)\)?$", r"\1", c, flags=re.S).strip() or "none")
        for n, s, c in rows
        if c and c.lower().startswith("none")
    ]
    return f"""# NeoForge counterparts

Each hook's `neoforge = ...` key in `crates/horizon-mod-vocab/src/hooks/` names the NeoForge 26.3 event or method it
matches, since a counterpart's name and contract are the rule for a hook. This page lists the keys as declared.
The book build renders it directly from those declarations; there is no generated Markdown snapshot to update.
A key names the event class (`PlayerEvent.Clone`) or runtime callback (`IPayloadHandler.handle`);
notes about cancelling or the event bus belong in the hook's `///` doc, not in the key.

{len(rows)} hooks: {len(mapped)} name a counterpart, {len(none)} say they have none.
This is a declaration reference, not an independent verification of every mapping. Registration events and the
callbacks they register are different moments: packet receipt names `IPayloadHandler.handle`, and per-draw item
decoration names `IItemDecorator.render`, not their registration events. A `none` explains why no event or method
matches that hook's runtime contract.

## Hooks with a counterpart

{table(mapped, "NeoForge")}

## Hooks that declare no counterpart

A hook with no NeoForge counterpart is judged on its merits like any other. The note is the key's own reason.

{table(none, "Reason")}
"""


def preprocess(context, book):
    root = Path(context["root"]).resolve().parents[1]
    return expand(book, page(root), "NeoForge counterparts", PAGE)


if __name__ == "__main__" and sys.argv[1:2] != ["supports"]:
    print(json.dumps(preprocess(*json.load(sys.stdin))))
