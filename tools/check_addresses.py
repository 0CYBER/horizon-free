#!/usr/bin/env python3
"""Check that built-in outside addresses stay where docs/layers.md says (the "Built-in addresses" section).

Reads the string literals of shipped Rust (every crate tools/layers.json doesn't call a tool, and the launcher),
without tests, examples or #[cfg(test)] modules, and finds the hosts in them: a URL's host anywhere in a string,
or a whole string that is a host name (`play.playhorizon.dev`, `stun.l.google.com:19302`). Placeholders
(`example.com`, `*.test`, `*.invalid`, `localhost`) and loopback, private and documentation IPs are skipped.
tools/addresses.json says where each host may be:
- `homes`: the files that hold the Horizon layer's addresses; anything goes there;
- `vanilla`: the endpoints vanilla itself contacts; allowed anywhere;
- `not_addresses`: strings that look like hosts but are not contacted (a suffix matcher, a translation key);
- `allowed`: today's addresses outside a home, "path host". They only go away: with --base, a branch may move
  an entry to another file but may not add a host nor grow the list. A changed sync-baseline declaration,
  verified by merge_base.sync_provenance, may restore exact rows from that upstream commit's inventory;
  only those newly restored rows are excluded from the growth comparison.

    python3 tools/check_addresses.py                     # the check
    python3 tools/check_addresses.py --base origin/main  # and nothing added since the base
    python3 tools/check_addresses.py --list              # every host found, with its files
"""

import argparse
import collections
import ipaddress
import json
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
CONFIG_PATH = "tools/addresses.json"
DOC = "docs/layers.md#built-in-addresses"
# A whole string is taken for a host name only with one of these endings: translation keys end in words
# (`options.online`, `mco.info`), so the common words are left out.
TLDS = "com|net|org|dev|gg|io|ms|me|app|xyz|co|za|uk|de|fr|tv|eu|ch|nl|ru|jp|au|br|ca|us|sh|cc|ai"
HOST = r"(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)+(?:" + TLDS + r")"
URL = re.compile(r"\b(?:https?|wss?)://(?:[^/@\s\"]*@)?(\[[0-9a-f:]+\]|[a-z0-9.-]+)", re.I)
WHOLE = re.compile(r"^(" + HOST + r"|(?:\d{1,3}\.){3}\d{1,3})(?::\d{1,5})?(?:/\S*)?$", re.I)
# Files with neither a URL nor a whole-string host skip the tokenizer. Rust's hex/Unicode escapes and
# line continuations can build or split an ASCII host while the raw source has no plain host and no
# "://", so those escape signatures also send a file to the tokenizer (the simple escapes cannot form
# a host character). This is a cheap hint, not a decode: raw strings, comments and #[cfg(test)] items
# are still excluded downstream.
_ESCAPE_HINT = r"\\x[0-9a-fA-F]{2}|\\u\{[0-9a-fA-F_]+\}|\\\r?\n"
MAYBE_HOST = re.compile(r"://|\"" + HOST + r"(?::\d+)?[\"/]|\"(?:\d{1,3}\.){3}\d{1,3}|" + _ESCAPE_HINT, re.I)
TEST_MOD = re.compile(r"#\[cfg\((?:all\()?test\b[^\]]*\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*;")
PLACEHOLDER = re.compile(r"(^|\.)(example(\.(com|net|org))?|test|invalid|localhost|local)$")
SKIP_DIRS = {"tests", "examples", "benches", "fixtures", "target"}

# Rust's ordinary-string escapes. Raw strings are decoded nowhere (see tokenize); these apply only to `"..."`.
_RUST_SIMPLE_ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "0": "\0", "\\": "\\", '"': '"', "'": "'"}
_RUST_HEX = "0123456789abcdefABCDEF"
_RUST_WS = " \t\r\n\x0b\x0c"


