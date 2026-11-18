#!/usr/bin/env python3
"""List what a set of functions uses on `Game` (crates/horizon-client), before they are moved out of `impl Game`.

Moving logic out of the large `impl Game` blocks needs a trait or borrow struct in a lower crate that carries exactly
what the functions touch. For each function, or line range, this prints:
- the `Game` fields read and written (`self.field`, `self.field.x`, `&mut self.field`), with counts and lines, and the
  `impl Game` methods it calls (`self.method(`, `Self::assoc(`), each with where it is defined and whether it is a mod
  point candidate;
- the free functions, types and constants it names, split into horizon-client and other crates, by `use` resolution;
- the markers that make order matter: random draws, entity ids, spawns, hook fires, sends, packet and event pushes;
- with --transitive N, the same field uses of the Game methods it calls, N levels down.

It reads the source as text: comments and strings are blanked, braces are matched, and `self.x` is matched by regex.
There is no type information, so:
- a field counts as written when it is assigned (`=`, `+=`), taken as `&mut`, or has a mutating method called on it
  (`push`, `insert`, `take`, `get_mut`, ...; the list is MUTATORS). A method not on the list reads as a read;
- uses through another name for the game (a closure parameter in `self.as_guest(conn, |game| ...)`, a `let g = self`)
  are not attributed unless you pass --alias game. A bare `self` handed on (`helper(self)`, `&mut *self`) is listed
  under "whole self": its uses are those of the callee;
- "mod point candidate" mimics point_shape::eligible (crates/horizon-mod-point-shape/src/lib.rs) from the signature
  and attributes: a `&self` or `&mut self` receiver, no `native_*` name, no const, async, unsafe or extern, no
  `cfg`, `cfg_attr`, `test` or `inline(always)` attribute (also on the impl or an enclosing inline module, or a
  gated `mod x;`), no `-> !` or `-> impl`, no `impl` parameter type, no parameter named `cancel` or `result`.
  It does not check that the parameter types are ones the API can carry, nor `#[replaceable]` marks;
- `use` resolution follows explicit imports and `super::*`/`crate::m::*` globs textually, then falls back to a name
  index of every crate, marked "by name" when it is a guess.

    python3 tools/game_uses.py crates/horizon-client/src/game/mobs/creatures/mod.rs::handle_mob_events_pending
    python3 tools/game_uses.py game/administration/commands/difficulty.rs::cmd_difficulty --transitive 1
    python3 tools/game_uses.py game/mobs/creatures/mod.rs:2208-2400 --json
    python3 tools/game_uses.py cmd_difficulty        # a bare name, if one Game method has it

A spec is `FILE::NAME[,NAME...]` (FILE is a path from the repository root, or the end of one under
crates/horizon-client/src), `FILE:FIRST-LAST` for a line range, or a bare method name.
"""

import argparse
import bisect
from collections import defaultdict
import json
import os
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
CRATES = ROOT / "crates"
CLIENT = CRATES / "horizon-client"
SRC = CLIENT / "src"

# Methods that change the value they are called on (receiver `&mut self`), so `self.field.push(x)` is a write.
MUTATORS = frozenset("""
push push_back push_front push_str pop pop_back pop_front insert remove clear extend append truncate drain retain
retain_mut dedup swap swap_remove resize take replace get_mut iter_mut values_mut last_mut first_mut as_mut entry
or_insert or_default or_insert_with sort sort_by sort_by_key sort_unstable reverse fill set get_or_insert
get_or_insert_with insert_with set_len split_off rotate_left rotate_right borrow_mut lock write store fetch_add
fetch_sub swap_with_slice copy_from_slice clone_from advance reserve shrink_to_fit
""".split())

# Calls that hand back an owned copy: a mutation after one does not touch the field.
COPIES = frozenset("clone cloned to_vec to_owned to_string collect copied".split())

KEYWORDS = frozenset("""
as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub
ref return self Self static struct super trait true type unsafe use where while
""".split())
PRELUDE = frozenset("""
Some None Ok Err Vec String Box Option Result Default Clone Copy Send Sync Sized Drop From Into ToString ToOwned
TryFrom TryInto AsRef AsMut Iterator IntoIterator Extend Fn FnMut FnOnce PartialEq Eq PartialOrd Ord Debug
i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f32 f64 bool char str drop panic unreachable
""".split())

# Order-sensitive markers: category, regex over a blanked body line.
MARKERS = [
    ("random", re.compile(r"\b\w*random\w*|\b\w*rng\w*|\bnext_(?:int|long|float|double|bool|gaussian|f32|f64|[iu](?:32|64))\w*")),
    ("entity id", re.compile(r"\bnext_entity\w*|\balloc_entity\w*|\bnew_entity_id\w*|\bnext_\w*_?id\b")),
    ("spawn", re.compile(r"\bspawn_\w+(?=\s*(?:::\s*<[^()]*>\s*)?\()|\bspawn\s*\(")),
    ("fire", re.compile(r"\bfire_\w+(?=\s*(?:::\s*<[^()]*>\s*)?\()")),
    ("send", re.compile(r"\b\w*send\w*\s*\(")),
    ("packet push", re.compile(
        r"\b\w*(?:packet|outbox|out_queue)\w*\s*\.\s*(?:push|push_back|extend|send|write)\w*|\bpush_packet\w*|\bqueue_packet\w*|\benqueue\w*")),
    ("other effect", re.compile(
        r"\bbroadcast\w*|\bplay_sound\w*|\bemit\w*|\bdispatch\w*|\bpush_event\w*|\bqueue_\w+")),
]


# ---------------------------------------------------------------------------------------------------------- text

BLANK_START = re.compile(r"//|/\*|[bc]?r#*\"|[bc]?\"|'")
CHAR_LITERAL = re.compile(r"'(?:\\(?:x[0-9a-fA-F]{2}|u\{[^}]*\}|.)|[^\\'\n])'")
RAW_START = re.compile(r'b?r(#*)"')


def blank(text):
    """The source with comments, string and char literals blanked to spaces (newlines and offsets kept)."""
    pieces = []  # (start, end) spans to blank
    n = len(text)
    i = 0
    while True:
        m = BLANK_START.search(text, i)
        if not m:
            break
        at, token = m.start(), m.group()
        if at and token[0] in "rbc" and (text[at - 1].isalnum() or text[at - 1] == "_"):
            i = m.end()
            continue
        if token == "//":
            j = text.find("\n", at)
            j = n if j < 0 else j
            pieces.append((at, j))
            i = j
        elif token == "/*":
            depth, j = 1, at + 2
            while j < n and depth:
                k = min((x for x in (text.find("/*", j), text.find("*/", j)) if x >= 0), default=n)
                if k >= n:
                    j = n
                    break
                if text.startswith("/*", k):
                    depth, j = depth + 1, k + 2
                else:
                    depth, j = depth - 1, k + 2
            pieces.append((at, j))
            i = j
        elif token == "'":
            c = CHAR_LITERAL.match(text, at)
            if c:
                pieces.append((at + 1, c.end() - 1))
                i = c.end()
            else:
                i = at + 1
        elif "r" in token and token.rstrip("#\"").endswith("r"):
            hashes = token.count("#")
            end = text.find('"' + "#" * hashes, m.end())
            end = n if end < 0 else end
            pieces.append((m.end(), end))
            i = min(n, end + 1 + hashes)
        else:
            j = m.end()
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            pieces.append((m.end(), min(j, n)))
            i = j + 1
    if not pieces:
        return text
    out, last = [], 0
    for a, b in pieces:
        out.append(text[last:a])
        out.append(re.sub(r"[^\n]", " ", text[a:b]))
        last = b
    out.append(text[last:])
    return "".join(out)


