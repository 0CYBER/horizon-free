#!/usr/bin/env python3
"""The port ledger: every vanilla method next to the Horizon functions that port it, both ways, and the gaps.

vanilla_map.py maps vanilla classes to the files that name them. The ledger goes one level down and both
ways: each vanilla method (from the decompiled sources) to the Rust functions, with their line spans, whose
comments cite it (`Class.method`, `Class::method`, `Class#method`, as vanilla_map reads them), and each
Rust function back to the vanilla methods it ports. Methods nobody cites get a second, inferred match from
what both sides share that is hard to share by chance: rare numbers (0.458, 72000) and game ids (a sound,
block, particle or item, `SoundEvents.ENTITY_SHULKER_BULLET_HIT` against "entity.shulker_bullet.hit").

Both sides' call graphs then show where a port exists but is not wired like vanilla, the kind of bug a
method-by-method reading misses:

- seam: a vanilla method ported on both paths (the in-frame game and the server-client side,
  game/remote*.rs, or the server, game/lan*.rs), one of which reaches the port of a callee and the other
  doesn't;
- callee: a ported vanilla method whose ported callee no Rust counterpart reaches;
- client: a vanilla effect only the client's copy draws (`addParticle`, `playLocalSound`,
  `itemUsed`, a `isClientSide()` branch) whose port nothing on the server-client side (game/remote*.rs)
  reaches;
- class: a whole vanilla class nothing ports while most of its package is ported (a renderer, a sound
  instance, a goal: the game reaches those by registration, so no call graph shows them);
- stale: a citation whose class or method doesn't exist in the pinned version;
- unported: vanilla methods with no port, ranked by how many vanilla methods call them.

    python3 tools/port_ledger.py build                         # index into .local/port-ledger/ledger.json
    python3 tools/port_ledger.py where KineticWeapon.damageEntities
    python3 tools/port_ledger.py where ShulkerBulletRenderer   # a whole class
    python3 tools/port_ledger.py what crates/horizon-client/src/game/spear.rs:150
    python3 tools/port_ledger.py class Shulker                 # every method: ported where, or not
    python3 tools/port_ledger.py coverage [--package world/entity] [--depth 3]
    python3 tools/port_ledger.py gaps [--kind seam] [--package world/entity] [--limit 50]
    python3 tools/port_ledger.py report                        # .local/port-ledger/report.md

Add --json to any query for machine output. The sources default to the lab's decompiled sources
(.local/minecraft/<version>/sources/{client,server}, tools/lab.py decompile); --sources takes any
directory of decompiled Java instead. Queries read the last build; `build` again after pulling.
"""

import argparse
from collections import Counter, defaultdict
from concurrent.futures import ProcessPoolExecutor
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / ".local" / "port-ledger"
LEDGER = OUT / "ledger.json"

JAVA_KEYWORDS = {
    "if", "for", "while", "switch", "catch", "synchronized", "return", "new", "throw", "else", "do", "try",
    "case", "assert", "super", "this", "instanceof", "yield",
}
RUST_KEYWORDS = {
    "if", "while", "match", "for", "loop", "return", "fn", "let", "move", "in", "as", "else", "unsafe",
    "where", "impl", "dyn", "ref", "mut", "async", "await", "break", "continue", "use", "pub", "crate",
    "self", "super", "struct", "enum", "type", "const", "static", "trait", "mod",
}
# Calls too generic to mean anything on their own (collections, math, builders, logging).
GENERIC_CALLS = {
    "get", "set", "add", "put", "remove", "contains", "size", "len", "is_empty", "isEmpty", "map", "filter",
    "iter", "into_iter", "collect", "unwrap", "unwrap_or", "clone", "to_string", "from", "into", "new",
    "push", "pop", "insert", "min", "max", "abs", "floor", "ceil", "sqrt", "sin", "cos", "clamp", "lerp",
    "equals", "hashCode", "toString", "of", "copy", "apply", "accept", "test", "run", "call", "value",
    "values", "keySet", "entrySet", "stream", "forEach", "orElse", "isPresent", "ifPresent", "getValue",
    "getKey", "length", "append", "format", "is_some", "is_none", "unwrap_or_default", "as_ref", "as_mut",
    "extend", "retain", "find", "any", "all", "position", "first", "last", "sort", "dedup", "default",
}
# Numbers that say nothing: indices, common scales, angles, tick rates.
TRIVIAL_NUMBERS = {
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "15", "16", "20", "24", "32", "40",
    "50", "60", "64", "100", "127", "128", "180", "200", "255", "256", "360", "1000", "0.0", "0.5", "1.0",
    "2.0", "0.1", "0.2", "0.25", "0.75", "0.3", "0.4", "0.6", "0.8", "0.9", "1.5", "3.0", "4.0", "10.0",
    "16.0", "20.0", "90", "90.0", "180.0", "360.0", "0.01", "0.001", "100.0", "-1", "1.0e-5", "1.0e-7",
}
# Vanilla classes whose UPPER_CASE fields name game ids (SoundEvents.ELYTRA_FLYING is "item.elytra.flying").
ID_HOLDERS = {
    "SoundEvents", "Items", "Blocks", "ParticleTypes", "EntityType", "MobEffects", "Enchantments",
    "Attributes", "DamageTypes", "BlockTags", "ItemTags", "EntityTypeTags", "DamageTypeTags", "GameRules",
    "CriteriaTriggers", "Stats", "MemoryModuleType", "SensorType", "Activity", "PoiTypes", "VillagerProfession",
    "BlockEntityType", "MenuType", "RecipeType", "DataComponents", "Fluids", "Biomes", "Structures",
    "LootTables", "BuiltInLootTables", "GameEvent", "Instruments", "PaintingVariants", "BannerPatterns",
}
# Vanilla calls whose effect only the client's copy of the world shows.
CLIENT_EFFECTS = re.compile(
    r"\.addParticle\(|\.addAlwaysVisibleParticle\(|\.playLocalSound\(|\bitemUsed\(|\.isClientSide\(\)"
)
SKIP_PACKAGES = (
    "net/minecraft/gametest/", "net/minecraft/data/", "net/minecraft/util/datafix/", "net/minecraft/obfuscate/",
    "com/mojang/realmsclient/", "net/minecraft/client/telemetry/", "net/minecraft/util/profiling/",
    "com/mojang/blaze3d/", "com/mojang/renderpearl/", "net/minecraft/server/jsonrpc/", "net/minecraft/util/filefix/",
    "net/minecraft/client/data/",
)

CITE = re.compile(r"\b([A-Z][A-Za-z0-9]*(?:\.[A-Z][A-Za-z0-9]*)*)(?:(?:\.|::|#)([A-Za-z_][A-Za-z0-9_]*))?")
JAVA_CLASS = re.compile(r"\b(class|interface|enum|record|@interface)\s+([A-Za-z_]\w*)")
JAVA_EXTENDS = re.compile(r"\bextends\s+([\w.]+)")
JAVA_IMPLEMENTS = re.compile(r"\bimplements\s+([\w.,\s<>?]+)")
JAVA_METHOD = re.compile(r"\b([A-Za-z_]\w*)\s*\([^()]*(?:\([^()]*\)[^()]*)*\)\s*(?:throws\s+[\w.,\s]+)?$")
JAVA_CALL = re.compile(r"(?:\b([A-Za-z_]\w*)\s*(?:\.|::)\s*|(\.|::)\s*)?\b([a-z][A-Za-z0-9_]*)\s*\(")
JAVA_METHOD_REF = re.compile(r"\b([A-Za-z_]\w*)::([a-z]\w*)\b")
JAVA_NEW = re.compile(r"\bnew\s+([A-Z][\w.]*)\s*[(<]")
JAVA_TYPED = re.compile(r"\b([A-Z][\w.]*)(?:<[^;=(){}]*?>)?(?:\[\])*\s+([a-z]\w*)\s*(?=[=;:,)])")
JAVA_ID = re.compile(r"\b(" + "|".join(sorted(ID_HOLDERS)) + r")\.([A-Z][A-Z0-9_]+)\b")
NUMBER = re.compile(r"(?<![\w.])-?(?:\d[\d_]*\.\d[\d_]*(?:[eE][-+]?\d+)?|\d[\d_]*(?:[eE][-+]?\d+)?)"
                    r"(?:_?(?:f32|f64|i32|i64|u8|u16|u32|u64|usize|isize|[fFdDL]))?(?![\w.])")