def _decode_rust_string(raw):
    """Decoded value of an ordinary Rust string literal's contents.

    Covers every escape valid in a shipped Rust string: \\n \\r \\t \\0 \\\\ \\" \\', ASCII \\xNN,
    \\u{...} (a hex digit followed by hex digits/underscores) and backslash-newline whitespace continuation.
    A sequence this decoder does not recognize is kept verbatim: invalid Rust cannot be in shipped
    source, and leaving it visible is safer than silently rewriting what the scanner read.
    """
    out, i, n = [], 0, len(raw)
    while i < n:
        c = raw[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        if i + 1 >= n:
            out.append(c)
            i += 1
            continue
        e = raw[i + 1]
        if e in _RUST_SIMPLE_ESCAPES:
            out.append(_RUST_SIMPLE_ESCAPES[e])
            i += 2
        elif e == "x" and i + 3 < n and raw[i + 2] in _RUST_HEX and raw[i + 3] in _RUST_HEX:
            value = int(raw[i + 2:i + 4], 16)
            if value < 0x80:  # Rust requires ASCII for \xNN in a string literal.
                out.append(chr(value))
            else:
                out.append(raw[i:i + 4])
            i += 4
        elif raw.startswith("u{", i + 1):
            close = raw.find("}", i + 3)
            body = raw[i + 3:close] if close > 0 else ""
            # rustc accepts a hex digit followed by any mix of hex digits and underscores: internal
            # repeats and a trailing underscore are valid, a leading underscore is not (verified with
            # rustc 1.99.1). Ignore underscores when counting the 1..6 actual hex digits to validate.
            digits = None
            if body and body[0] in _RUST_HEX and all(d == "_" or d in _RUST_HEX for d in body):
                candidate = body.replace("_", "")
                if 1 <= len(candidate) <= 6:
                    digits = candidate
            if digits:
                value = int(digits, 16)
                if value <= 0x10FFFF and not 0xD800 <= value <= 0xDFFF:
                    out.append(chr(value))
                    i = close + 1
                    continue
            end = close + 1 if close > 0 else n
            out.append(raw[i:end])  # malformed or out-of-range: keep it visible
            i = end
        elif e in "\r\n":
            # Line continuation: the backslash, its newline and the following ASCII whitespace vanish.
            j = i + 2
            if e == "\r" and j < n and raw[j] == "\n":
                j += 1
            while j < n and raw[j] in _RUST_WS:
                j += 1
            i = j
        else:
            out.append(raw[i:i + 2])
            i += 2
    return "".join(out)


def tokenize(text):
    """(literals, mask): each string literal as (offset, content), and the source with comments and string contents
    blanked (newlines kept), so braces and attributes can be matched on code only."""
    out, mask = [], list(text)
    i, n = 0, len(text)

    def blank(a, b):
        for k in range(a, b):
            if mask[k] != "\n":
                mask[k] = " "

    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c in "rb" and re.match(r'b?r#*"', text[i:i + 68]) and not (i and (text[i - 1].isalnum() or text[i - 1] == "_")):
            m = re.match(r'b?r(#*)"', text[i:])
            hashes = m.group(1)
            start = i + len(m.group(0))
            end = text.find('"' + hashes, start)
            end = n if end < 0 else end
            out.append((start, text[start:end]))
            blank(start, end)
            i = end + 1 + len(hashes)
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            # Keep the source offset (for #[cfg(test)] ranges) and the masked span (for brace
            # matching), but hand the scanner the literal's decoded value, not its spelling.
            out.append((i + 1, _decode_rust_string(text[i + 1:j])))
            blank(i + 1, j)
            i = j + 1
        elif c == "'":
            # A char literal ('"', '\'', 'a') or a lifetime ('a).
            m = re.match(r"'(\\.[^']*|[^\\'])'", text[i:i + 12])
            if m:
                blank(i, i + len(m.group(0)))
                i += len(m.group(0))
            else:
                i += 1
        else:
            i += 1
    return out, "".join(mask)


TEST_ATTR = re.compile(r"#\[cfg\((?:all\()?test\b[^\]]*\]")
ATTR = re.compile(r"\s*#\[[^\]]*\]")
PATH_ATTR = re.compile(r'#\[path\s*=\s*"([^"]+)"\]')


def test_items(text, mask):
    """([(start, end)], [test module file names]) for the #[cfg(test)] items of a source file."""
    ranges, files = [], []
    for m in TEST_ATTR.finditer(mask):
        j, path = m.end(), None
        while True:
            a = ATTR.match(mask, j)
            if not a:
                break
            p = PATH_ATTR.search(text[a.start():a.end()])
            path = p.group(1) if p else path
            j = a.end()
        mod = re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;", mask[j:])
        if mod:
            files.append(path or mod.group(1))
            ranges.append((m.start(), j + mod.end()))
            continue
        depth, k = 0, j
        while k < len(mask):
            ch = mask[k]
            if ch in "([":
                depth += 1
            elif ch in ")]":
                depth -= 1
            elif ch == ";" and depth == 0:
                break
            elif ch == "{" and depth == 0:
                braces = 1
                k += 1
                while k < len(mask) and braces:
                    braces += {"{": 1, "}": -1}.get(mask[k], 0)
                    k += 1
                break
            k += 1
        ranges.append((m.start(), k))
    return ranges, files


def module_dir(path):
    """Where `mod x;` in this file looks for x.rs."""
    if path.name in ("lib.rs", "main.rs", "mod.rs"):
        return path.parent
    return path.parent / path.stem


def shipped_files():
    """The shipped Rust files, relative to the root, minus test modules."""
    layers = json.loads((ROOT / "tools" / "layers.json").read_text())["crates"]
    roots = [ROOT / "launcher" / "src"]
    for crate in sorted((ROOT / "crates").iterdir()):
        if (crate / "src").is_dir() and layers.get(crate.name) != "tool":
            roots.append(crate / "src")
    files = []
    for root in roots:
        for dirpath, dirnames, filenames in os.walk(root):
            dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
            for name in sorted(filenames):
                path = Path(dirpath) / name
                if name.endswith(".rs") and not (name == "tests.rs" or name.endswith("_tests.rs")):
                    files.append(path)
    return files


def ip_skipped(host):
    try:
        ip = ipaddress.ip_address(host.strip("[]"))
    except ValueError:
        return False
    documentation = any(ip in ipaddress.ip_network(n) for n in ("192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24"))
    return ip.is_loopback or ip.is_private or ip.is_unspecified or ip.is_link_local or documentation


def hosts(content):
    found = [m.group(1) for m in URL.finditer(content)]
    whole = WHOLE.match(content.strip())
    if whole and content == content.strip():
        found.append(whole.group(1))
    out = []
    for host in found:
        host = host.lower().rstrip(".")
        if "." not in host and ":" not in host:
            continue
        if PLACEHOLDER.search(host) or ".example." in host or ip_skipped(host):
            continue
        out.append(host)
    return out


def scan(files=None):
    """{(path, host)} for every host in shipped code."""
    if files is None:
        files = shipped_files()
    parsed, test_paths = {}, set()
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        if not MAYBE_HOST.search(text) and not TEST_MOD.search(text):
            continue
        strings, mask = tokenize(text)
        ranges, test_mods = test_items(text, mask)
        parsed[path] = [content for offset, content in strings if not any(a <= offset < b for a, b in ranges)]
        for name in test_mods:
            if name.endswith(".rs"):
                test_paths.add((path.parent / name).resolve())
            else:
                test_paths.add((module_dir(path) / f"{name}.rs").resolve())
                test_paths.add((module_dir(path) / name / "mod.rs").resolve())
    found = set()
    for path, strings in parsed.items():
        if path.resolve() in test_paths:
            continue
        rel = path.relative_to(ROOT).as_posix()
        for content in strings:
            for host in hosts(content):
                found.add((rel, host))
    return found


def check(config, found, base=None, upstream_rows=frozenset()):
    """(problems, notes)."""
    homes = set(config.get("homes", []))
    vanilla = set(config.get("vanilla", []))
    not_addresses = config.get("not_addresses", {})
    allowed = {tuple(entry.split(" ", 1)) for entry in config.get("allowed", [])}
    problems, used = [], set()
    for rel, host in sorted(found):
        if rel in homes or host in vanilla or any(host == h or host.endswith("." + h) for h in vanilla):
            continue
        if host in not_addresses:
            continue
        if (rel, host) in allowed:
            used.add((rel, host))
            continue
        problems.append(f"{rel}: {host} is a built-in address outside the address list. A vanilla endpoint goes in "
                        f"'vanilla', anything else in the Horizon layer's address home ({DOC})")
    if base is not None:
        before = {tuple(entry.split(" ", 1)) for entry in base.get("allowed", [])}
        restored = (allowed - before) & upstream_rows
        effective = allowed - restored
        new_hosts = {host for _, host in effective} - {host for _, host in before}
        for host in sorted(new_hosts):
            problems.append(f"{host}: added to 'allowed' in {CONFIG_PATH}; new addresses go in a home ({DOC})")
        if len(effective) > len(before):
            problems.append(f"'allowed' in {CONFIG_PATH} grew from {len(before)} to {len(allowed)} entries; it only "
                            f"shrinks ({DOC})")
    notes = [f"{rel} {host}: no longer there; you can remove it from 'allowed' in {CONFIG_PATH}"
             for rel, host in sorted(allowed - used)]
    return problems, notes


def base_config(base):
    """tools/addresses.json where the branch left `base` (their merge base), as check_layers.base_config."""
    merge_base = subprocess.run(["git", "merge-base", base, "HEAD"], capture_output=True, text=True, cwd=ROOT)
    if merge_base.returncode != 0:
        return None
    shown = subprocess.run(["git", "show", f"{merge_base.stdout.strip()}:{CONFIG_PATH}"], capture_output=True,
                           text=True, cwd=ROOT)
    return json.loads(shown.stdout) if shown.returncode == 0 else None


def _merge_base():
    if __package__:
        from . import merge_base
    else:
        import merge_base
    return merge_base


def restored_upstream_rows(base):
    """Allow exact upstream rows only for a changed, verified sync declaration."""
    config, errors = _merge_base().verified_sync_config(base, CONFIG_PATH)
    rows = {tuple(row.split(" ", 1)) for row in config.get("allowed", [])} if config is not None else set()
    return rows, errors


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--base", help="refuse addresses added to 'allowed' since the branch left this git ref")
    parser.add_argument("--list", action="store_true", help="print every host found and its files")
    parser.add_argument("--pull-request-merge", action="store_true", help=_merge_base().PULL_REQUEST_MERGE_HELP)
    args = parser.parse_args(argv)
    if args.base:
        args.base, error = _merge_base().pull_request_base(args.base, args.pull_request_merge)
        if error:
            print(f"Address check failed: {error}")
            return 1
    config = json.loads((ROOT / CONFIG_PATH).read_text())
    found = scan()
    if args.list:
        by_host = collections.defaultdict(list)
        for rel, host in found:
            by_host[host].append(rel)
        for host in sorted(by_host):
            print(f"{host}: {', '.join(sorted(by_host[host]))}")
        return 0
    base = base_config(args.base) if args.base else None
    problems, notes = check(config, found, base)
    if args.base and (problems or config != base):
        # Only a violation, or an inventory that differs from the base's, can need rows a verified sync restored:
        # every other run stays off the network.
        upstream_rows, provenance = restored_upstream_rows(args.base)
        if upstream_rows or provenance:
            problems, notes = check(config, found, base, upstream_rows)
        problems = provenance + problems
    for note in notes:
        print(f"note: {note}")
    if problems:
        print(f"Address check failed ({DOC}):")
        for problem in problems:
            print(f"  {problem}")
        return 1
    print(f"Address check passed: {len({h for _, h in found})} hosts, "
          f"{len(config.get('allowed', []))} entries outside a home left to move ({DOC}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