class Source:
    """One file: its text, the blanked text, line lookup and brace depths."""

    def __init__(self, path, root=ROOT):
        self.path = path
        self.rel = os.path.relpath(path, root)
        self.text = path.read_text(encoding="utf-8", errors="replace")
        self.code = blank(self.text)
        self.lines = [m.start() for m in re.finditer("\n", self.code)]
        self._braces = None

    def line(self, offset):
        return bisect.bisect_left(self.lines, offset) + 1

    def at(self, offset):
        return f"{self.rel}:{self.line(offset)}"

    def snippet(self, offset):
        start = self.text.rfind("\n", 0, offset) + 1
        end = self.text.find("\n", offset)
        return self.text[start:end if end >= 0 else len(self.text)].strip()

    def depth(self, offset):
        """Brace depth at `offset`."""
        if self._braces is None:
            depth, self._braces = 0, ([], [])
            for m in re.finditer(r"[{}]", self.code):
                depth += 1 if m.group() == "{" else -1
                self._braces[0].append(m.start())
                self._braces[1].append(depth)
        index = bisect.bisect_right(self._braces[0], offset) - 1
        return self._braces[1][index] if index >= 0 else 0

    def block_span(self, offset):
        """(open, close) of the smallest `{}` block holding `offset`; the whole file when it is at top level."""
        if not hasattr(self, "_spans"):
            self._spans, stack = [], []
            for m in re.finditer(r"[{}]", self.code):
                if m.group() == "{":
                    stack.append(m.start())
                elif stack:
                    self._spans.append((stack.pop(), m.start()))
        best = (0, len(self.code))
        for a, b in self._spans:
            if a < offset < b and b - a < best[1] - best[0]:
                best = (a, b)
        return best

    def match_brace(self, open_at):
        depth = 0
        for m in re.finditer(r"[{}]", self.code[open_at:]):
            depth += 1 if m.group() == "{" else -1
            if depth == 0:
                return open_at + m.start()
        return len(self.code) - 1

    def attrs_before(self, offset):
        """The attributes (as text) right before an item starting at `offset`."""
        text, end, found = self.code, offset, []
        while True:
            head = text[:end].rstrip()
            if not head.endswith("]"):
                return found
            depth, k = 0, len(head) - 1
            while k >= 0:
                depth += (head[k] == "]") - (head[k] == "[")
                if depth == 0:
                    break
                k -= 1
            if k <= 0 or head[:k].rstrip()[-1:] != "#":
                return found
            found.append(head[k + 1:-1].strip())
            end = head[:k].rstrip().rfind("#")


def split_top(text, sep=","):
    """Split at `sep` outside any bracket pair (angle brackets too, `->` excepted)."""
    parts, depth, last = [], 0, 0
    for i, c in enumerate(text):
        if c in "([{<":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == ">" and text[i - 1:i] != "-":
            depth -= 1
        elif c == sep and depth <= 0:
            parts.append(text[last:i])
            last = i + 1
    parts.append(text[last:])
    return [p.strip() for p in parts if p.strip()]


def balanced(text, start, open_char="(", close_char=")"):
    """The index just past the group opening at `start`."""
    depth = 0
    for i in range(start, len(text)):
        if close_char == ">" and text[i] == ">" and text[i - 1:i] == "-":
            continue  # the arrow of `Fn() -> T`, not a closing bracket
        depth += (text[i] == open_char) - (text[i] == close_char)
        if depth == 0:
            return i + 1
    return len(text)


# ------------------------------------------------------------------------------------------------------ the index

class Fn:
    def __init__(self, src, name, fn_at, owner, owner_trait, attrs, impl_attrs):
        self.src = src
        self.name = name
        self.fn_at = fn_at
        self.owner = owner  # "Game" for an impl on Game, else the impl's type text or None for a free function
        self.trait = owner_trait
        self.attrs = attrs
        self.impl_attrs = impl_attrs
        code = src.code
        m = re.compile(r"fn\s+(\w+)\s*(<)?").match(code, fn_at)
        k = m.end() - 1 if m.group(2) else m.end()
        if m.group(2):
            k = balanced(code, k, "<", ">")
        while code[k].isspace():
            k += 1
        params_end = balanced(code, k)
        self.params_text = code[k + 1:params_end - 1]
        j, depth = params_end, 0
        while j < len(code):
            c = code[j]
            depth += (c in "([<") - (c in ")]>" and code[j - 1] != "-")
            if depth <= 0 and c in "{;":
                break
            j += 1
        self.sig_end = j
        tail = code[params_end:j]
        ret = re.match(r"\s*->\s*(.*?)(?:\bwhere\b.*)?\Z", tail, re.S)
        self.ret = ret.group(1).strip() if ret else ""
        self.has_body = code[j:j + 1] == "{"
        self.body_start = j if self.has_body else None
        self.body_end = src.match_brace(j) if self.has_body else None
        params = split_top(self.params_text)
        first = params[0] if params else ""
        r = re.match(r"(&\s*(?:'\w+\s+)?(mut\s+)?)?(mut\s+)?self\b(\s*:)?", first)
        if r and r.group(4):
            self.receiver = "typed"
        elif r and r.group(1):
            self.receiver = "&mut self" if r.group(2) else "&self"
        elif r:
            self.receiver = "self"
        else:
            self.receiver = None
        self.params = params[1:] if self.receiver else params
        self.param_names = set(re.findall(r"(?:^|\s)(?:mut\s+|ref\s+)*(\w+)\s*:", " ".join(
            p if ":" in p else "" for p in self.params), re.M))
        self.test_only = False
        self.gated_file = False

    @property
    def line(self):
        return self.src.line(self.fn_at)

    @property
    def where(self):
        return f"{self.src.rel}:{self.line}"

    def qualifiers(self):
        before = self.src.code[max(0, self.fn_at - 160):self.fn_at]
        m = re.search(r"(?:\b(?:pub(?:\s*\([^)]*\))?|const|async|unsafe|default|extern(?:\s*\"[^\"]*\")?)\s+)*\Z", before)
        return m.group(0).split() if m else []

    def eligible(self):
        """(bool, reason): whether point_shape::eligible, read from the text, admits it as a `Game` method."""
        why = []
        if self.owner != "Game" or self.trait:
            return False, "not an inherent `impl Game` method"
        if self.receiver not in ("&self", "&mut self"):
            why.append("receiver is %s" % (self.receiver or "none (associated fn)"))
        if self.name.startswith("native_"):
            why.append("native_* name")
        quals = set(self.qualifiers())
        for word in ("const", "async", "unsafe", "extern"):
            if word in quals:
                why.append(word)
        for attr in self.attrs + self.impl_attrs:
            head = re.match(r"\w+", attr)
            head = head.group() if head else ""
            if head in ("cfg", "cfg_attr", "test"):
                why.append("#[%s]" % head)
            elif head == "inline" and re.search(r"\(\s*always\s*\)", attr):
                why.append("#[inline(always)]")
        if self.ret == "!":
            why.append("returns !")
        if re.match(r"impl\b", self.ret):
            why.append("returns impl Trait")
        for p in self.params:
            name, _, ty = p.partition(":") if "::" not in p.split(":")[0] else (p, "", "")
            if re.match(r"impl\b", ty.strip()):
                why.append("impl Trait parameter")
            if re.fullmatch(r"(?:mut\s+|ref\s+)*(?:cancel|result)", name.strip()):
                why.append("parameter named %s" % name.strip().split()[-1])
        if self.test_only:
            why.append("test-only code")
        if self.gated_file:
            why.append("file behind a cfg-gated mod")
        return (not why), "; ".join(dict.fromkeys(why))


