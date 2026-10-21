#!/usr/bin/env python3
"""Convert vanilla's BlockStateData.java into a plain data file.

Reads  .local/minecraft/<pinned version>/sources/server/net/minecraft/util/datafix/fixes/BlockStateData.java
Writes crates/horizon-world/src/datafix/block_state_data.txt

Output: UTF-8, '#' header lines, then one line per register() call, in the order the
calls execute (the static initializer order of the bootstrap* methods), tab separated:

    ID <TAB> newName <TAB> newProps <TAB> legacyName1 <TAB> legacyProps1 <TAB> legacyName2 <TAB> legacyProps2 ...

Props are `key=value` pairs joined by `;`, sorted by key; empty string when there are none
(including an explicit `Map.of()`).

Usage: tools/gen_block_state_data.py [path/to/BlockStateData.java] [-o output.txt]
Re-runnable: the output only depends on the Java file.
"""
import argparse
import re
import sys
from pathlib import Path

try:
    from . import lab
except ImportError:
    import lab

REPO = Path(__file__).resolve().parent.parent
DEFAULT_JAVA = lab.GAME / "sources/server/net/minecraft/util/datafix/fixes/BlockStateData.java"
DEFAULT_OUT = REPO / "crates/horizon-world/src/datafix/block_state_data.txt"

TOKEN_RE = re.compile(
    r"""
      (?P<ws>\s+)
    | (?P<lcomment>//[^\n]*)
    | (?P<bcomment>/\*.*?\*/)
    | (?P<string>"(?:[^"\\]|\\.)*")
    | (?P<ident>[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)
    | (?P<number>-?[0-9]+)
    | (?P<punct>[(),;{}=<>?\[\].@!\-+*/%|&~:^'])
    """,
    re.X | re.S,
)


def tokenize(text):
    pos = 0
    out = []
    while pos < len(text):
        m = TOKEN_RE.match(text, pos)
        if not m:
            raise SystemExit(f"tokenize error at offset {pos}: {text[pos:pos+60]!r}")
        pos = m.end()
        kind = m.lastgroup
        if kind in ("ws", "lcomment", "bcomment"):
            continue
        out.append((kind, m.group()))
    return out


def unquote(s):
    inner = s[1:-1]
    if "\\" in inner:
        raise SystemExit(f"unexpected escape in string {s}")
    return inner


class Parser:
    """Parses a token list holding a call expression tree: ident(args) / strings / numbers / idents."""

    def __init__(self, toks, consts):
        self.toks = toks
        self.i = 0
        self.consts = consts

    def peek(self):
        return self.toks[self.i] if self.i < len(self.toks) else (None, None)

    def next(self):
        t = self.toks[self.i]
        self.i += 1
        return t

    def expect(self, val):
        k, v = self.next()
        if v != val:
            raise SystemExit(f"expected {val!r} got {v!r} at token {self.i}")

    def parse_args(self):
        """After '(' consumed: return list of parsed args up to ')'."""
        args = []
        if self.peek()[1] == ")":
            self.next()
            return args
        while True:
            args.append(self.parse_expr())
            k, v = self.next()
            if v == ")":
                return args
            if v != ",":
                raise SystemExit(f"expected ',' or ')' got {v!r} at token {self.i}")

    def parse_expr(self):
        k, v = self.next()
        if k == "string":
            return ("str", unquote(v))
        if k == "number":
            return ("int", int(v))
        if k == "ident":
            if self.peek()[1] == "(":
                self.next()
                args = self.parse_args()
                return ("call", v, args)
            return ("name", v)
        raise SystemExit(f"unexpected token {v!r} at {self.i}")


