"""Reading Rust source without a compiler: code with comments and strings blanked, test regions, use trees.

Shared by the layering scanners. Blanking keeps every offset and newline, so a match in the blanked text has the
same line number as in the file.
"""

import re

TOKEN = re.compile(r"//|/\*|(?<![\w])[bc]?r(#*)\"|\"|'")
COMMENT_EDGE = re.compile(r"/\*|\*/")
STRING_REST = re.compile(r'(?:[^"\\]|\\.)*"', re.S)
# `#[cfg(test)]`, `#[cfg(any(test, ...))]`, `#[test]`; not `#[cfg(not(test))]`.
TEST_ATTR = re.compile(r"#\[\s*(?:test\s*\]|cfg\s*\(\s*(?:test\s*\)|any\s*\([^\]]*?\btest\b))")
NOT_TEST = re.compile(r"\bnot\s*\(\s*test\s*\)")
USE = re.compile(r"(?<![\w])(pub(?:\s*\([^)]*\))?\s+)?use\s+([^;]+);")
MOD_DECL = re.compile(r"(?<![\w])(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
INCLUDE = re.compile(r'include!\(\s*"([^"]+)"\s*\)')


def _blank(chars, start, stop):
    for i in range(start, stop):
        if chars[i] != "\n":
            chars[i] = " "


def blank(text):
    """`text` with comments, string and char literals replaced by spaces (newlines kept)."""
    chars, pos, end = list(text), 0, len(text)
    while token := TOKEN.search(text, pos):
        kind, start, pos = token.group(), token.start(), token.end()
        if kind == "//":
            stop = text.find("\n", pos)
            stop = end if stop < 0 else stop
        elif kind == "/*":
            depth, stop = 1, pos
            while depth and (edge := COMMENT_EDGE.search(text, stop)):
                depth, stop = depth + (1 if edge.group() == "/*" else -1), edge.end()
            stop = end if depth else stop
        elif kind == "'":
            # A char literal ('x', '{', '\'', '\u{7b}') is blanked; a lifetime or label ('a) is code.
            if text.startswith("\\", pos):
                close = text.find("'", pos + 2)
                stop = end if close < 0 else close + 1
            elif text.startswith("'", pos + 1):
                stop = pos + 2
            else:
                continue
        elif kind == '"':
            rest = STRING_REST.match(text, pos)
            stop = rest.end() if rest else end
        else:
            close = text.find('"' + token.group(1), pos)
            stop = end if close < 0 else close + 1 + len(token.group(1))
        _blank(chars, start, stop)
        pos = stop
    return "".join(chars)


def line_starts(text):
    starts = [0]
    starts.extend(i + 1 for i, c in enumerate(text) if c == "\n")
    return starts


def line_of(starts, offset):
    """1-based line of `offset`."""
    lo, hi = 0, len(starts)
    while lo + 1 < hi:
        mid = (lo + hi) // 2
        if starts[mid] <= offset:
            lo = mid
        else:
            hi = mid
    return lo + 1


def item_end(code, pos):
    """Where the item starting at `pos` ends: after its `;` or its matching `}`, whichever comes first."""
    depth, i, end = 0, pos, len(code)
    while i < end:
        c = code[i]
        if c == ";" and depth == 0:
            return i + 1, False
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return i + 1, True
            if depth < 0:
                return i, True
        i += 1
    return end, True


def attr_end(code, pos):
    """The offset after the `#[...]` attribute starting at `pos`."""
    depth, i = 0, pos + 1
    while i < len(code):
        if code[i] == "[":
            depth += 1
        elif code[i] == "]":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return len(code)


def test_regions(code):
    """[(start, end)] of the items under a test attribute, and the names of `mod x;` declared under one."""
    regions, test_mods = [], []
    for attr in TEST_ATTR.finditer(code):
        stop = attr_end(code, attr.start())
        if NOT_TEST.search(code, attr.start(), stop):
            continue
        end, braced = item_end(code, stop)
        if not braced and (decl := MOD_DECL.search(code, stop, end)):
            test_mods.append(decl.group(1))
        regions.append((attr.start(), end))
    return regions, test_mods


def in_regions(regions, offset):
    return any(start <= offset < end for start, end in regions)


def split_top(text, sep=","):
    """Split at `sep` outside braces."""
    parts, depth, cur = [], 0, []
    for c in text:
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        if c == sep and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(c)
    parts.append("".join(cur))
    return [p.strip() for p in parts if p.strip()]


def use_leaves(tree, prefix=()):
    """Flatten a use tree into (path segments, local name), the name None for a glob."""
    tree = re.sub(r"\s+", " ", tree).strip()
    if tree.endswith("}") and "{" in tree:
        head, body = tree[: tree.index("{")], tree[tree.index("{") + 1: -1]
        base = prefix + tuple(s.strip() for s in head.split("::") if s.strip())
        out = []
        for part in split_top(body):
            out.extend(use_leaves(part, base))
        return out
    alias = re.fullmatch(r"(.*?)\s+as\s+(\w+)", tree)
    path, name = (alias.group(1), alias.group(2)) if alias else (tree, None)
    segs = prefix + tuple(s.strip() for s in path.split("::") if s.strip())
    if segs and segs[-1] == "*":
        return [(segs[:-1], None)]
    if segs and segs[-1] == "self":
        segs = segs[:-1]
    if not segs:
        return []
    return [(segs, name or segs[-1])]