class Index:
    """Every `fn` of horizon-client, the `impl Game` ones by name, plus the Game fields and a name index."""

    def __init__(self, src_root=SRC, root=ROOT):
        self.src_root = src_root
        self.sources = {}
        self.fns = []  # every Fn
        self.methods = defaultdict(list)  # inherent `impl Game` method name -> [Fn]
        self.trait_methods = defaultdict(list)  # the same for `impl Trait for Game`
        self.fields = []
        self.gated_dirs = set()
        self.heads = set()
        for path in sorted(src_root.rglob("*.rs")):
            self.sources[path] = Source(path, root)
        self.by_rel = {src.rel: src for src in self.sources.values()}
        self.collect_gated()
        for src in self.sources.values():
            self.scan(src)
        self.read_struct()

    # --- structure
    def collect_gated(self):
        """Files declared as `#[cfg(..)] mod x;` (and everything under them): compiled only in some builds."""
        for path, src in self.sources.items():
            stem = path.stem
            base = path.parent if stem in ("mod", "lib", "main") else path.parent / stem
            for m in re.finditer(r"\bmod\s+(\w+)\s*;", src.code):
                attrs = src.attrs_before(self.item_start(src.code, m.start()))
                if any(re.match(r"cfg(?:_attr)?\b", a) for a in attrs):
                    for cand in (base / (m.group(1) + ".rs"), base / m.group(1) / "mod.rs"):
                        self.gated_dirs.add(cand)
                        self.gated_dirs.add(cand.parent if cand.name == "mod.rs" else cand.with_suffix(""))

    def file_gated(self, path):
        if path in self.gated_dirs:
            return True
        return any(parent in self.gated_dirs for parent in path.parents)

    def test_file(self, path):
        return (path.name.endswith("_tests.rs") or path.name in ("tests.rs", "test.rs")
                or "tests" in path.parts[len(self.src_root.parts):])

    def scan(self, src):
        code = src.code
        gated_file = self.file_gated(src.path)
        in_test_file = self.test_file(src.path)
        # inline modules with cfg attributes
        mods = []
        for m in re.finditer(r"\bmod\s+(\w+)\s*\{", code):
            attrs = src.attrs_before(self.item_start(code, m.start()))
            if any(re.match(r"cfg(?:_attr)?\b", a) for a in attrs):
                test = any(re.search(r"\btest\b", a) for a in attrs)
                mods.append((m.start(), src.match_brace(m.end() - 1), test))
        impls = []
        for m in re.finditer(r"\bimpl\b([^{};]*)\{", code):
            head = m.group(1)
            k = 0
            if head.lstrip().startswith("<"):
                k = head.index("<")
                k = balanced(head, k, "<", ">")
            rest = head[k:].strip()
            trait = None
            parts = re.split(r"\s+for\s+", rest, maxsplit=1)
            if len(parts) == 2:
                trait, ty = parts
            else:
                ty = rest
            ty = re.sub(r"\s+where\b.*\Z", "", ty, flags=re.S).strip()
            tm = re.match(r"(?:&\s*(?:'\w+\s+)?(?:mut\s+)?)?((?:\w+\s*::\s*)*)(\w+)\s*(?:<.*>)?\Z", ty, re.S)
            if not tm:
                continue
            before = code[max(0, m.start() - 3):m.start()]
            if before.strip().endswith((":", "(", "<", ",", "&", ">")):
                continue
            end = src.match_brace(m.end() - 1)
            impls.append((m.start(), m.end() - 1, end, tm.group(2), trait))
        for start, open_at, end, owner, trait in impls:
            impl_attrs = src.attrs_before(self.item_start(code, start))
            depth = src.depth(open_at)
            for fm in re.finditer(r"\bfn\s+\w+", code[open_at:end]):
                at = open_at + fm.start()
                if src.depth(at) != depth:
                    continue
                f = Fn(src, fm.group().split()[1], at, owner, trait is not None, src.attrs_before(self.item_start(code, at)),
                       impl_attrs)
                f.test_only = in_test_file or any(a <= at <= b and t for a, b, t in mods)
                f.cfg_mod = any(a <= at <= b for a, b, _ in mods)
                f.gated_file = gated_file
                if f.cfg_mod and not f.test_only:
                    f.impl_attrs = impl_attrs + ["cfg(module)"]
                if owner == "Game":
                    self.fns.append(f)
                    (self.trait_methods if trait else self.methods)[f.name].append(f)
        # free functions and other impls' fns are built only when a spec names them
        src.impl_spans = impls
        src.in_test_file = in_test_file

    def plain_fns(self, src, name):
        game_ats = {f.fn_at for f in self.fns if f.src is src}
        out = []
        for fm in re.finditer(r"\bfn\s+" + re.escape(name) + r"\b", src.code):
            if fm.start() in game_ats:
                continue
            owner = None
            for _start, open_at, end, ty, _trait in src.impl_spans:
                if open_at < fm.start() < end:
                    owner = ty
            try:
                f = Fn(src, name, fm.start(), owner, False, [], [])
            except (AttributeError, IndexError):
                continue
            f.test_only = src.in_test_file
            out.append(f)
        return out

    @staticmethod
    def item_start(code, offset):
        """Where an item that has its keyword at `offset` starts: before its visibility and qualifiers."""
        before = code[max(0, offset - 160):offset]
        m = re.search(r"(?:\b(?:pub(?:\s*\([^)]*\))?|const|async|unsafe|default|extern(?:\s*\"[^\"]*\")?)\s+)*\Z", before)
        return offset - len(m.group(0)) if m else offset

    def read_struct(self):
        for src in self.sources.values():
            m = re.search(r"^(?:pub(?:\([^)]*\))?\s+)?struct\s+Game\b[^{;]*\{", src.code, re.M)
            if not m:
                continue
            end = src.match_brace(m.end() - 1)
            body = src.code[m.end():end]
            for part in split_top(body):
                part = re.sub(r"^(?:#\[[^\]]*\]\s*)+", "", part.strip())
                fm = re.match(r"(?:pub(?:\s*\([^)]*\))?\s+)?(\w+)\s*:(?!:)", part)
                if fm:
                    self.fields.append(fm.group(1))
            self.struct_at = src.at(m.start())
            return
        raise SystemExit("game_uses: struct Game not found under crates/horizon-client/src")

    # --- lookups
    def find(self, spec):
        """Resolve a spec to a list of (label, Source, start, end, Fn or None)."""
        m = re.fullmatch(r"(.+?):(\d+)-(\d+)", spec)
        if m:
            src = self.source(m.group(1))
            first, last = int(m.group(2)), int(m.group(3))
            starts = [0] + [p + 1 for p in src.lines]
            if not 1 <= first <= last <= len(starts):
                raise SystemExit(f"game_uses: {src.rel} has {len(starts)} lines; the range {first}-{last} is not "
                                 "inside it (FIRST must be at least 1 and not after LAST)")
            a = starts[first - 1]
            b = starts[last] if last < len(starts) else len(src.code)
            return [(f"{src.rel}:{first}-{last}", src, a, b, None)]
        if "::" in spec:
            file, names = spec.rsplit("::", 1)
            src = self.source(file)
            out = []
            for name in names.split(","):
                found = [f for f in self.methods.get(name, []) + self.trait_methods.get(name, [])
                         if f.src is src and f.has_body] + \
                    [f for f in self.plain_fns(src, name) if f.has_body]
                if not found:
                    raise SystemExit(f"game_uses: no fn `{name}` with a body in {src.rel}")
                for f in found:
                    out.append((f"{f.where} {f.name}", src, f.body_start, f.body_end, f))
            return out
        found = [f for f in self.methods.get(spec, []) if f.has_body and not f.test_only] or \
                [f for f in self.methods.get(spec, []) if f.has_body]
        if len(found) != 1:
            raise SystemExit(f"game_uses: `{spec}` names {len(found)} Game methods: "
                             + ", ".join(f.where for f in found) + "; give FILE::NAME")
        f = found[0]
        return [(f"{f.where} {f.name}", f.src, f.body_start, f.body_end, f)]

    def source(self, file):
        path = (ROOT / file)
        if path.is_file() and path.resolve() in self.sources:
            return self.sources[path.resolve()]
        wanted = file.replace("\\", "/").lstrip("./")
        hits = [s for s in self.sources.values()
                if s.rel.replace(os.sep, "/").endswith("/" + wanted) or s.rel.replace(os.sep, "/") == wanted]
        if len(hits) != 1:
            raise SystemExit(f"game_uses: `{file}` matches {len(hits)} files of horizon-client"
                             + "".join("\n  " + s.rel for s in hits[:8]))
        return hits[0]