def eval_map(expr, consts):
    """Evaluate a Map.of(...) call or a constant name into a dict."""
    if expr[0] == "name":
        if expr[1] not in consts:
            raise SystemExit(f"unresolved map constant {expr[1]}")
        return dict(consts[expr[1]])
    if expr[0] == "call" and expr[1] == "Map.of":
        args = expr[2]
        if len(args) % 2:
            raise SystemExit("odd Map.of args")
        d = {}
        for a in range(0, len(args), 2):
            k, v = args[a], args[a + 1]
            if k[0] != "str" or v[0] != "str":
                raise SystemExit(f"non-string Map.of entry {k} {v}")
            if k[1] in d:
                raise SystemExit(f"duplicate key {k[1]}")
            d[k[1]] = v[1]
        return d
    raise SystemExit(f"cannot evaluate map expr {expr}")


def fmt_props(d):
    for k, v in d.items():
        if not re.fullmatch(r"[a-z0-9_]+", k) or not re.fullmatch(r"[A-Za-z0-9_]+", v):
            raise SystemExit(f"unexpected prop chars {k}={v}")
    return ";".join(f"{k}={d[k]}" for k in sorted(d))


def eval_create(expr, consts):
    if expr[0] != "call" or expr[1] != "create":
        raise SystemExit(f"expected create(...) got {expr}")
    args = expr[2]
    if not args or args[0][0] != "str":
        raise SystemExit(f"create without name: {expr}")
    name = args[0][1]
    if not re.fullmatch(r"[a-z0-9_:]+", name) and name != "%%FILTER_ME%%":
        raise SystemExit(f"bad block name {name!r}")
    if len(args) == 1:
        return name, ""
    if len(args) == 2:
        return name, fmt_props(eval_map(args[1], consts))
    raise SystemExit(f"create with {len(args)} args")