RUST_FN = re.compile(r"\bfn\s+([A-Za-z_]\w*)")
RUST_IMPL = re.compile(r"^\s*(?:unsafe\s+)?impl\b(?:\s*<.*?>)?\s+(?:(?:[\w:]+)(?:<.*?>)?\s+for\s+)?([\w:]+)")
RUST_TRAIT = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+(\w+)")
RUST_MOD = re.compile(r"(?:^|\]\s*)(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*$")
TEST_FILE = re.compile(r"(?:^|/)(?:tests?|benches|examples)/|(?:^|/|_)tests?\.rs$|_fixture\.rs$|fixture[s]?/|/probe")
RUST_CALL = re.compile(r"\b([a-z_][a-z0-9_]*)\s*(?:::\s*<[^<>]*(?:<[^<>]*>[^<>]*)*>)?\s*\(")
RUST_STRING = re.compile(r'"((?:[^"\\]|\\.)*)"')


# ---- Source text --------------------------------------------------------------------------------------


def blank_java(text):
    """The code with comments and literals blanked out (offsets and newlines kept), the comments and the
    string literals with their offsets."""
    out = list(text)
    strings = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif text.startswith('"""', i):
            j = text.find('"""', i + 3)
            j = n if j < 0 else j + 3
            strings.append((i, text[i + 3:j - 3]))
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif c == '"' or c == "'":
            j = i + 1
            while j < n and text[j] != c and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            if c == '"':
                strings.append((i, text[i + 1:j]))
            for k in range(i, min(j + 1, n)):
                out[k] = " "
            i = j + 1
        else:
            i += 1
    return "".join(out), strings


def blank_rust(text):
    """Like blank_java for Rust: line and block comments (nested), strings, raw strings, char literals but
    not lifetimes. Also gives the comments as (line, text)."""
    out = list(text)
    comments, strings = [], []
    i, n = 0, len(text)
    line = 1
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            i += 1
        elif text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            comments.append((line, text[i:j]))
            for k in range(i, j):
                out[k] = " "
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
            body = text[i:j]
            for offset, part in enumerate(body.split("\n")):
                comments.append((line + offset, part))
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            line += body.count("\n")
            i = j
        elif (c == "r" and re.match(r'r#*"', text[i:i + 8])) or (c == "b" and re.match(r'br#*"', text[i:i + 9])):
            start = text.index('"', i)
            hashes = text[i:start].count("#")
            end = text.find('"' + "#" * hashes, start + 1)
            end = n if end < 0 else end + 1 + hashes
            strings.append((i, text[start + 1:end - 1 - hashes]))
            for k in range(i, end):
                if out[k] != "\n":
                    out[k] = " "
            line += text[i:end].count("\n")
            i = end
        elif c == '"' or (c == "b" and text.startswith('b"', i)):
            start = i + (1 if c == "b" else 0)
            j = start + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            strings.append((i, text[start + 1:j]))
            for k in range(i, min(j + 1, n)):
                if out[k] != "\n":
                    out[k] = " "
            line += text[i:j].count("\n")
            i = j + 1
        elif c == "'":
            # A char literal ('a', '\n', '\u{1F600}'), not a lifetime ('a, 'static).
            m = re.match(r"'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^\\'\n])'", text[i:i + 12])
            if m:
                for k in range(i, i + m.end()):
                    out[k] = " "
                i += m.end()
            else:
                i += 1
        else:
            i += 1
    return "".join(out), comments, strings


def line_starts(text):
    starts = [0]
    for m in re.finditer("\n", text):
        starts.append(m.end())
    return starts


def line_of(starts, offset):
    lo, hi = 0, len(starts) - 1
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if starts[mid] <= offset:
            lo = mid
        else:
            hi = mid - 1
    return lo + 1


def number_token(raw):
    """A numeric literal's value as a canonical token, or None for a trivial one."""
    s = raw.replace("_", "")
    s = re.sub(r"(f32|f64|i32|i64|u8|u16|u32|u64|usize|isize|[fFdDL])$", "", s)
    try:
        v = float(s)
    except ValueError:
        return None
    if v.is_integer() and abs(v) < 1e15 and "." not in s and "e" not in s.lower():
        token = str(int(v))
    else:
        token = repr(round(v, 9))
    return None if token in TRIVIAL_NUMBERS or token.lstrip("-") in TRIVIAL_NUMBERS else "n:" + token


def id_tokens_from_string(s):
    """Game id tokens a Rust string literal names: "minecraft:entity.shulker_bullet.hit" ->
    entity_shulker_bullet_hit (and its last two parts, for ids whose holder drops a prefix)."""
    s = s.strip()
    if not re.fullmatch(r"(?:[a-z0-9_.-]+:)?[a-z0-9_./-]+", s) or len(s) < 4:
        return set()
    path = s.split(":", 1)[-1]
    token = re.sub(r"[./-]", "_", path)
    if "_" not in token and len(token) < 6:
        return set()
    return {"id:" + token}


# ---- Java ---------------------------------------------------------------------------------------------