# ----------------------------------------------------------------------------------------------------- analysis

class Uses:
    """What one body (or a union of bodies) uses."""

    def __init__(self):
        self.fields = {}  # name -> {"read": n, "write": n, "lines": [...]}
        self.methods = {}  # name -> {"count": n, "lines": [...]}
        self.assoc = {}  # Self::name( / Game::name(
        self.whole_self = []
        self.destructured = {}
        self.markers = []
        self.names = {}  # path -> {"count": n, "lines": [...]}
        self.unknown_fields = {}

    def field(self, name, mode, where):
        f = self.fields.setdefault(name, {"read": 0, "write": 0, "lines": []})
        f[mode] += 1
        if where not in f["lines"]:
            f["lines"].append(where)

    @staticmethod
    def bump(table, key, where, site=None):
        row = table.setdefault(key, {"count": 0, "lines": []})
        row["count"] += 1
        if where not in row["lines"]:
            row["lines"].append(where)
        if site:
            row.setdefault("sites", []).append(site)


ASSIGN = re.compile(r"\s*(?:=(?![=>])|[+\-*/%^|&]=|<<=|>>=)")


def classify_chain(code, pos):
    """Read the access chain after `self.field` from `pos`: ('write'|'read', first_method_called)."""
    n = len(code)
    first_call = None
    while pos < n:
        m = re.compile(r"\s*\[").match(code, pos)
        if m:
            pos = balanced(code, m.end() - 1, "[", "]")
            continue
        m = re.compile(r"\s*\?").match(code, pos)
        if m:
            pos = m.end()
            continue
        m = re.compile(r"\s*\.\s*(\w+)\s*(?:::\s*<[^()]*?>\s*)?(\()?").match(code, pos)
        if m:
            if m.group(2):
                first_call = first_call or m.group(1)
                if m.group(1) in COPIES:
                    return "read", m.group(1)  # what follows changes a copy, not the field
                if m.group(1) in MUTATORS or m.group(1).startswith("set_"):
                    return "write", m.group(1)
                pos = balanced(code, m.end() - 1)
                continue
            pos = m.end()
            continue
        break
    if ASSIGN.match(code, pos):
        return "write", None
    return "read", first_call