def matching_brace(toks, i):
    """toks[i] is '{'; return index of matching '}'."""
    depth = 0
    for j in range(i, len(toks)):
        if toks[j][1] == "{":
            depth += 1
        elif toks[j][1] == "}":
            depth -= 1
            if depth == 0:
                return j
    raise SystemExit("unbalanced braces")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("java", nargs="?", default=str(DEFAULT_JAVA))
    ap.add_argument("-o", "--output", default=str(DEFAULT_OUT))
    a = ap.parse_args()
    java_path = Path(a.java)
    text = java_path.read_text(encoding="utf-8")

    toks = tokenize(text)

    # ---- map constants: `private static final Map<String, String> NAME = <expr>;`
    consts = {}
    i = 0
    n = len(toks)
    while i < n:
        if (
            toks[i][1] == "Map"
            and toks[i + 1][1] == "<"
            and i >= 3
            and toks[i - 1][1] == "final"
            and toks[i - 2][1] == "static"
        ):
            # Map < String , String > NAME = expr ;
            j = i + 1
            while toks[j][1] != ">":
                j += 1
            name = toks[j + 1][1]
            assert toks[j + 2][1] == "=", toks[j : j + 4]
            p = Parser(toks[j + 3 :], consts)
            expr = p.parse_expr()
            assert toks[j + 3 + p.i][1] == ";", toks[j + 3 + p.i]
            if name in consts:
                raise SystemExit(f"duplicate const {name}")
            consts[name] = eval_map(expr, consts)
            i = j + 3 + p.i
        else:
            i += 1

    # ---- methods: find `void bootstrapX ( ) {` bodies
    bodies = {}
    for i in range(n - 5):
        if (
            toks[i][1] == "void"
            and toks[i + 1][1].startswith("bootstrap")
            and toks[i + 2][1] == "("
            and toks[i + 3][1] == ")"
            and toks[i + 4][1] == "{"
        ):
            end = matching_brace(toks, i + 4)
            bodies[toks[i + 1][1]] = toks[i + 5 : end]

    # ---- static initializer order
    order = None
    for i in range(n - 1):
        if toks[i][1] == "static" and toks[i + 1][1] == "{":
            end = matching_brace(toks, i + 1)
            seq = []
            k = i + 2
            while k < end:
                if toks[k][0] == "ident" and toks[k][1].startswith("bootstrap") and toks[k + 1][1] == "(":
                    seq.append(toks[k][1])
                    k += 3
                else:
                    k += 1
            order = seq
    if not order:
        raise SystemExit("static initializer not found")
    if set(order) != set(bodies) or len(order) != len(set(order)):
        raise SystemExit(f"bootstrap methods {sorted(bodies)} vs called {order}")

    # ---- parse register calls
    lines = []
    total_register = 0
    total_create = 0
    total_legacy = 0
    max_id = -1
    for m in order:
        body = bodies[m]
        k = 0
        while k < len(body):
            t = body[k]
            if t[0] == "ident" and t[1] == "register" and body[k + 1][1] == "(":
                p = Parser(body[k + 2 :], consts)
                args = p.parse_args()
                k += 2 + p.i
                if body[k][1] != ";":
                    raise SystemExit(f"{m}: register not followed by ';'")
                total_register += 1
                if args[0][0] != "int":
                    raise SystemExit(f"{m}: register id not int: {args[0]}")
                rid = args[0][1]
                if len(args) < 2:
                    raise SystemExit(f"{m}: register({rid}) without new tag")
                fields = [str(rid)]
                for c in args[1:]:
                    name, props = eval_create(c, consts)
                    total_create += 1
                    fields += [name, props]
                total_legacy += len(args) - 2
                max_id = max(max_id, rid)
                lines.append("\t".join(fields))
            else:
                # anything other than register(...) ; in a bootstrap body is unexpected
                raise SystemExit(f"{m}: unexpected token {t} in body")
                k += 1
            k += 1  # skip ';'

    # ---- independent validation: count `register(` occurrences inside bootstrap bodies (text based)
    src_lines = text.split("\n")
    in_body = False
    textual = 0
    for ln in src_lines:
        if re.match(r"\s*private static void bootstrap\w*\(\)\s*\{", ln):
            in_body = True
            continue
        if in_body:
            if re.match(r"   \}\s*$", ln):
                in_body = False
                continue
            textual += len(re.findall(r"\bregister\(", ln))
    if textual != total_register or textual != len(lines):
        raise SystemExit(f"register count mismatch: textual {textual}, parsed {total_register}, lines {len(lines)}")
    textual_create = 0
    in_body = False
    for ln in src_lines:
        if re.match(r"\s*private static void bootstrap\w*\(\)\s*\{", ln):
            in_body = True
            continue
        if in_body:
            if re.match(r"   \}\s*$", ln):
                in_body = False
                continue
            textual_create += len(re.findall(r"\bcreate\(", ln))
    if textual_create != total_create:
        raise SystemExit(f"create count mismatch: textual {textual_create}, parsed {total_create}")

    header = [
        f"# Generated by tools/gen_block_state_data.py from vanilla {lab.VERSION} BlockStateData.java. Do not edit.",
        "# One line per register(ID, new, legacy...) call, in execution order of the static initializer",
        "# (bootstrap0, 1, 2, 3_1, 3_2, 4 .. 9, A .. F).",
        "# Format (tab separated): ID <TAB> newName <TAB> newProps [<TAB> legacyName <TAB> legacyProps]...",
        "# ID is the decimal 1.12 (blockId << 4 | data) id; newName/newProps is the 1.13 block state;",
        "# each legacy pair is a 1.12 Name + Properties tag that maps to ID.",
        "# Props: key=value pairs joined by ';', sorted by key; empty string when there are none.",
        "# newName may be the literal %%FILTER_ME%% (vanilla FILTER_ME marker, the skull ids 2304..2317).",
    ]
    out = Path(a.output)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(header + lines) + "\n", encoding="utf-8")

    print(f"java:            {java_path}")
    print(f"output:          {out}")
    print(f"map constants:   {len(consts)}")
    print(f"bootstrap order: {' '.join(order)}")
    print(f"register calls:  {total_register} (grep count {textual})")
    print(f"data lines:      {len(lines)}")
    print(f"create() calls:  {total_create} (all resolved; grep count {textual_create})")
    print(f"legacy tags:     {total_legacy}")
    print(f"highest ID:      {max_id}")


if __name__ == "__main__":
    sys.exit(main())