def parse_java(path_and_root):
    path, root = path_and_root
    rel = os.path.relpath(path, root).replace(os.sep, "/")
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return []
    code, strings = blank_java(text)
    starts = line_starts(text)
    package = (re.search(r"^\s*package\s+([\w.]+)\s*;", code, re.M) or [None, ""])[1].replace(".", "/")
    imports = {m[1].rsplit(".", 1)[-1]: m[1] for m in re.finditer(r"^\s*import\s+(?:static\s+)?([\w.]+)\s*;", code, re.M)}
    classes = []  # dicts
    scopes = []  # (kind, ref, depth) where kind: class/method/block
    depth = 0
    seg_start = 0  # where the current header text begins
    method_body = {}  # id(method dict) -> [body start, body end]
    i, n = 0, len(code)
    while i < n:
        c = code[i]
        if c == "{":
            header = code[seg_start:i].strip()
            header_one = " ".join(header.split())
            parent = scopes[-1] if scopes else None
            m = JAVA_CLASS.search(header_one)
            in_method = any(s[0] == "method" for s in scopes)
            if m and "new " not in header_one.split(m.group(0))[0][-6:] and not in_method:
                outer = next((s[1] for s in reversed(scopes) if s[0] == "class"), None)
                name = m.group(2)
                cls = {
                    "name": name,
                    "path": (outer["path"] + "$" + name) if outer else (package + "/" + name if package else name),
                    "kind": m.group(1),
                    "file": rel,
                    "line": line_of(starts, i),
                    "extends": [],
                    "methods": {},
                    "imports": imports,
                    "package": package,
                    "outer": outer["path"] if outer else None,
                    "fields": {},
                }
                ext = JAVA_EXTENDS.search(header_one)
                if ext:
                    cls["extends"].append(ext.group(1).split("<")[0])
                imp = JAVA_IMPLEMENTS.search(header_one)
                if imp:
                    cls["extends"] += [p.strip().split("<")[0] for p in re.split(r",(?![^<]*>)", imp.group(1)) if p.strip()]
                if m.group(1) == "record":
                    cls["fields"].update({v: t.split(".")[-1] for t, v in JAVA_TYPED.findall(header_one.split("(", 1)[-1])})
                    for field in re.findall(r"\b([a-z]\w*)\s*(?:,|\)\s*(?:implements|$))", header_one.split("(", 1)[-1]):
                        cls["methods"].setdefault(field, {"name": field, "lines": [], "calls": [], "tokens": [], "client": False, "accessor": True})
                classes.append(cls)
                scopes.append(("class", cls, depth))
            elif parent and parent[0] == "class" and not in_method:
                mm = JAVA_METHOD.search(header_one)
                name = mm.group(1) if mm else None
                if header_one.startswith("static") and header_one.strip() == "static":
                    name = "<clinit>"
                elif name in JAVA_KEYWORDS or (mm and "=" in header_one[:mm.start()]):
                    name = None
                elif not mm and re.fullmatch(r"(?:public\s+|protected\s+|private\s+)*" + re.escape(parent[1]["name"]), header_one):
                    name = "<init>"  # A record's compact constructor.
                if name and name == parent[1]["name"]:
                    name = "<init>"
                elif name and name[0].isupper():
                    name = None  # An enum constant's body.
                if name:
                    cls = parent[1]
                    meth = cls["methods"].setdefault(name, {"name": name, "lines": [], "calls": [], "tokens": [], "client": False})
                    meth.pop("accessor", None)
                    params = header_one[mm.end(1):] if mm else ""
                    meth.setdefault("vars", {}).update({v: t.split(".")[-1] for t, v in JAVA_TYPED.findall(params)})
                    span = [line_of(starts, i), None]
                    meth["lines"].append(span)
                    scopes.append(("method", (meth, span, i), depth))
                else:
                    scopes.append(("block", None, depth))
            else:
                scopes.append(("block", None, depth))
            depth += 1
            seg_start = i + 1
        elif c == "}":
            depth -= 1
            if scopes:
                kind, ref, _ = scopes.pop()
                if kind == "method":
                    meth, span, body_start = ref
                    span[1] = line_of(starts, i)
                    body = code[body_start:i]
                    raw = text[body_start:i]
                    for m in JAVA_CALL.finditer(body):
                        recv, dot, name = m.group(1), m.group(2), m.group(3)
                        if name in JAVA_KEYWORDS:
                            continue
                        before = body[max(0, m.start() - 4):m.start()]
                        if before.endswith("new "):
                            continue
                        meth["calls"].append([recv or (dot and "?") or "", name])
                    for m in JAVA_METHOD_REF.finditer(body):
                        meth["calls"].append([m.group(1), m.group(2)])
                    for m in JAVA_NEW.finditer(body):
                        meth["calls"].append([m.group(1).split(".")[-1], "<init>"])
                    tokens = set()
                    for m in JAVA_ID.finditer(body):
                        tokens.add("id:" + m.group(2).lower())
                    for m in NUMBER.finditer(body):
                        t = number_token(m.group(0))
                        if t:
                            tokens.add(t)
                    for offset, s in strings:
                        if body_start <= offset < i:
                            tokens |= id_tokens_from_string(s)
                    meth["tokens"] = sorted(set(meth["tokens"]) | tokens)
                    meth.setdefault("vars", {}).update({v: t.split(".")[-1] for t, v in JAVA_TYPED.findall(body)})
                    for t, v in re.findall(r"\binstanceof\s+([A-Z][\w.]*)(?:<[^>]*>)?\s+([a-z]\w*)", body):
                        meth["vars"][v] = t.split(".")[-1]
                    if CLIENT_EFFECTS.search(raw):
                        meth["client"] = True
            seg_start = i + 1
        elif c == ";":
            # An abstract or interface method: a declaration with no body.
            if scopes and scopes[-1][0] == "class":
                header_one = " ".join(code[seg_start:i].split())
                fm = re.search(r"\b([A-Z][\w.]*)(?:<.*>)?(?:\[\])*\s+([a-z]\w*)\s*(?:=.*)?$", header_one)
                if fm and "(" not in header_one.split("=")[0]:
                    scopes[-1][1]["fields"][fm.group(2)] = fm.group(1).split(".")[-1]
                mm = JAVA_METHOD.search(header_one)
                if mm and "=" not in header_one[:mm.start()] and mm.group(1) not in JAVA_KEYWORDS:
                    cls = scopes[-1][1]
                    cls["methods"].setdefault(mm.group(1), {"name": mm.group(1), "lines": [], "calls": [], "tokens": [], "client": False, "abstract": True})
            seg_start = i + 1
        i += 1
    for cls in classes:
        for meth in cls["methods"].values():
            meth["calls"] = sorted({tuple(x) for x in meth["calls"]})
    return classes


def java_files(roots):
    seen = {}
    for root in roots:
        for dirpath, _, files in os.walk(root):
            for f in files:
                if f.endswith(".java"):
                    full = os.path.join(dirpath, f)
                    rel = os.path.relpath(full, root).replace(os.sep, "/")
                    seen.setdefault(rel, (full, root))  # client and server share code: first wins
    return sorted(seen.values())


# ---- Rust ---------------------------------------------------------------------------------------------