def analyse(index, src, start, end, fn, aliases=()):
    use = Uses()
    code = src.code
    body = code[start:end]
    base = start
    # `self` is the game only in an `impl Game` method (or a line range, where it is assumed to be)
    own_self = fn is None or fn.owner == "Game"
    names = (("self",) if own_self else ()) + tuple(aliases)
    alt = "|".join(re.escape(n) for n in names)
    field_re = re.compile(rf"(?<![\w.])(?:{alt})\s*\.\s*(\w+)\s*(::\s*<[^()]*?>\s*)?(\()?") if names else None
    fields = set(index.fields)
    for m in (field_re.finditer(body) if field_re else ()):
        name, call = m.group(1), m.group(3)
        at = base + m.start()
        where = src.at(at)
        if call or m.group(2):
            Uses.bump(use.methods, name, where)
            continue
        if name.isdigit():
            continue
        prefix = code[max(0, at - 16):at]
        mode, _ = classify_chain(code, base + m.end())
        if re.search(r"&\s*mut\s*\(?\s*\Z", prefix) or re.search(r"\bref\s+mut\s*\Z", prefix):
            mode = "write"
        if name in fields:
            use.field(name, mode, where)
        else:
            Uses.bump(use.unknown_fields, name, where)
    # whole self: `self` not followed by `.` or `::`, not part of `Self`
    signature = re.compile(r"\bfn\s+\w+\s*(?:<[^>]*>)?\s*\(\s*(?:&\s*(?:'\w+\s+)?)?(?:mut\s+)?\Z")
    for m in re.finditer(r"(?<![\w.])self\b(?!\s*\.)(?!\s*::)" if own_self else r"(?!)", body):
        if signature.search(body[max(0, m.start() - 80):m.start()]):
            continue
        use.whole_self.append(src.at(base + m.start()) + "  " + src.snippet(base + m.start()))
    # `Self::name(..)` / `Game::name(..)`: a call to a Game method belongs with the methods (and is followed by
    # --transitive); an associated constant or an unknown name stays in `assoc`
    path_re = r"(?<![\w.])(?:Self|Game)\s*::\s*(\w+)\s*(\()?" if own_self else r"(?<![\w.])Game\s*::\s*(\w+)\s*(\()?"
    for m in re.finditer(path_re, body):
        name, call = m.group(1), m.group(2)
        where = src.at(base + m.start())
        if call and (name in index.methods or name in index.trait_methods):
            Uses.bump(use.methods, name, where)
        else:
            Uses.bump(use.assoc, name + ("()" if call else ""), where)
    # `let Self { a, b, .. } = self;`
    for m in re.finditer(r"\blet\s+(?:&\s*(?:mut\s+)?)?Self\s*\{([^}]*)\}\s*=" if own_self else r"(?!)", body):
        for part in split_top(m.group(1)):
            fm = re.match(r"(?:ref\s+)?(?:mut\s+)?(\w+)", part)
            if fm and fm.group(1) in fields:
                use.destructured[fm.group(1)] = src.at(base + m.start())
    # markers: every match, with its offset, so draws repeated on one line stay and the order is the source's
    for category, rx in MARKERS:
        for m in rx.finditer(body):
            at = base + m.start()
            use.markers.append({"line": src.line(at), "file": src.rel, "offset": at, "category": category,
                                "match": re.sub(r"\s+", "", m.group()), "code": src.snippet(at)})
    use.markers.sort(key=lambda x: (x["offset"], x["category"]))
    collect_names(index, src, body, base, fn, use)
    return use


NAME = re.compile(r"(?<![\w.$])([A-Za-z_]\w*(?:\s*::\s*(?:<[^<>]*(?:<[^<>]*>[^<>]*)*>\s*::\s*)?[A-Za-z_]\w*)*)(\s*!)?(\s*\()?")


def collect_names(index, src, body, base, fn, use):
    local = set(fn.param_names) if fn else set()
    local |= set(re.findall(r"\blet\s+(?:mut\s+)?(\w+)\s*(?::[^=;]*)?=\s*(?:move\s*)?\|", body))
    for m in NAME.finditer(body):
        path, macro, call = m.group(1), m.group(2), m.group(3)
        if macro:
            continue
        segs = [s.strip() for s in re.split(r"\s*::\s*", re.sub(r"::\s*<.*?>\s*", "", path))]
        head = segs[0]
        before = body[max(0, m.start() - 8):m.start()]
        if head in KEYWORDS or head in PRELUDE or head in ("self", "Self", "Game", "super"):
            if not (head in ("super", "crate") and len(segs) > 1):
                continue
        if re.search(r"\b(?:fn|let|mut|ref|struct|enum|trait|type|mod|use)\s*\Z", before) or before.rstrip().endswith("'"):
            continue
        if head in local:
            continue
        if len(segs) == 1:
            if head[0].islower() and not call:
                continue
            if len(head) == 1:
                continue
        elif head[0].islower() and head not in index.heads and head not in ("crate", "super"):
            continue
        Uses.bump(use.names, "::".join(segs), src.at(base + m.start()), (src.rel, base + m.start()))


# ------------------------------------------------------------------------------------------------------ resolution

