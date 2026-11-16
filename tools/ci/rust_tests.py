"""A small reader of Rust source for CI's test checks: functions, their attributes, and what their bodies call.

Not a parser. It blanks comments and the contents of string and character literals (keeping every line break, so
offsets and line numbers stay those of the file), then finds each `fn NAME` with a body, the attributes written
just before it, and its body's text. Good enough to tell a `#[test]` function from a helper, see whether it is
`#[ignore]`d, and find the names it calls; it does not expand macros, resolve paths or types, or follow calls
into other files. Used by tools/ci/check_game_boot_tests.py and tools/ci/move_to_daily.py.
"""

import re
from typing import NamedTuple

# What `blank` acts on: a comment, a raw string (group 1: its hashes), a string, a quote (character or lifetime).
TOKEN = re.compile(r"""//|/\*|(?<!\w)[bc]?r(#*)"|(?<!\w)[bc]?"|"|'""")
NOT_NEWLINE = re.compile(r"[^\n]")
BRACES = re.compile(r"[{}]")
FN = re.compile(r"\bfn\s+(?:r#)?([A-Za-z_]\w*)")
ATTRIBUTE = re.compile(r"#\s*\[\s*((?:\w+\s*::\s*)*\w+)")
# The attributes that make a function a test: `#[test]`, and the `#[tokio::test]`-style ones of async runtimes.
# The daily run's mark, and the literal `functions` must keep for `Function.is_daily` to see it.
DAILY_MARK = re.compile(r'#\s*\[\s*ignore\s*=\s*"daily"\s*\]')
DAILY_KEEP = frozenset({"daily"})
TEST_ATTRIBUTES = re.compile(r"^(?:test|(?:\w+::)+test)$")


class Function(NamedTuple):
    name: str
    line: int  # 1-based line of the `fn` keyword
    attributes: tuple[str, ...]  # attribute paths written before it, outermost first (`test`, `ignore`, `cfg_attr`)
    attribute_text: str  # the attributes' blanked text, for `cfg_attr(..., ignore)`
    body: str  # the blanked text between the body's braces

    @property
    def is_test(self):
        return any(TEST_ATTRIBUTES.match(path) for path in self.attributes)

    @property
    def is_ignored(self):
        """An unconditional `#[ignore]`. A conditional one (`#[cfg_attr(windows, ignore)]`) does not count: the test
        still runs in some configuration, so the checks treat it as not ignored."""
        return "ignore" in self.attributes

    @property
    def is_daily(self):
        """Marked `#[ignore = "daily"]` (read from `functions(source, keep=DAILY_KEEP)`, which keeps that literal)."""
        return bool(DAILY_MARK.search(self.attribute_text))


def blank(source, keep=frozenset()):
    """`source` with comments and literal contents replaced by spaces, line breaks kept.

    A string literal whose whole content is in `keep` stays as written, so a check can still see it."""
    out = list(source)
    i, n = 0, len(source)

    def wipe(start, end):
        out[start:end] = NOT_NEWLINE.sub(" ", source[start:end])

    while (token := TOKEN.search(source, i)):
        i = token.start()
        c = source[i]
        if source.startswith("//", i):
            end = source.find("\n", i)
            end = n if end < 0 else end
            wipe(i, end)
            i = end
        elif source.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if source.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif source.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            wipe(i, j)
            i = j
        elif token[1] is not None:
            close = '"' + token[1]
            start = token.end()
            end = source.find(close, start)
            end = n if end < 0 else end
            if source[start:end] not in keep:
                wipe(start, end)
            i = end + len(close)
        elif c != "'":
            start = token.end()
            j = start
            while j < n and source[j] != '"':
                j += 2 if source[j] == "\\" else 1
            if source[start:j] not in keep:
                wipe(start, j)
            i = j + 1
        elif c == "'":
            # A character literal ('a', '\n', '\u{1F600}', b'x'), or a lifetime or label ('a, 'static), which stays.
            if source.startswith("\\", i + 1):
                end = source.find("'", i + 3)
                end = n if end < 0 else end
                wipe(i + 1, end)
                i = end + 1
            elif i + 2 < n and source[i + 2] == "'":
                wipe(i + 1, i + 2)
                i += 3
            else:
                i += 1
    return "".join(out)


def matching_brace(text, start):
    """The index just past the brace that closes the `{` at `start` (len(text) if it never closes)."""
    depth = 0
    for brace in BRACES.finditer(text, start):
        depth += 1 if brace[0] == "{" else -1
        if not depth:
            return brace.end()
    return len(text)


def functions(source, keep=frozenset()):
    """Every function with a body in `source`, nested ones included (a nested one's text is also its parent's)."""
    text = blank(source, keep)
    found = []
    for match in FN.finditer(text):
        # The body starts at the first `{` after the signature; a `;` first means a declaration without one.
        depth = 0
        for brace in range(match.end(), len(text)):
            ch = text[brace]
            if ch == ">" and text[brace - 1] == "-":
                continue  # `->`, not a closing angle bracket
            if ch in "(<[":
                depth += 1
            elif ch in ")>]" and depth:
                depth -= 1
            elif ch == ";" and not depth:
                brace = -1
                break
            elif ch == "{" and not depth:
                break
        else:
            brace = -1
        if brace < 0:
            continue
        end = matching_brace(text, brace)
        # The attributes: back from the `fn` (over qualifiers like `pub(crate) async unsafe`) to the end of the
        # previous item or the start of the enclosing block.
        start = match.start()
        k = start - 1
        bracket = 0
        while k >= 0:
            ch = text[k]
            if ch == "]":
                bracket += 1
            elif ch == "[":
                bracket -= 1
            elif ch in "{};" and not bracket:
                break
            k -= 1
        prefix = text[k + 1:start]
        attributes = tuple(re.sub(r"\s+", "", path) for path in ATTRIBUTE.findall(prefix))
        found.append(Function(match[1], text.count("\n", 0, match.start()) + 1, attributes, prefix,
                              text[brace + 1:end - 1]))
    return found