def parse_rust(path):
    text = (ROOT / path).read_text(encoding="utf-8", errors="replace")
    code, comments, strings = blank_rust(text)
    starts = line_starts(text)
    lines_raw = text.split("\n")
    fns = []
    scopes = []  # (kind, ref)
    depth = 0
    seg_start = 0
    paren = 0
    i, n = 0, len(code)
    while i < n:
        c = code[i]
        if c == "(" or c == "[":
            paren += 1
        elif c == ")" or c == "]":
            paren = max(0, paren - 1)
        elif c == "{":
            header = " ".join(code[seg_start:i].split())
            m = RUST_FN.search(header)
            impl_type = next((s[1] for s in reversed(scopes) if s[0] in ("impl", "trait")), None)
            if m and "=" not in header[:m.start()]:
                line = line_of(starts, seg_start + (code[seg_start:i].find("fn") if "fn" in code[seg_start:i] else 0))
                owner = next((s[1] for s in reversed(scopes) if s[0] == "fn"), None)
                in_tests = any(s[0] == "mod" and s[1] in ("tests", "test") for s in scopes) or "#[test]" in header
                fn = {
                    "name": m.group(1),
                    "owner": impl_type,
                    "file": path,
                    "line": line,
                    "end": None,
                    "parent": owner["name"] if owner else None,
                    "body": i,
                    "test": bool(in_tests or TEST_FILE.search(path)),
                }
                fns.append(fn)
                scopes.append(("fn", fn))
            elif RUST_IMPL.match(header) and "fn " not in header:
                scopes.append(("impl", RUST_IMPL.match(header).group(1).split("::")[-1]))
            elif RUST_TRAIT.match(header):
                scopes.append(("trait", RUST_TRAIT.match(header).group(1)))
            elif RUST_MOD.match(header):
                scopes.append(("mod", RUST_MOD.match(header).group(1)))
            else:
                scopes.append(("block", None))
            depth += 1
            seg_start = i + 1
        elif c == "}":
            depth -= 1
            if scopes:
                kind, ref = scopes.pop()
                if kind == "fn":
                    ref["end"] = line_of(starts, i)
                    ref["body_end"] = i
            seg_start = i + 1
        elif c == ";" and paren == 0:
            header = " ".join(code[seg_start:i].split())
            m = RUST_FN.search(header)
            if m and scopes and scopes[-1][0] == "trait" and "=" not in header[:m.start()]:
                line = line_of(starts, seg_start + code[seg_start:i].find("fn"))
                fns.append({"name": m.group(1), "owner": scopes[-1][1], "file": path, "line": line, "end": line,
                            "parent": None, "body": i, "body_end": i, "decl": True, "test": bool(TEST_FILE.search(path))})
            seg_start = i + 1
        i += 1
    # Each fn's own code (nested fns excluded) for calls and literals.
    fns.sort(key=lambda f: (f["line"], -(f["end"] or 0)))
    for fn in fns:
        body = list(code[fn["body"]:fn.get("body_end", fn["body"])])
        for other in fns:
            if other is fn or other["body"] <= fn["body"] or other.get("body_end", 0) > fn.get("body_end", 0):
                continue
            for k in range(other["body"] - fn["body"], other.get("body_end", other["body"]) - fn["body"]):
                if 0 <= k < len(body):
                    body[k] = " "
        body = "".join(body)
        calls = set()
        for m in RUST_CALL.finditer(body):
            name = m.group(1)
            if name in RUST_KEYWORDS:
                continue
            nxt = body[m.end() - 1 - 1:m.end() - 1]
            if body[m.start(1) + len(name):m.start(1) + len(name) + 1] == "!":
                continue
            calls.add(name)
        # Method references passed as values: Game::swing_arm, Self::tick, |g| g.foo
        for m in re.finditer(r"(?:\bSelf|\b[A-Z]\w*)::([a-z_]\w*)\b(?!\s*\()", body):
            calls.add(m.group(1))
        tokens = set()
        for m in NUMBER.finditer(body):
            t = number_token(m.group(0))
            if t:
                tokens.add(t)
        for offset, s in strings:
            if fn["body"] <= offset < fn.get("body_end", 0):
                tokens |= id_tokens_from_string(s)
        fn["calls"] = sorted(calls)
        fn["tokens"] = sorted(tokens)
    # Comments: a fn's doc and attribute block above its `fn` line, and the comments inside it (not inside
    # a nested fn).
    cites_by_fn = defaultdict(list)
    spans = sorted(((f["line"], f["end"] or f["line"], idx) for idx, f in enumerate(fns)), key=lambda s: (s[0], -s[1]))
    def innermost(line):
        best = None
        for a, b, idx in spans:
            if a <= line <= b and (best is None or (a >= fns[best]["line"])):
                best = idx
        return best
    doc_owner = {}
    for idx, fn in enumerate(fns):
        k = fn["line"] - 1  # 1-based line above
        while k >= 1:
            s = lines_raw[k - 1].strip()
            if s.startswith("///") or s.startswith("//") or s.startswith("#[") or s.startswith("#!["):
                doc_owner[k] = idx
                k -= 1
            else:
                break
    file_comments = []
    for line, text_line in comments:
        idx = doc_owner.get(line)
        if idx is None:
            idx = innermost(line)
        if idx is None:
            file_comments.append((line, text_line))
        else:
            cites_by_fn[idx].append((line, text_line))
    for idx, fn in enumerate(fns):
        fn["comments"] = cites_by_fn.get(idx, [])
        fn.pop("body", None)
        fn.pop("body_end", None)
    return {"file": path, "fns": fns, "file_comments": file_comments}


def rust_files():
    out = subprocess.run(["git", "ls-files", "crates/*.rs", "launcher/*.rs"], cwd=ROOT, capture_output=True, text=True).stdout
    return [f for f in out.split() if f.endswith(".rs")]


# ---- Build --------------------------------------------------------------------------------------------


def default_sources():
    lock = json.loads((ROOT / "research-lock.json").read_text()) if (ROOT / "research-lock.json").exists() else {}
    version = (lock.get("minecraft") or {}).get("version") or lock.get("version") or "26.3"
    base = ROOT / ".local" / "minecraft" / str(version) / "sources"
    return [str(base / side) for side in ("client", "server") if (base / side).is_dir()]


class Vanilla:
    """The decompiled classes: simple-name lookup (with `Outer.Inner`), supertypes, methods."""

    def __init__(self, classes):
        self.classes = {c["path"]: c for c in classes}
        self.by_simple = defaultdict(set)
        for c in classes:
            parts = c["path"].rsplit("/", 1)[-1].split("$")
            self.by_simple[parts[-1]].add(c["path"])
            if len(parts) > 1:
                self.by_simple[".".join(parts[-2:])].add(c["path"])
                self.by_simple[".".join(parts)].add(c["path"])

    def resolve(self, simple, near=None):
        """The class a (dotted) simple name means: unique; else the one `near`'s imports, package or outer
        classes give; else the unique top-level one under net/minecraft."""
        cands = self.by_simple.get(simple, set())
        if len(cands) == 1:
            return next(iter(cands))
        if not cands:
            return None
        if near:
            cls = self.classes.get(near)
            if cls:
                imported = cls["imports"].get(simple.split(".")[0])
                if imported:
                    want = imported.replace(".", "/")
                    rest = simple.split(".")[1:]
                    for cand in cands:
                        if cand == want + ("$" + "$".join(rest) if rest else "") or cand.replace("$", "/") == want:
                            return cand
                same = [c for c in cands if c.startswith(cls["package"] + "/") and "/" not in c[len(cls["package"]) + 1:]]
                if len(same) == 1:
                    return same[0]
                inner = [c for c in cands if c.startswith(near + "$") or (cls["outer"] and c.startswith(cls["outer"] + "$"))]
                if len(inner) == 1:
                    return inner[0]
        top = [c for c in cands if "$" not in c and c.startswith("net/minecraft/")]
        return top[0] if len(top) == 1 else None

    def lineage(self, path, limit=12):
        """The class and its supertypes, nearest first."""
        out, todo, seen = [], [path], set()
        while todo and len(out) < limit:
            p = todo.pop(0)
            if p in seen or p not in self.classes:
                continue
            seen.add(p)
            out.append(p)
            for sup in self.classes[p]["extends"]:
                r = self.resolve(sup.split(".")[-1] if "." not in sup or sup.startswith(("net.", "com.")) else sup, near=p)
                if not r and "." in sup:
                    r = self.resolve(sup.rsplit(".", 1)[-1], near=p)
                if r:
                    todo.append(r)
        return out

    def find_method(self, path, name):
        """(declaring class, method) for `name` on the class or the nearest supertype that has it."""
        for p in self.lineage(path):
            m = self.classes[p]["methods"].get(name)
            if m:
                return p, m
        return None, None


def key(cls_path, method):
    return f"{cls_path}#{method}"


def short(cls_path, method=None):
    simple = cls_path.rsplit("/", 1)[-1].replace("$", ".")
    return f"{simple}.{method}" if method else simple