class Resolver:
    """`use` resolution as far as practical: explicit imports, `super::*`/`crate::m::*` globs, then a name index."""

    def __init__(self, index):
        self.index = index
        self.crates = {}  # normalised crate name -> crate dir name
        for manifest in CRATES.glob("*/Cargo.toml"):
            self.crates[manifest.parent.name.replace("-", "_")] = manifest.parent.name
        self.deps = {}  # local name -> package
        text = (CLIENT / "Cargo.toml").read_text()
        section = re.search(r"\[dependencies\](.*?)(?:\n\[|\Z)", text, re.S)
        for line in (section.group(1) if section else "").splitlines():
            m = re.match(r'\s*([\w-]+)\s*=\s*(.*)', line)
            if not m:
                continue
            pkg = re.search(r'package\s*=\s*"([^"]+)"', m.group(2))
            self.deps[m.group(1).replace("-", "_")] = (pkg.group(1) if pkg else m.group(1))
        self.cache = {}
        self.uses_cache = {}
        self.items = {}
        self.module_heads = {p.stem for p in SRC.rglob("*.rs")} | {p.name for p in SRC.rglob("*") if p.is_dir()}

    def crate_items(self, crate):
        """name -> [(file, line, kind)] for the items declared in a crate's sources."""
        if crate not in self.items:
            table = self.items[crate] = defaultdict(list)
            rx = re.compile(r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+|const\s+|async\s+)*"
                            r"(fn|struct|enum|trait|type|const|static|union|mod)\s+(\w+)", re.M)
            for path in sorted((CRATES / crate).glob("src/**/*.rs")):
                known = self.index.sources.get(path)
                try:
                    # comments and strings blanked, so a `fn x()` in a comment is no declaration
                    code = known.code if known else blank(path.read_text(encoding="utf-8", errors="replace"))
                except OSError:
                    continue
                newlines = None
                for m in rx.finditer(code):
                    if newlines is None:
                        newlines = [x.start() for x in re.finditer("\n", code)]
                    table[m.group(2)].append((os.path.relpath(path, ROOT), bisect.bisect_left(newlines, m.start()) + 1,
                                              m.group(1)))
        return self.items[crate]

    def find_items(self, name, crate=None, within=()):
        """[(crate, file, line, kind)] of items called `name`: in `crate`, else horizon-client first, then every crate.
        `within` are path segments the file's path should contain (a module path), to narrow a match."""
        pools = [crate] if crate else ["horizon-client"]
        hits = []
        for c in pools:
            hits = [(c, *h) for h in self.crate_items(c).get(name, [])]
        if not hits and not crate:
            for c in sorted(self.crates.values()):
                hits += [(c, *h) for h in self.crate_items(c).get(name, [])]
        if within and len(hits) > 1:
            narrowed = [h for h in hits if all(w in h[1] for w in within)]
            hits = narrowed or hits
        return hits

    # --- use declarations of a file
    def uses_of(self, src, at=None):
        """(binds, globs) of the `use` declarations in scope at offset `at` (None: those at the top of the file).
        A `use` inside a block holds only in that block, and an inner one replaces an outer one of the same name."""
        if src.path not in self.uses_cache:
            found = []
            for m in re.finditer(r"(?<![\w])use\s+([^;]*);", src.code):
                span = src.block_span(m.start())
                for path, alias in expand_use(m.group(1)):
                    found.append((span, path, alias))
            self.uses_cache[src.path] = found
        binds, globs = {}, []
        top = (0, len(src.code))
        visible = [u for u in self.uses_cache[src.path]
                   if u[0] == top or (at is not None and u[0][0] < at < u[0][1])]
        for _, path, alias in sorted(visible, key=lambda u: -(u[0][1] - u[0][0])):
            if path.endswith("::*") or path == "*":
                globs.append(path[:-3] if path != "*" else "")
            else:
                name = alias or path.split("::")[-1]
                if path.endswith("::self"):
                    path = path[:-6]
                    name = alias or path.split("::")[-1]
                binds[name] = path
        return binds, globs

    def module_file(self, src, path):
        """The source file of module path `crate::a::b` / `super::x` / `self::y` from `src`, as far as it exists."""
        segs = [s for s in path.split("::") if s]
        if not segs:
            return None
        here = module_dir_of(src.path)  # directory holding this module's children
        if segs[0] == "crate":
            cur, segs = SRC, segs[1:]
        elif segs[0] == "self":
            cur, segs = here, segs[1:]
        elif segs[0] == "super":
            cur = here
            while segs and segs[0] == "super":
                cur, segs = cur.parent, segs[1:]
        else:
            return None
        found = None
        for seg in segs:
            if (cur / (seg + ".rs")).is_file():
                found, cur = cur / (seg + ".rs"), cur / seg
            elif (cur / seg / "mod.rs").is_file():
                found, cur = cur / seg / "mod.rs", cur / seg
            else:
                break
        if not segs:
            found = parent_file(src.path) if path.startswith("super") else src.path
            if path.startswith("crate"):
                found = SRC / "lib.rs" if (SRC / "lib.rs").is_file() else SRC / "main.rs"
        return found

    def resolve(self, src, name, depth=0, seen=None, at=None):
        """(origin, detail, how): origin is a crate dir name, 'std', 'external:X' or 'horizon-client'.
        `at` is the offset of the reference: it picks the `use` declarations in scope there."""
        binds, globs = self.uses_of(src, at)
        key = (src.path, name, binds.get(name), tuple(globs))
        if key in self.cache:
            return self.cache[key]
        seen = seen or set()
        if key in seen or depth > 7:
            return None
        seen = seen | {key}
        out = self._resolve(src, name, depth, seen, binds, globs)
        self.cache[key] = out
        return out

    def classify(self, src, path, depth, seen, at=None):
        if depth > 7:
            return ("unresolved", path, "use")
        segs = path.split("::")
        head = segs[0]
        if head in ("std", "core", "alloc"):
            return ("std", path, "use")
        if head == "horizon_client":
            segs = ["crate"] + segs[1:]
            path = "::".join(segs)
            head = "crate"
        if head == "crate" and len(segs) > 1:
            for root in ("lib.rs", "client_lib.rs"):
                root_src = self.index.sources.get(SRC / root)
                bound = self.uses_of(root_src)[0].get(segs[1]) if root_src else None
                if bound and not bound.startswith(("crate", "self", "super")) and bound.split("::")[0] != segs[1]:
                    return self.classify(src, "::".join([bound] + segs[2:]), depth + 1, seen)
        if head in ("crate", "self", "super"):
            target = self.module_file(src, "::".join(segs[:-1])) if len(segs) > 1 else None
            if target and target in self.index.sources:
                tsrc = self.index.sources[target]
                inner = self.resolve(tsrc, segs[-1], depth + 1, seen)
                if inner and inner[0] != "horizon-client":
                    return inner
                local = self.local_item(tsrc, segs[-1])
                if local:
                    return ("horizon-client", local, "use")
            return ("horizon-client", path, "use")
        if head in self.deps:
            pkg = self.deps[head]
            if pkg.replace("-", "_") in self.crates or pkg in self.crates.values():
                return (self.crates.get(pkg.replace("-", "_"), pkg), path, "use")
            return ("external:" + pkg, path, "use")
        if head in self.crates:
            return (self.crates[head], path, "use")
        bound = self.uses_of(src, at)[0].get(head)
        if bound and bound != path:
            return self.classify(src, bound + ("::" + "::".join(segs[1:]) if len(segs) > 1 else ""), depth + 1, seen, at)
        if head in self.module_heads:
            return ("horizon-client", path, "use (relative)")
        return ("unresolved", path, "use")

    def local_item(self, src, name):
        m = re.search(r"^[ \t]*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+|const\s+|async\s+)*(?:fn|struct|enum|trait|type|const|static|union)\s+"
                      + re.escape(name) + r"\b", src.code, re.M)
        return src.at(m.start()) if m else None

    def _resolve(self, src, name, depth, seen, binds, globs):
        if name in binds:
            return self.classify(src, binds[name], depth, seen)
        local = self.local_item(src, name)
        if local:
            return ("horizon-client", local, "defined in this file")
        for glob in globs:
            if glob.split("::")[0] in ("std", "core", "alloc"):
                continue
            if glob.split("::")[0] in ("crate", "self", "super") or glob == "":
                target = self.module_file(src, glob) if glob else None
                if target and target in self.index.sources and target != src.path:
                    inner = self.resolve(self.index.sources[target], name, depth + 1, seen)
                    if inner:
                        return (inner[0], inner[1], "glob " + glob + "::*")
                continue
            root = glob.split("::")[0]
            origin = self.crates.get(self.deps.get(root, root).replace("-", "_"))
            if origin:
                hits = self.find_items(name, origin, within=glob.split("::")[1:])
                if hits:
                    return (origin, f"{hits[0][1]}:{hits[0][2]}", "glob " + glob + "::*")
        return None

    def final(self, src, path, at=None):
        """Resolve a used path (a type's path, or a free function's) to (origin, detail, how)."""
        segs = path.split("::")
        head = segs[0]
        if head in ("std", "core", "alloc"):
            return ("std", path, "")
        if head in ("super", "crate", "self"):
            return self.classify(src, path, 0, set(), at)
        own = head == "horizon_client"
        if own:
            via = self.classify(src, path, 0, set())
            if via[0] != "horizon-client":
                return self.final(src, via[1], at)
        pkg = self.deps.get(head, head)
        crate = self.crates.get(pkg.replace("-", "_"))
        if head in self.deps and not crate and head not in self.uses_of(src, at)[0]:
            return ("external:" + pkg, path, "")
        if own or (crate and head not in self.uses_of(src, at)[0]):
            crate = "horizon-client" if own else crate
            name = segs[-1]
            hits = self.find_items(name, crate, within=[s_ for s_ in segs[1:-1]])
            if len(hits) >= 1:
                return (crate, f"{hits[0][1]}:{hits[0][2]}", "" if len(hits) == 1 else f"{len(hits)} items of that name")
            return (crate, path, "item not found by name (re-export?)" if own else "")
        got = self.resolve(src, head, at=at)
        if got:
            return got
        if len(segs) == 1 and head[0].islower() and not self.find_items(head, "horizon-client"):
            return ("unresolved", "", "")
        hits = self.find_items(head)
        crates = sorted({h[0] for h in hits})
        if len(hits) == 1 or (len(crates) == 1 and len(hits) <= 3):
            h = hits[0]
            return (h[0], f"{h[1]}:{h[2]}", "by name" + (f", {len(hits)} candidates" if len(hits) > 1 else ""))
        if hits:
            return ("ambiguous", f"{len(hits)} items called {head}", "by name")
        return ("unresolved", "", "")


