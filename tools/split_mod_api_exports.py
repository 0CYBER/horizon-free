#!/usr/bin/env python3
"""One-shot arismoko/horizon#1456 move; --check BASE proves the scoped statements and residual sources."""
import argparse
from collections import defaultdict
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SRC = Path("crates/horizon-mod-vocab/src")
INCLUDE = 'include!(concat!(env!("OUT_DIR"), "/mod_api_exports_{}.rs"));\n'


def extract(text, private=False):
    fragments = defaultdict(lambda: defaultdict(list))
    pattern = re.compile(
        r"(?P<block>(?:(?P<indent>    |)(?:#\[[^\n]*|//[^\n]*)\n)*"
        r"(?P<space>    |)(?P<item>pub(?:\(crate\))? use [\s\S]*?;\n|"
        r"(?:pub(?:\(crate\))? )?mod \w+;\n))", re.M)
    edits = []
    scopes = {}
    if private:
        for m in re.finditer(r"^pub mod (\w+) \{[\s\S]*?^\}", text, re.M):
            scopes[m.group(1)] = (m.start(), m.end())
    for m in pattern.finditer(text):
        # A statement begins at its line's start, never inside an inline impl or function.
        if m.start() and text[m.start() - 1] != "\n":
            continue
        indent = m.group("space")
        scope = "private" if private else "root"
        if indent:
            enclosing = [name for name, (a, b) in scopes.items() if a < m.start() < b]
            if not enclosing:
                continue
            scope += "_" + enclosing[0]
        item = m.group("item")
        if " use " in item:
            path = item.split(" use ", 1)[1]
            area = re.sub(r"^(crate|self)::", "", path).split("::")[0]
            area = ("part_table" if "part_table" in path else "phase") if area.startswith("{") else re.match(r"\w+", area).group()
            area = {"modblockentityviewnew": "containers", "reloadfilesnew": "reload"}.get(area, area)
        else:
            area = re.search(r"mod (\w+);", item).group(1)
        block = m.group("block")
        if indent:
            block = "\n".join(line[4:] if line.startswith("    ") else line
                              for line in block.split("\n"))
        fragments[area][scope].append(block)
        edits.append((m.start(), m.end()))
    for a, b in reversed(edits):
        text = text[:a] + text[b:]
    used = {scope for scopes in fragments.values() for scope in scopes}
    if private:
        for name in scopes:
            scope = "private_" + name
            if scope in used:
                text = text.replace(f"pub mod {name} {{\n", f"pub mod {name} {{\n    " + INCLUDE.format(scope))
    text += "\n" + INCLUDE.format("private" if private else "root")
    return text, fragments


def render(scopes):
    text = "".join(scopes.get("root", []))
    private = "".join(scopes.get("private", []))
    for scope, blocks in sorted(scopes.items()):
        if scope.startswith("private_"):
            body = "".join(blocks)
            private += "mod " + scope.removeprefix("private_") + " {\n" + body + "}\n"
    if private:
        text += "mod __private {\n" + private + "}\n"
    return text


def formatted(text):
    # Declared edits: indentation, blank lines, wrapping and sorting adjacent imports by rustfmt.
    return subprocess.check_output(
        ["rustfmt", "--edition", "2024", "--config", "skip_children=true", "--emit", "stdout"],
        input=text.encode()).decode()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", metavar="BASE")
    args = parser.parse_args()
    all_fragments = defaultdict(lambda: defaultdict(list))
    for name in ["lib.rs", "__private.rs"]:
        path = SRC / name
        before = (subprocess.check_output(["git", "show", f"{args.check}:{path}"], cwd=ROOT).decode()
                  if args.check else (ROOT / path).read_text())
        after, fragments = extract(before, name == "__private.rs")
        if args.check:
            assert formatted(after) == (ROOT / path).read_text(), path
        else:
            (ROOT / path).write_text(formatted(after))
        for area, scopes in fragments.items():
            for scope, blocks in scopes.items():
                all_fragments[area][scope].extend(blocks)
    for area, scopes in all_fragments.items():
        path = ROOT / SRC / "exports" / f"{area}.rs"
        expected = render(scopes)
        if args.check:
            assert formatted(expected) == path.read_text(), path
        else:
            path.parent.mkdir(exist_ok=True)
            path.write_text(formatted(expected))
    print(f"Verified {len(all_fragments)} area fragments and both residual sources" if args.check
          else f"Moved declarations and exports into {len(all_fragments)} area fragments")


if __name__ == "__main__":
    main()