def build(sources, jobs):
    if not sources:
        sys.exit("No decompiled sources: run `python3 tools/lab.py decompile`, or pass --sources DIR.")
    files = java_files(sources)
    classes = []
    with ProcessPoolExecutor(max_workers=jobs) as pool:
        for chunk in pool.map(parse_java, files, chunksize=64):
            classes.extend(chunk)
        rust = list(pool.map(parse_rust, rust_files(), chunksize=8))
    van = Vanilla(classes)

    # Vanilla methods: one entry per class and name (overloads merged), with resolved callees.
    methods = {}
    for c in classes:
        for name, m in c["methods"].items():
            methods[key(c["path"], name)] = {
                "class": c["path"], "name": name, "file": c["file"], "lines": m["lines"],
                "calls_raw": m["calls"], "tokens": m["tokens"], "client": m["client"],
                "abstract": m.get("abstract", False), "accessor": m.get("accessor", False),
                "vars": m.get("vars", {}),
                "size": sum((b or a) - a + 1 for a, b in m["lines"]),
            }
    defined = defaultdict(set)
    for k, m in methods.items():
        defined[m["name"]].add(k)
    # A field name every class declares with the same type (player: LocalPlayer in the client classes
    # that hold one) resolves through other classes too (this.minecraft.player.itemUsed).
    seen_types = defaultdict(set)
    for c in classes:
        for f, t in c["fields"].items():
            seen_types[f].add(t)
    field_types = {f: next(iter(t)) for f, t in seen_types.items() if len(t) == 1}
    for k, m in methods.items():
        callees = set()
        for recv, name in m["calls_raw"]:
            target = None
            if recv in ("", "this", "super"):
                decl, _ = van.find_method(m["class"], name)
                if not decl:
                    cls = van.classes[m["class"]]
                    if cls["outer"]:
                        decl, _ = van.find_method(cls["outer"], name)
                target = key(decl, name) if decl else None
            elif recv[:1].isupper() and not recv.isupper():
                cp = van.resolve(recv, near=m["class"])
                if cp:
                    decl, _ = van.find_method(cp, name)
                    target = key(decl, name) if decl else None
            elif recv[:1].islower():
                # A field, parameter or local whose declared type is known.
                typ = m["vars"].get(recv)
                if not typ:
                    for p in van.lineage(m["class"]):
                        typ = van.classes[p]["fields"].get(recv)
                        if typ:
                            break
                if not typ:
                    typ = field_types.get(recv)
                cp = van.resolve(typ, near=m["class"]) if typ else None
                if cp:
                    decl, _ = van.find_method(cp, name)
                    target = key(decl, name) if decl else None
            if not target and name not in GENERIC_CALLS and len(defined.get(name, ())) == 1:
                target = next(iter(defined[name]))
            if target and target != k:
                callees.add(target)
        m["callees"] = sorted(callees)
        del m["calls_raw"]
        del m["vars"]
    callers = defaultdict(set)
    for k, m in methods.items():
        for c in m["callees"]:
            callers[c].add(k)
    # Overrides: a call to Entity.getKnownMovement also runs ServerPlayer.getKnownMovement.
    subclasses = defaultdict(set)
    for p in van.classes:
        for sup in van.lineage(p)[1:]:
            subclasses[sup].add(p)
    def overrides(k):
        m = methods[k]
        return [key(sub, m["name"]) for sub in subclasses.get(m["class"], ()) if key(sub, m["name"]) in methods]

    # Rust functions and their citations.
    fns = []
    stale = []
    class_cites = defaultdict(list)
    file_class_cites = defaultdict(set)
    for f in rust:
        for fn in f["fns"]:
            fn["id"] = len(fns)
            fn["cites"] = []
            fn["primary"] = []
            seen = set()
            doc = [(ln, t) for ln, t in fn["comments"] if ln < fn["line"] and t.strip().startswith("///")]
            lead = doc[0] if doc else None
            for line, text in fn["comments"]:
                is_lead = lead is not None and line == lead[0] and re.match(r"\s*///\s*`?[A-Z]", text)
                for m in CITE.finditer(text):
                    cls_name, member = m.group(1), m.group(2)
                    if member and member[:1].isupper():
                        # Outer.Inner (a class) or a constant.
                        if van.resolve(f"{cls_name}.{member}"):
                            cls_name, member = f"{cls_name}.{member}", None
                        else:
                            continue
                    cp = van.resolve(cls_name)
                    if not cp:
                        if member and sum(ch.isupper() for ch in cls_name) >= 1 and len(cls_name) > 3 and cls_name not in ("Self", "Some", "None", "Ok", "Err", "Vec", "Option", "Result", "String", "Box", "Arc", "Rc", "HashMap", "HashSet", "BTreeMap"):
                            stale.append({"what": "class", "cite": f"{cls_name}.{member}", "file": f["file"], "line": line})
                        continue
                    if not member:
                        if sum(ch.isupper() for ch in cls_name.split(".")[-1]) >= 2:
                            class_cites[cp].append(fn["id"])
                        continue
                    decl, meth = van.find_method(cp, member)
                    if not decl:
                        cls = van.classes[cp]
                        fields = re.search(r"\b" + re.escape(member) + r"\b", open(os.path.join(next(s for s in sources if os.path.exists(os.path.join(s, cls["file"]))), cls["file"]), errors="replace").read()) if member[0].islower() else None
                        if not fields:
                            stale.append({"what": "method", "cite": f"{short(cp)}.{member}", "file": f["file"], "line": line})
                        continue
                    k = key(decl, member)
                    if is_lead and k not in fn["primary"]:
                        fn["primary"].append(k)
                    if k not in seen:
                        seen.add(k)
                        fn["cites"].append(k)
            del fn["comments"]
            fns.append(fn)
        for line, text in f["file_comments"]:
            for m in CITE.finditer(text):
                if not m.group(2) and sum(ch.isupper() for ch in m.group(1).split(".")[-1]) >= 2:
                    cp = van.resolve(m.group(1))
                    if cp:
                        file_class_cites[cp].add(f["file"])
    ported = defaultdict(list)  # vanilla key -> fns whose doc opens with it
    mentioned = defaultdict(list)  # vanilla key -> fns that name it elsewhere
    for fn in fns:
        if fn.get("test"):
            continue
        for k in fn["primary"]:
            ported[k].append(fn["id"])
        for k in fn["cites"]:
            if k not in fn["primary"]:
                mentioned[k].append(fn["id"])

    # Named matches: a fn with the method's own name (tickRain -> tick_rain) in a file that cites the class.
    files_citing = defaultdict(set)
    for fn in fns:
        for k in fn["cites"]:
            files_citing[methods[k]["class"]].add(fn["file"]) if k in methods else None
    for cp, ids in class_cites.items():
        for fid in ids:
            files_citing[cp].add(fns[fid]["file"])
    for cp, files_ in file_class_cites.items():
        files_citing[cp] |= files_
    by_name_live = defaultdict(list)
    for fn in fns:
        if not fn.get("test"):
            by_name_live[fn["name"]].append(fn["id"])
    named = {}
    for k, m in methods.items():
        if k in ported or k in mentioned or m["name"].startswith("<") or len(m["name"]) < 4:
            continue
        snake = re.sub(r"(?<!^)(?=[A-Z])", "_", m["name"]).lower()
        if snake in GENERIC_CALLS:
            continue
        homes = files_citing.get(m["class"], set())
        outer = van.classes[m["class"]]["outer"]
        if outer:
            homes = homes | files_citing.get(outer, set())
        hits = [fid for fid in by_name_live.get(snake, ()) if fns[fid]["file"] in homes]
        if hits:
            named[k] = hits

    # Inferred matches for what nobody cites: rare shared ids and numbers, plus the name.
    df_j, df_r = Counter(), Counter()
    for m in methods.values():
        df_j.update(set(m["tokens"]))
    for fn in fns:
        df_r.update(set(fn["tokens"]))
    by_token = defaultdict(list)
    for fn in fns:
        for t in fn["tokens"]:
            if df_r[t] <= 6:
                by_token[t].append(fn["id"])
    nj, nr = max(1, len(methods)), max(1, len(fns))
    inferred = {}
    for k, m in methods.items():
        if k in ported or k in mentioned or k in named or m["class"].startswith(SKIP_PACKAGES):
            continue
        rare = [t for t in m["tokens"] if df_j[t] <= 6 and t in by_token]
        if not rare:
            continue
        score = defaultdict(float)
        shared = defaultdict(list)
        for t in rare:
            w = math.log(nj / df_j[t]) + math.log(nr / df_r[t])
            for fid in by_token[t]:
                score[fid] += w
                shared[fid].append(t)
        snake = re.sub(r"(?<!^)(?=[A-Z])", "_", m["name"]).lower()
        best = []
        for fid, s in score.items():
            n_shared = len(shared[fid])
            ids = sum(t.startswith("id:") for t in shared[fid])
            name_hit = fns[fid]["name"] == snake or snake in fns[fid]["name"]
            if n_shared >= 3 or (n_shared >= 2 and (ids or name_hit)) or (ids and name_hit):
                best.append((s + (8 if name_hit else 0), fid, shared[fid]))
        best.sort(reverse=True)
        if best:
            inferred[k] = [{"fn": fid, "score": round(s, 1), "shared": sh[:6]} for s, fid, sh in best[:3]]

    # Rust call graph by name (names are what a text index can resolve; ambiguous names fan out).
    by_name = defaultdict(list)
    for fn in fns:
        by_name[fn["name"]].append(fn["id"])
    rcallers = defaultdict(set)
    for fn in fns:
        for c in fn["calls"]:
            if c in GENERIC_CALLS:
                continue
            for fid in by_name.get(c, ()):
                rcallers[fid].add(fn["id"])

    def reach(fid, depth=4):
        """Names of the fns `fid` calls within `depth` steps (generic names don't fan out)."""
        names, frontier = set(), {fid}
        for _ in range(depth):
            nxt = set()
            for f in frontier:
                for c in fns[f]["calls"]:
                    if c not in names:
                        names.add(c)
                        if c not in GENERIC_CALLS and len(by_name.get(c, ())) <= 4:
                            nxt.update(by_name.get(c, ()))
            frontier = nxt
        return names

    remote = {fn["id"] for fn in fns if re.search(r"/game/remote[^/]*\.rs$", fn["file"])}

    def ancestors(fid, depth=2):
        """The fns that call `fid` within `depth` steps (through specific names only)."""
        seen, frontier = set(), {fid}
        for _ in range(depth):
            nxt = set()
            for f in frontier:
                if fns[f]["name"] in GENERIC_CALLS or len(by_name.get(fns[f]["name"], ())) > 4:
                    continue
                nxt |= rcallers.get(f, set()) - seen
            seen |= nxt
            frontier = nxt
        return seen

    def side(fid):
        """Which path a port runs on: the server-client side (game/remote*.rs), the server
        (game/lan*.rs, src/lan/), or the in-frame game."""
        f = fns[fid]["file"]
        if re.search(r"/game/remote[^/]*\.rs$", f):
            return "client"
        if re.search(r"/game/lan[^/]*\.rs$|/src/lan/", f):
            return "server"
        return "frame"

    def callee_port_names(c):
        """Names of the fns that port vanilla method c: those whose doc opens with it, else a fn with the
        same, specific name (itemUsed -> item_used)."""
        names = {fns[f]["name"] for o in [c] + overrides(c) for f in ported.get(o, [])}
        if names:
            return names
        snake = re.sub(r"(?<!^)(?=[A-Z])", "_", methods[c]["name"]).lower()
        if (len(snake) >= 8 or "_" in snake) and snake not in GENERIC_CALLS and 0 < len(by_name.get(snake, ())) <= 4:
            return {snake}
        return set()

    def substantial(c):
        """Worth reporting as a missing call: Horizon ports it as a function of its own (or an override
        of it), or it is more than a getter."""
        if any(ported.get(o) for o in [c] + overrides(c)):
            return True
        m = methods[c]
        if m["size"] <= 3 or m["accessor"]:
            return False
        return not re.match(r"(get|is|has|can|should|set)[A-Z]", m["name"]) or m["size"] > 8

    def reached_from_remote(fid, depth=4):
        seen, frontier = {fid}, {fid}
        for _ in range(depth):
            if frontier & remote:
                return True
            nxt = set()
            for f in frontier:
                if fns[f]["name"] in GENERIC_CALLS:
                    continue
                nxt |= rcallers.get(f, set()) - seen
            seen |= nxt
            frontier = nxt
        return bool(frontier & remote)

    gaps = []
    for k, fids in ported.items():
        m = methods.get(k)
        if not m:
            continue
        reaches = {fid: reach(fid) for fid in fids}
        port_names = {c: callee_port_names(c) for c in m["callees"]}
        port_names = {c: v for c, v in port_names.items() if v}
        # One port is often split over several fns (the click, then each hand's turn; a tick spread over
        # the loop): every port on one path is judged as one, against the ports on the other paths.
        by_side = defaultdict(list)
        for fid in sorted(fids, key=lambda f: (fns[f]["file"], fns[f]["line"])):
            by_side[side(fid)].append(fid)
        clusters = list(by_side.values())
        creach = [set().union(*(reaches[f] for f in c)) | {fns[f]["name"] for f in c} for c in clusters]
        ccover = [{c for c, names in port_names.items() if r & names} for r in creach]
        for callee, targets in port_names.items():
            hit = [i for i, r in enumerate(creach) if r & targets]
            miss = [i for i, r in enumerate(creach) if not (r & targets)]
            if hit and miss:
                for i in miss:
                    peers = [h for h in hit if side(clusters[h][0]) != side(clusters[i][0]) and ccover[h] & ccover[i]]
                    if peers:
                        gaps.append({"kind": "seam", "vanilla": k, "callee": callee, "fn": clusters[i][0],
                                     "cluster": clusters[i], "sibling": clusters[peers[0]][0]})
            elif not hit and len(targets) <= 3 and substantial(callee):
                gaps.append({"kind": "callee", "vanilla": k, "callee": callee, "fn": fids[0]})
        if m["client"] and not any(reached_from_remote(fid) for fid in fids):
            gaps.append({"kind": "client", "vanilla": k, "fn": fids[0]})
    for s in stale:
        gaps.append({"kind": "stale", **s})
    # Whole classes nothing ports, in packages where Horizon ports the neighbours (a renderer, a sound
    # instance, a goal): the kind of gap no call graph shows, since the game reaches them by registration.
    by_class = defaultdict(list)
    for k, m in methods.items():
        if not m["abstract"] and not m["accessor"] and m["name"] != "<clinit>":
            by_class[m["class"]].append(k)
    pkg_rate = defaultdict(Counter)
    for cp, ks in by_class.items():
        pkg = cp.rsplit("/", 1)[0]
        hit = any(x in ported or x in mentioned for x in ks) or cp in class_cites
        pkg_rate[pkg]["classes"] += 1
        pkg_rate[pkg]["hit"] += hit
    for cp, ks in by_class.items():
        if cp.startswith(SKIP_PACKAGES) or cp.startswith("com/") or "$" in cp or len(ks) < 2:
            continue
        if any(x in ported or x in mentioned or x in inferred for x in ks) or cp in class_cites or cp in file_class_cites:
            continue
        if van.classes[cp]["kind"] in ("interface", "@interface", "record"):
            continue
        pkg = cp.rsplit("/", 1)[0]
        rate = pkg_rate[pkg]["hit"] / max(1, pkg_rate[pkg]["classes"])
        if rate >= 0.3:
            gaps.append({"kind": "class", "vanilla_class": cp, "package_ported": round(rate, 2), "methods": len(ks)})

    ledger = {
        "sources": sources,
        "methods": methods,
        "callers": {k: sorted(v) for k, v in callers.items()},
        "fns": fns,
        "ported": dict(ported),
        "mentioned": dict(mentioned),
        "named": named,
        "inferred": inferred,
        "class_cites": {k: sorted(set(v)) for k, v in class_cites.items()},
        "file_class_cites": {k: sorted(v) for k, v in file_class_cites.items()},
        "gaps": gaps,
        "classes": {c["path"]: {"file": c["file"], "line": c["line"], "extends": c["extends"], "kind": c["kind"]} for c in classes},
    }
    OUT.mkdir(parents=True, exist_ok=True)
    LEDGER.write_text(json.dumps(ledger))
    return ledger