def module_dir_of(path):
    """The directory holding the child modules of the module in `path`."""
    return path.parent if path.stem in ("mod", "lib", "main") else path.parent / path.stem


def parent_file(path):
    up = module_dir_of(path) if path.stem in ("mod", "lib", "main") else path.parent
    # a `mod.rs`'s parent module is the folder's own parent module file
    folder = path.parent if path.stem == "mod" else path.parent
    for cand in (folder.parent / (folder.name + ".rs"), folder.parent / "mod.rs") if path.stem == "mod" else \
            (path.parent / "mod.rs", path.parent.parent / (path.parent.name + ".rs")):
        if cand.is_file():
            return cand
    return None


def expand_use(text):
    """Flatten a use tree into [(path, alias)]."""
    text = re.sub(r"\s+as\s+", "\x00", text)  # keep `as` apart before the other whitespace goes
    text = re.sub(r"\s+", "", text)

    def walk(prefix, tree):
        out = []
        if tree.startswith("{"):
            inner = tree[1:balanced(tree, 0, "{", "}") - 1]
            for part in split_top(inner):
                out += walk(prefix, part)
            return out
        m = re.match(r"([\w:]*?)(::)?(\{.*\})\Z", tree)
        if m and m.group(3):
            head = m.group(1)
            return walk(prefix + (head + "::" if head else ""), m.group(3))
        alias = None
        if "\x00" in tree:
            tree, alias = tree.split("\x00", 1)
        return [((prefix + tree).lstrip(":"), alias)]

    return walk("", text)


# ------------------------------------------------------------------------------------------------------- reports

def merge(a, b):
    """Add the Uses `b` into `a`."""
    for name, f in b.fields.items():
        row = a.fields.setdefault(name, {"read": 0, "write": 0, "lines": []})
        row["read"] += f["read"]
        row["write"] += f["write"]
        row["lines"] += [w for w in f["lines"] if w not in row["lines"]]
    for table in ("methods", "assoc", "names", "unknown_fields"):
        for key, row in getattr(b, table).items():
            into = getattr(a, table).setdefault(key, {"count": 0, "lines": []})
            into["count"] += row["count"]
            into["lines"] += [w for w in row["lines"] if w not in into["lines"]]
            if "sites" in row:
                into.setdefault("sites", []).extend(row["sites"])
    a.whole_self += b.whole_self
    a.destructured.update(b.destructured)
    a.markers += b.markers


def method_info(index, name):
    defs = index.methods.get(name, [])
    if not defs and name in index.trait_methods:
        rows = [{"at": d.where, "candidate": False, "reason": "method of a trait impl for Game, not an inherent impl",
                 "receiver": d.receiver} for d in index.trait_methods[name]]
        return {"defined": rows, "candidate": False}
    live = [d for d in defs if not d.test_only] or defs
    if not live:
        return {"defined": [], "candidate": None, "reason": "no `impl Game` definition found (field closure, trait method or macro)"}
    rows = []
    for d in live:
        ok, why = d.eligible()
        rows.append({"at": d.where, "candidate": ok, "reason": why, "receiver": d.receiver})
    return {"defined": rows, "candidate": all(r["candidate"] for r in rows) if rows else None}


def transitive(index, start_uses, levels, aliases):
    """Follow the called Game methods `levels` deep: {level: {method: Uses}} with first-seen order."""
    seen, out = set(), []
    frontier = list(start_uses.methods)
    for level in range(1, levels + 1):
        nxt, entries = [], []
        for name in frontier:
            if name in seen:
                continue
            seen.add(name)
            defs = [d for d in index.methods.get(name, []) if d.has_body and not d.test_only]
            if not defs:
                entries.append({"method": name, "defined": None, "uses": None})
                continue
            for d in defs:
                u = analyse(index, d.src, d.body_start, d.body_end, d, aliases)
                entries.append({"method": name, "defined": d.where, "uses": u})
                nxt += list(u.methods)
        out.append((level, entries))
        frontier = nxt
    return out


def build(args):
    index = Index()
    resolver = Resolver(index)
    index.heads = set(resolver.crates) | set(resolver.deps) | {"std", "core", "alloc", "crate", "super"}
    targets = []
    for spec in args.specs:
        targets += index.find(spec)
    report = {"targets": [], "game_struct": index.struct_at, "game_fields": len(index.fields)}
    combined = Uses()
    for label, src, start, end, fn in targets:
        uses = analyse(index, src, start, end, fn, args.alias)
        merge(combined, uses)
        entry = target_report(index, resolver, label, src, fn, uses)
        if args.transitive:
            tr = transitive(index, uses, args.transitive, args.alias)
            entry["transitive"] = transitive_report(uses, tr)
        report["targets"].append(entry)
    if len(targets) > 1:
        entry = target_report(index, resolver, "combined: " + ", ".join(t[0].split(" ")[-1] for t in targets), None, None,
                              combined)
        if args.transitive:
            entry["transitive"] = transitive_report(combined, transitive(index, combined, args.transitive, args.alias))
        report["combined"] = entry
    return report


def target_report(index, resolver, label, src, fn, uses):
    entry = {"target": label, "file": src.rel if src else None}
    if fn is not None:
        ok, why = fn.eligible()
        entry["signature"] = {"receiver": fn.receiver, "owner": fn.owner, "mod_point_candidate": ok, "reason": why}
    entry["fields"] = {k: v for k, v in sorted(uses.fields.items(), key=lambda kv: (-(kv[1]["read"] + kv[1]["write"]), kv[0]))}
    entry["destructured_fields"] = uses.destructured
    entry["not_game_fields"] = {k: {"count": v["count"], "first": v["lines"][0]} for k, v in uses.unknown_fields.items()}
    entry["methods"] = {}
    for name, row in sorted(uses.methods.items()):
        info = method_info(index, name)
        entry["methods"][name] = {"count": row["count"], "lines": row["lines"], **info}
    entry["assoc"] = {k: v for k, v in sorted(uses.assoc.items())}
    entry["whole_self"] = uses.whole_self
    # Each use is resolved where it stands (its file and its scope), then equal results are added up, so two
    # targets importing different crates under one name keep separate rows. Members of one type
    # (`MobEvent::Spawn`, `Species::Wither`) are folded into the type.
    folded = {}
    for path, row in sorted(uses.names.items()):
        segs = path.split("::")
        cut = next((i for i, seg in enumerate(segs) if seg[0].isupper() and i < len(segs) - 1), None)
        key = "::".join(segs[:cut + 1]) if cut is not None else path
        for rel, offset in row["sites"]:
            owner = index.by_rel[rel]
            origin, detail, how = resolver.final(owner, key, offset)
            into = folded.setdefault((origin, key, detail, how), {"count": 0, "members": [], "first": owner.at(offset)})
            into["count"] += 1
            if cut is not None:
                into["members"].append("::".join(segs[cut + 1:]))
    groups = defaultdict(list)
    for (origin, key, detail, how), row in folded.items():
        groups[origin].append({"path": key, "count": row["count"], "at": detail, "how": how, "first": row["first"],
                               "members": sorted(set(row["members"]))})
    for rows in groups.values():
        rows.sort(key=lambda r: r["path"])
    entry["names"] = dict(groups)
    entry["markers"] = sorted(uses.markers, key=lambda m: (m["file"], m["offset"], m["category"]))
    return entry