# ---- Queries ------------------------------------------------------------------------------------------


def load():
    if not LEDGER.exists():
        sys.exit("No ledger yet: run `python3 tools/port_ledger.py build` first.")
    return json.loads(LEDGER.read_text())


def fn_ref(led, fid):
    f = led["fns"][fid]
    owner = f"{f['owner']}::" if f.get("owner") else ""
    return f"{f['file']}:{f['line']}-{f['end']} {owner}{f['name']}"


def status_of(led, k):
    if k in led["ported"]:
        return "ported"
    if k in led["mentioned"]:
        return "mentioned"
    if k in led.get("named", {}):
        return "named"
    if k in led["inferred"]:
        return "inferred"
    return "missing"


def match_classes(led, query):
    q = query.replace(".", "$")
    return [p for p in led["classes"] if p.endswith("/" + q) or p == q or p.rsplit("/", 1)[-1] == q]


def cmd_where(led, query, as_json):
    out = []
    if "." in query and query.split(".")[-1][:1].islower():
        cls_q, meth = query.rsplit(".", 1)
        for cp in match_classes(led, cls_q):
            # The method on the class or a supertype.
            for p in [cp] + [x for x in led["classes"] if False]:
                k = key(p, meth)
                if k in led["methods"]:
                    out.append(method_view(led, k))
    else:
        for cp in match_classes(led, query):
            fids = led["class_cites"].get(cp, [])
            meths = [k for k in led["methods"] if led["methods"][k]["class"] == cp]
            out.append({
                "class": short(cp), "path": cp, "file": led["classes"][cp]["file"],
                "named_in": [fn_ref(led, f) for f in fids[:40]] + led.get("file_class_cites", {}).get(cp, []),
                "methods": {led["methods"][k]["name"]: status_of(led, k) for k in sorted(meths)},
            })
    emit(out, as_json, f"Nothing in the pinned version is called {query}.")


def method_view(led, k):
    m = led["methods"][k]
    return {
        "vanilla": short(m["class"], m["name"]),
        "source": f"{m['file']}:{m['lines'][0][0] if m['lines'] else '?'}",
        "status": status_of(led, k),
        "ported_in": [fn_ref(led, f) for f in led["ported"].get(k, [])],
        "mentioned_in": [fn_ref(led, f) for f in led["mentioned"].get(k, [])][:20],
        "same_name_in": [fn_ref(led, f) for f in led.get("named", {}).get(k, [])],
        "inferred": [f"{fn_ref(led, i['fn'])} (score {i['score']}, shares {', '.join(i['shared'])})" for i in led["inferred"].get(k, [])],
        "client_effect": m["client"],
        "calls": [f"{short(led['methods'][c]['class'], led['methods'][c]['name'])} [{status_of(led, c)}]" for c in m["callees"] if c in led["methods"]][:40],
        "called_by": [short(led["methods"][c]["class"], led["methods"][c]["name"]) for c in led["callers"].get(k, []) if c in led["methods"]][:40],
        "gaps": [gap_text(led, g) for g in led["gaps"] if g.get("vanilla") == k],
    }


def cmd_what(led, where, as_json):
    path, _, line = where.partition(":")
    path = path.removeprefix("./")
    fns = [f for f in led["fns"] if f["file"] == path]
    if line:
        n = int(line)
        inside = [f for f in fns if f["line"] <= n <= (f["end"] or f["line"])]
        fns = inside[-1:] if inside else []
    out = []
    for f in fns:
        out.append({
            "fn": fn_ref(led, f["id"]),
            "ports": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k in f["primary"] if k in led["methods"]],
            "mentions": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k in f["cites"] if k in led["methods"] and k not in f["primary"]],
            "inferred_for": [short(led["methods"][k]["class"], led["methods"][k]["name"]) for k, v in led["inferred"].items() if any(i["fn"] == f["id"] for i in v)],
        })
    emit(out, as_json, f"No function at {where}.")


def cmd_class(led, query, as_json):
    out = []
    for cp in match_classes(led, query):
        rows = []
        for k in sorted(k for k in led["methods"] if led["methods"][k]["class"] == cp):
            m = led["methods"][k]
            where = ([fn_ref(led, f) for f in led["ported"].get(k, [])]
                     or [fn_ref(led, f) for f in led.get("named", {}).get(k, [])]
                     or [fn_ref(led, f) + " (mention)" for f in led["mentioned"].get(k, [])[:1]]
                     or [fn_ref(led, i["fn"]) + " ?" for i in led["inferred"].get(k, [])[:1]])
            rows.append({"method": m["name"], "status": status_of(led, k), "where": where, "callers": len(led["callers"].get(k, []))})
        out.append({"class": short(cp), "file": led["classes"][cp]["file"], "methods": rows})
    emit(out, as_json, f"No class {query}.")


def package_of(path, depth):
    parts = path.split("/")
    base = 2 if parts[:2] == ["net", "minecraft"] else 1
    return "/".join(parts[base:base + depth])


def coverage_rows(led, package=None, depth=2):
    rows = defaultdict(Counter)
    for k, m in led["methods"].items():
        if m["class"].startswith(SKIP_PACKAGES) or m["accessor"] or m["abstract"] or m["name"] == "<clinit>":
            continue
        pkg = package_of(m["class"], depth)
        if package and not pkg.startswith(package) and not m["class"].split("/", 2)[-1].startswith(package):
            continue
        rows[pkg][status_of(led, k)] += 1
        rows[pkg]["total"] += 1
    return rows


def cmd_coverage(led, package, depth, as_json):
    rows = coverage_rows(led, package, depth)
    cols = ("ported", "named", "mentioned", "inferred", "missing")
    out = [dict({"package": p, "total": c["total"]}, **{k: c[k] for k in cols},
                found_pct=round(100 * (c["total"] - c["missing"]) / c["total"], 1))
           for p, c in sorted(rows.items(), key=lambda kv: -kv[1]["total"])]
    if as_json:
        print(json.dumps(out, indent=1))
        return
    tot = Counter()
    for r in out:
        tot.update({k: r[k] for k in ("total",) + cols})
    print(f"{'package':40s} {'methods':>7s} {'ported':>7s} {'named':>6s} {'mention':>7s} {'infer':>6s} {'missing':>7s} found%")
    for r in out + ([dict({"package": "all"}, **tot, found_pct=round(100 * (tot["total"] - tot["missing"]) / tot["total"], 1))] if tot["total"] else []):
        print(f"{r['package']:40s} {r['total']:7d} {r['ported']:7d} {r['named']:6d} {r['mentioned']:7d} {r['inferred']:6d} {r['missing']:7d} {r['found_pct']:5.0f}")