def transitive_report(direct, levels):
    seen = set(direct.fields)
    out = []
    for level, entries in levels:
        added = {}
        called = []
        for e in entries:
            if e["uses"] is None:
                called.append({"method": e["method"], "defined": None})
                continue
            called.append({"method": e["method"], "defined": e["defined"]})
            for name, f in e["uses"].fields.items():
                row = added.setdefault(name, {"read": 0, "write": 0, "via": []})
                row["read"] += f["read"]
                row["write"] += f["write"]
                row["via"].append(e["method"])
        new = {k: v for k, v in sorted(added.items()) if k not in seen}
        out.append({"level": level, "methods": called, "fields": dict(sorted(added.items())), "new_fields": sorted(new),
                    "markers": [{**m, "via": e["method"]} for e in entries if e["uses"] for m in e["uses"].markers]})
        seen |= set(added)
    return out


def short(items, limit=3):
    items = list(items)
    return ", ".join(items[:limit]) + (f" (+{len(items) - limit})" if len(items) > limit else "")


def render(entry):
    home = entry.get("file")

    def loc(where):
        """`file:12` as `L12` when it is in the target's own file."""
        return "L" + where.rsplit(":", 1)[1] if home and where.startswith(home + ":") else where

    out = []
    p = out.append
    p("=" * 100)
    p(entry["target"])
    sig = entry.get("signature")
    if sig:
        p(f"  receiver: {sig['receiver']}; owner: {sig['owner']}; mod point candidate: "
          f"{'yes' if sig['mod_point_candidate'] else 'no (' + sig['reason'] + ')'}")
    fields = entry["fields"]
    p(f"\nGame fields: {len(fields)} distinct, {sum(f['read'] for f in fields.values())} reads, "
      f"{sum(f['write'] for f in fields.values())} writes")
    if fields:
        width = max(len(k) for k in fields)
        for name, f in fields.items():
            mode = "RW" if f["read"] and f["write"] else ("W" if f["write"] else "R")
            p(f"  {name:<{width}}  {mode:<2}  read {f['read']:>3}  write {f['write']:>3}   {short([loc(w) for w in f['lines']], 4)}")
    if entry["destructured_fields"]:
        p("  destructured (`let Self { .. } = self`): " + ", ".join(entry["destructured_fields"]))
    if entry["not_game_fields"]:
        p("  `self.x` that is not a Game field (other receiver type, or a field closure?): "
          + ", ".join(f"{k} x{v['count']}" for k, v in entry["not_game_fields"].items()))
    methods = entry["methods"]
    p(f"\nGame methods called: {len(methods)} distinct")
    for name, m in methods.items():
        defs = m.get("defined") or []
        if not defs:
            p(f"  {name} x{m['count']}  {m.get('reason')}")
            continue
        cand = "mod point candidate" if m["candidate"] else "not a candidate"
        where = defs[0]["at"] + (f" (+{len(defs) - 1} more defs)" if len(defs) > 1 else "")
        why = "" if m["candidate"] else f" ({next((d['reason'] for d in defs if not d['candidate']), '')})"
        p(f"  {name} x{m['count']}  {where}  [{cand}{why}]")
    if entry["assoc"]:
        p("  Self::/Game:: items: " + ", ".join(f"{k} x{v['count']}" for k, v in entry["assoc"].items()))
    if entry["whole_self"]:
        p(f"\nWhole `self` handed on or reborrowed ({len(entry['whole_self'])}): the callee's uses count too")
        for line in entry["whole_self"][:12]:
            p("  " + line)
        if len(entry["whole_self"]) > 12:
            p(f"  (+{len(entry['whole_self']) - 12} more)")
    p("\nFree functions, types and constants named")
    names = entry["names"]
    for origin in sorted(names, key=lambda o: (o != "horizon-client", o in ("unresolved", "ambiguous"), o)):
        rows = names[origin]
        label = {"horizon-client": "horizon-client", "unresolved": "unresolved (local closure, generic, macro-made?)"}.get(origin, origin)
        p(f"  {label} ({len(rows)}):")
        for r in rows:
            tail = f"  {r['at']}" if r["at"] else ""
            note = f"  [{r['how']}]" if r["how"] else ""
            members = f"::{{{short(r['members'], 5)}}}" if r["members"] else ""
            p(f"    {r['path']}{members} x{r['count']}{tail}{note}")
    p("\nOrder markers (keep this order when moving the code)")
    if not entry["markers"]:
        p("  none")
    for m in entry["markers"]:
        p(f"  {loc(m['file'] + ':' + str(m['line'])):<8} {m['category']:<12} {m['match']:<24} {m['code'][:80]}")
    tr = entry.get("transitive")
    if tr:
        for level in tr:
            p(f"\nTransitive level {level['level']}: {len(level['methods'])} methods followed, "
              f"{len(level['fields'])} fields touched, {len(level['new_fields'])} not used directly")
            for name, f in level["fields"].items():
                new = "new" if name in level["new_fields"] else "   "
                p(f"  {name:<32} {new} read {f['read']:>3}  write {f['write']:>3}  via {short(sorted(set(f['via'])), 3)}")
            unknown = [m["method"] for m in level["methods"] if m["defined"] is None]
            if unknown:
                p("  no body found for: " + short(unknown, 8))
            if level["markers"]:
                p(f"  markers inside the callees ({len(level['markers'])}):")
                for m in level["markers"][:15]:
                    p(f"    {m['file']}:{m['line']}  {m['category']:<12} {m['match']:<24} via {m['via']}")
    return "\n".join(out)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("specs", nargs="+", metavar="SPEC", help="FILE::NAME[,NAME], FILE:FIRST-LAST, or a bare method name")
    parser.add_argument("--transitive", type=int, default=0, metavar="N", help="follow called Game methods N levels")
    parser.add_argument("--alias", action="append", default=[], metavar="NAME",
                        help="also count `NAME.field` as a use of Game (a closure parameter holding the game)")
    parser.add_argument("--json", action="store_true", help="print JSON instead of text")
    args = parser.parse_args(argv)
    report = build(args)
    if args.json:
        json.dump(report, sys.stdout, indent=2)
        print()
        return 0
    for entry in report["targets"]:
        print(render(entry))
    if "combined" in report:
        print(render(report["combined"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