def gap_text(led, g):
    m = led["methods"]
    name = lambda k: short(m[k]["class"], m[k]["name"]) if k in m else k
    if g["kind"] == "seam":
        rest = [led["fns"][f]["name"] for f in g.get("cluster", [])[1:]]
        with_rest = f" (with {', '.join(rest)})" if rest else ""
        return (f"seam: {fn_ref(led, g['fn'])}{with_rest} ports {name(g['vanilla'])} but never reaches the port of "
                f"{name(g['callee'])}; {fn_ref(led, g['sibling'])} does")
    if g["kind"] == "callee":
        return f"callee: {name(g['vanilla'])} calls {name(g['callee'])}; {fn_ref(led, g['fn'])} doesn't reach its port"
    if g["kind"] == "client":
        return (f"client: {name(g['vanilla'])} shows an effect only the client's copy draws; its port "
                f"{fn_ref(led, g['fn'])} isn't reached from game/remote*.rs")
    if g["kind"] == "class":
        return (f"class: nothing ports {short(g['vanilla_class'])} ({g['methods']} methods), though "
                f"{round(100 * g['package_ported'])}% of {g['vanilla_class'].rsplit('/', 1)[0]} is ported")
    if g["kind"] == "stale":
        return f"stale: {g['file']}:{g['line']} cites {g['cite']}, which the pinned version doesn't have"
    return json.dumps(g)


def unported(led, package=None, limit=50):
    rows = []
    for k, m in led["methods"].items():
        if status_of(led, k) != "missing" or m["class"].startswith(SKIP_PACKAGES) or m["accessor"] or m["abstract"]:
            continue
        if m["name"] in ("<clinit>", "toString", "hashCode", "equals", "codec", "streamCodec") or m["class"].startswith("com/"):
            continue
        if package and not m["class"].startswith("net/minecraft/" + package):
            continue
        cls_ported = any(key(m["class"], n) in led["ported"] for n in (led["methods"][x]["name"] for x in [k]))
        rows.append((len(led["callers"].get(k, [])), k))
    rows.sort(reverse=True)
    return [{"vanilla": short(led["methods"][k]["class"], led["methods"][k]["name"]), "callers": n,
             "status": status_of(led, k), "source": led["methods"][k]["file"]} for n, k in rows[:limit]]


def cmd_gaps(led, kind, package, limit, as_json):
    if kind == "unported":
        emit(unported(led, package, limit), as_json, "Nothing unported there.")
        return
    order = {"seam": 0, "client": 1, "class": 2, "callee": 3, "stale": 4}
    gaps = [g for g in led["gaps"] if (not kind or g["kind"] == kind)]
    if package:
        gaps = [g for g in gaps if (g.get("vanilla") or g.get("vanilla_class") or "").startswith("net/minecraft/" + package)
                or package in g.get("file", "")]
    gaps.sort(key=lambda g: (order.get(g["kind"], 9), -g.get("package_ported", 0),
                             g.get("vanilla") or g.get("vanilla_class") or g.get("file", "")))
    shown = gaps[:limit]
    if as_json:
        print(json.dumps([dict(g, text=gap_text(led, g)) for g in shown], indent=1))
        return
    counts = Counter(g["kind"] for g in gaps)
    print(", ".join(f"{k} {v}" for k, v in sorted(counts.items(), key=lambda kv: order.get(kv[0], 9))) or "No gaps.")
    for g in shown:
        print("-", gap_text(led, g))


def emit(out, as_json, empty):
    if as_json:
        print(json.dumps(out, indent=1))
        return
    if not out:
        print(empty)
        return
    for item in out:
        for k, v in item.items():
            if isinstance(v, list):
                if v:
                    print(f"{k}:")
                    for x in v:
                        print(f"  {x if not isinstance(x, dict) else json.dumps(x)}")
            elif isinstance(v, dict):
                print(f"{k}:")
                for a, b in v.items():
                    print(f"  {a}: {b}")
            else:
                print(f"{k}: {v}")
        print()


def cmd_report(led):
    lines = ["# Port ledger", "", f"Vanilla sources: {', '.join(led['sources'])}", ""]
    rows = coverage_rows(led, None, 2)
    tot = Counter()
    for c in rows.values():
        tot.update(c)
    lines += [f"{tot['total']} vanilla methods in the packages Horizon ports: {tot['ported']} ported by a fn whose doc opens "
              f"with them, {tot['named']} by a same-named fn in a file citing the class, {tot['mentioned']} named in a comment, "
              f"{tot['inferred']} matched by shared ids and numbers, {tot['missing']} with no port found.", "",
              "## Coverage by package", "",
              "| Package | Methods | Ported | Named | Mentioned | Inferred | Missing | Found % |",
              "|---|---:|---:|---:|---:|---:|---:|---:|"]
    for p, c in sorted(rows.items(), key=lambda kv: -kv[1]["total"])[:80]:
        lines.append(f"| {p} | {c['total']} | {c['ported']} | {c['named']} | {c['mentioned']} | {c['inferred']} | "
                     f"{c['missing']} | {100 * (c['total'] - c['missing']) / c['total']:.0f} |")
    counts = Counter(g["kind"] for g in led["gaps"])
    lines += ["", "## Gaps", "", ", ".join(f"{k}: {v}" for k, v in counts.most_common()), ""]
    for kind, title in (("seam", "Ported on both paths (in-frame and server-client), wired on only one"),
                        ("client", "Client-only effects the server-client path never reaches"),
                        ("class", "Whole classes nothing ports, next to ported neighbours"),
                        ("callee", "Ported callers that never reach a ported callee"),
                        ("stale", "Citations the pinned version doesn't have")):
        items = [g for g in led["gaps"] if g["kind"] == kind]
        if not items:
            continue
        lines += [f"### {title} ({len(items)})", ""]
        lines += ["- " + gap_text(led, g).split(": ", 1)[1] for g in items[:300]]
        lines.append("")
    lines += ["## Most-called vanilla methods with no port", ""]
    lines += [f"- {u['vanilla']} ({u['callers']} callers, {u['status']})" for u in unported(led, None, 150)]
    (OUT / "report.md").write_text("\n".join(lines) + "\n")
    print(OUT / "report.md")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    sub = parser.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--sources", action="append", help="a directory of decompiled vanilla Java (repeatable)")
    b.add_argument("--jobs", type=int, default=os.cpu_count())
    sub.add_parser("where").add_argument("query")
    sub.add_parser("what").add_argument("location")
    sub.add_parser("class").add_argument("query")
    c = sub.add_parser("coverage")
    c.add_argument("--package")
    c.add_argument("--depth", type=int, default=2)
    g = sub.add_parser("gaps")
    g.add_argument("--kind", choices=["seam", "client", "class", "callee", "stale", "unported"])
    g.add_argument("--package")
    g.add_argument("--limit", type=int, default=50)
    sub.add_parser("report")
    args = parser.parse_args()
    if args.cmd == "build":
        led = build(args.sources or default_sources(), args.jobs)
        rows = coverage_rows(led)
        tot = Counter()
        for r in rows.values():
            tot.update(r)
        found = tot["total"] - tot["missing"]
        print(f"{len(led['methods'])} vanilla methods, {len(led['fns'])} Rust functions; of {tot['total']} in ported "
              f"packages: {tot['ported']} ported, {tot['named']} named, {tot['mentioned']} mentioned, {tot['inferred']} "
              f"inferred ({100 * found / max(1, tot['total']):.0f}% found); gaps: "
              + ", ".join(f"{k} {v}" for k, v in Counter(gg['kind'] for gg in led['gaps']).most_common())
              + f" -> {LEDGER}")
        return
    led = load()
    if args.cmd == "where":
        cmd_where(led, args.query, args.json)
    elif args.cmd == "what":
        cmd_what(led, args.location, args.json)
    elif args.cmd == "class":
        cmd_class(led, args.query, args.json)
    elif args.cmd == "coverage":
        cmd_coverage(led, args.package, args.depth, args.json)
    elif args.cmd == "gaps":
        cmd_gaps(led, args.kind, args.package, args.limit, args.json)
    elif args.cmd == "report":
        cmd_report(led)


if __name__ == "__main__":
    main()
