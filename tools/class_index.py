#!/usr/bin/env python3
"""Index a game jar's classes for version diffs (docs/version-ports.md).

Each class gets its superclass, interfaces, fields and methods; each method a hash of its bytecode in which
every constant-pool reference is replaced by what it names and every branch target by an instruction index.
Recompiling unchanged code (a constant pool reordered, ldc widened to ldc_w) then hashes the same. Lambda
bodies (lambda$name$N) are folded into one entry per declaring method, `lambda$name`, so lambdas renumbered by
an unrelated change don't show as changes.

    class_index.py JAR [JAR...] -o classes.json
"""

import argparse
from concurrent.futures import ProcessPoolExecutor
import hashlib
import json
import re
import struct
import sys
import zipfile

LAMBDA = re.compile(r"^lambda\$(.*)\$\d+$")

# Operand bytes per opcode, for the fixed-length instructions. None: tableswitch, lookupswitch, wide.
OPERANDS = [0] * 202
for _op, _size in [(16, 1), (17, 2), (18, 1), (19, 2), (20, 2), (169, 1), (188, 1), (185, 4), (186, 4), (197, 3)]:
    OPERANDS[_op] = _size
for _op in list(range(21, 26)) + list(range(54, 59)):
    OPERANDS[_op] = 1  # Local variable loads and stores.
OPERANDS[132] = 2  # iinc.
for _op in list(range(153, 169)) + [198, 199]:
    OPERANDS[_op] = 2  # Branches.
OPERANDS[200] = OPERANDS[201] = 4  # goto_w, jsr_w.
for _op in [178, 179, 180, 181, 182, 183, 184, 187, 189, 192, 193]:
    OPERANDS[_op] = 2  # Constant-pool references.
OPERANDS[170] = OPERANDS[171] = OPERANDS[196] = None
CP_OPERAND = {18, 19, 20, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 189, 192, 193, 197}
BRANCH = set(range(153, 169)) | {198, 199, 200, 201}
# ldc_w loads what ldc loads; goto_w jumps where goto jumps.
CANONICAL = {19: 18, 200: 167, 201: 168}


def lambda_owner(name):
    match = LAMBDA.match(name)
    return f"lambda${match[1]}" if match else name


class ClassFile:
    def __init__(self, data):
        self.data = data
        self.offset = 0
        self.parse()

    def u(self, size):
        value = int.from_bytes(self.data[self.offset:self.offset + size], "big")
        self.offset += size
        return value

    def raw(self, size):
        value = self.data[self.offset:self.offset + size]
        self.offset += size
        return value

    def parse(self):
        if self.raw(4) != b"\xca\xfe\xba\xbe":
            raise ValueError("Invalid class file magic")
        self.raw(4)
        count = self.u(2)
        self.pool = [None] * count
        index = 1
        while index < count:
            tag = self.u(1)
            if tag == 1:
                entry = (1, self.raw(self.u(2)).decode("utf-8", errors="replace"))
            elif tag in (3, 4):
                entry = (tag, self.raw(4))
            elif tag in (5, 6):
                entry = (tag, self.raw(8))
            elif tag in (7, 8, 16, 19, 20):
                entry = (tag, self.u(2))
            elif tag in (9, 10, 11, 12, 17, 18):
                entry = (tag, self.u(2), self.u(2))
            elif tag == 15:
                entry = (tag, self.u(1), self.u(2))
            else:
                raise ValueError(f"Unknown constant pool tag {tag}")
            self.pool[index] = entry
            index += 2 if tag in (5, 6) else 1
        self.access = self.u(2)
        self.name = self.class_name(self.u(2))
        super_index = self.u(2)
        self.super = self.class_name(super_index) if super_index else None
        self.interfaces = [self.class_name(self.u(2)) for _ in range(self.u(2))]
        self.fields = {}
        for _ in range(self.u(2)):
            access, name, descriptor = self.u(2), self.utf8(self.u(2)), self.utf8(self.u(2))
            constant = None
            for attribute, data in self.attributes():
                if attribute == "ConstantValue":
                    constant = self.constant(struct.unpack(">H", data)[0])
            self.fields[f"{name} {descriptor}"] = [access, constant]
        self.methods = []
        for _ in range(self.u(2)):
            access, name, descriptor = self.u(2), self.utf8(self.u(2)), self.utf8(self.u(2))
            code = next((data for attribute, data in self.attributes() if attribute == "Code"), None)
            self.methods.append((access, name, descriptor, code))
        self.bootstrap = []
        for attribute, data in self.attributes():
            if attribute == "BootstrapMethods":
                reader = ClassFile.__new__(ClassFile)
                reader.data, reader.offset = data, 0
                for _ in range(reader.u(2)):
                    method = reader.u(2)
                    self.bootstrap.append((method, [reader.u(2) for _ in range(reader.u(2))]))

    def attributes(self):
        return [(self.utf8(self.u(2)), self.raw(self.u(4))) for _ in range(self.u(2))]

    def utf8(self, index):
        return self.pool[index][1]

    def class_name(self, index):
        return self.utf8(self.pool[index][1])

    def member(self, index):
        _, owner, name_and_type = self.pool[index]
        _, name, descriptor = self.pool[name_and_type]
        return f"{self.class_name(owner)}.{lambda_owner(self.utf8(name))}{self.utf8(descriptor)}"

    def constant(self, index):
        """What a constant-pool entry names, independent of where it sits in the pool."""
        entry = self.pool[index]
        tag = entry[0]
        if tag == 1:
            return f"u:{entry[1]}"
        if tag == 3:
            return f"i:{struct.unpack('>i', entry[1])[0]}"
        if tag == 4:
            return f"f:{entry[1].hex()}"
        if tag == 5:
            return f"l:{struct.unpack('>q', entry[1])[0]}"
        if tag == 6:
            return f"d:{entry[1].hex()}"
        if tag == 7:
            return f"c:{self.utf8(entry[1])}"
        if tag == 8:
            return f"s:{self.utf8(entry[1])}"
        if tag in (9, 10, 11):
            return f"m:{self.member(index)}"
        if tag == 12:
            return f"nt:{lambda_owner(self.utf8(entry[1]))}{self.utf8(entry[2])}"
        if tag == 15:
            return f"h{entry[1]}:{self.member(entry[2])}"
        if tag == 16:
            return f"t:{self.utf8(entry[1])}"
        if tag in (17, 18):
            method, arguments = self.bootstrap[entry[1]]
            return (f"y:{self.constant(entry[2])}@{self.constant(method)}"
                    f"({','.join(self.constant(a) for a in arguments)})")
        return f"x{tag}:{entry[1]}"

    def code_tokens(self, code):
        """The method's instructions, exception handlers included, as position-independent tokens."""
        max_stack, max_locals, length = struct.unpack(">HHI", code[:8])
        body = code[8:8 + length]
        starts, instructions = {}, []
        pc = 0
        while pc < length:
            starts[pc] = len(instructions)
            op = body[pc]
            if op == 170 or op == 171:  # tableswitch, lookupswitch
                at = (pc + 4) & ~3
                default = struct.unpack(">i", body[at:at + 4])[0]
                if op == 170:
                    low, high = struct.unpack(">ii", body[at + 4:at + 12])
                    targets = struct.unpack(f">{high - low + 1}i", body[at + 12:at + 12 + 4 * (high - low + 1)])
                    keys = list(range(low, high + 1))
                    end = at + 12 + 4 * len(targets)
                else:
                    count = struct.unpack(">i", body[at + 4:at + 8])[0]
                    pairs = struct.unpack(f">{2 * count}i", body[at + 8:at + 8 + 8 * count])
                    keys, targets = pairs[0::2], pairs[1::2]
                    end = at + 8 + 8 * count
                instructions.append(("switch", pc, default, list(zip(keys, targets))))
                pc = end
                continue
            if op == 196:  # wide
                size = 5 if body[pc + 1] == 132 else 3
                instructions.append(("op", pc, 196, body[pc + 1:pc + 1 + size]))
                pc += 1 + size
                continue
            if op >= len(OPERANDS):
                raise ValueError(f"Unknown opcode {op}")
            size = OPERANDS[op]
            instructions.append(("op", pc, op, body[pc + 1:pc + 1 + size]))
            pc += 1 + size
        tokens = [f"{max_stack},{max_locals}"]
        for kind, pc, *rest in instructions:
            if kind == "switch":
                default, cases = rest
                tokens.append(f"switch {starts.get(pc + default)} "
                              + " ".join(f"{k}>{starts.get(pc + t)}" for k, t in cases))
                continue
            op, operand = rest
            if op in CP_OPERAND:
                index = operand[0] if op == 18 else int.from_bytes(operand[:2], "big")
                tokens.append(f"{CANONICAL.get(op, op)} {self.constant(index)}")
            elif op in BRANCH:
                offset = int.from_bytes(operand, "big", signed=True)
                tokens.append(f"{CANONICAL.get(op, op)} >{starts.get(pc + offset)}")
            else:
                tokens.append(f"{op} {operand.hex()}")
        exceptions = 8 + length
        for i in range(struct.unpack(">H", code[exceptions:exceptions + 2])[0]):
            start, end, handler, kind = struct.unpack(">HHHH", code[exceptions + 2 + 8 * i:exceptions + 10 + 8 * i])
            caught = self.class_name(kind) if kind else "*"
            tokens.append(f"try {starts.get(start)} {starts.get(end, len(instructions))} {starts.get(handler)} {caught}")
        return tokens

    def index(self):
        methods, lambdas = {}, {}
        for access, name, descriptor, code in self.methods:
            tokens = [str(access & 0x0FFF)] + (self.code_tokens(code) if code else ["abstract"])
            digest = hashlib.sha1("\n".join(tokens).encode()).hexdigest()[:16]
            owner = lambda_owner(name)
            if owner != name:
                lambdas.setdefault(owner, []).append(digest)
            else:
                methods[f"{name}{descriptor}"] = digest
        for owner, digests in lambdas.items():
            methods[owner] = hashlib.sha1(" ".join(sorted(digests)).encode()).hexdigest()[:16]
        return {"access": self.access, "super": self.super, "interfaces": self.interfaces,
                "fields": self.fields, "methods": methods}


def index_entries(jar, names):
    with zipfile.ZipFile(jar) as archive:
        return {name[:-6]: ClassFile(archive.read(name)).index() for name in names}


def index_jar(jar, jobs=None, prefixes=("net/minecraft/", "com/mojang/")):
    """{internal class name: index} for every game class in a jar."""
    with zipfile.ZipFile(jar) as archive:
        names = sorted(n for n in archive.namelist() if n.endswith(".class") and n.startswith(prefixes))
    batches = [names[i::32] for i in range(32)]
    result = {}
    with ProcessPoolExecutor(max_workers=jobs) as pool:
        for part in pool.map(index_entries, [jar] * len(batches), batches):
            result.update(part)
    return dict(sorted(result.items()))


def index_jars(jars, jobs=None):
    """Several jars into one index; a class in more than one keeps the first jar's (client before server)."""
    classes = {}
    for jar in jars:
        for name, data in index_jar(jar, jobs).items():
            classes.setdefault(name, data)
    return classes


def local_name(name):
    return name.rsplit("/", 1)[-1]


def moves(added, removed):
    """{old: new} for a removed class whose local name (Outer$Inner) one added class has, uniquely on both sides."""
    def unique(names):
        by_local = {}
        for name in names:
            by_local.setdefault(local_name(name), []).append(name)
        return {local: found[0] for local, found in by_local.items() if len(found) == 1}
    gone, new = unique(removed), unique(added)
    return {gone[local]: new[local] for local in sorted(gone.keys() & new.keys())}


def diff(old, new):
    """Classes added, removed, moved (same local name, another package) and changed; per changed class its
    methods and fields added, removed and changed. A moved class that changed too is listed under its new name
    with moved_from."""
    added = sorted(new.keys() - old.keys())
    removed = sorted(old.keys() - new.keys())
    moved = moves(added, removed)
    targets = set(moved.values())
    added = [name for name in added if name not in targets]
    removed = [name for name in removed if name not in moved]
    changed = {}
    pairs = [(name, name) for name in sorted(old.keys() & new.keys())] + sorted(moved.items())
    for name, target in pairs:
        a, b = old[name], new[target]
        if a == b:
            continue
        change = {"moved_from": name} if name != target else {}
        for part in ("methods", "fields"):
            x, y = a[part], b[part]
            part_change = {"added": sorted(y.keys() - x.keys()), "removed": sorted(x.keys() - y.keys()),
                           "changed": sorted(k for k in x.keys() & y.keys() if x[k] != y[k])}
            if any(part_change.values()):
                change[part] = part_change
        for part in ("super", "interfaces", "access"):
            if a[part] != b[part]:
                change[part] = [a[part], b[part]]
        changed[target] = change
    return {"added": added, "removed": removed, "moved": moved, "changed": dict(sorted(changed.items()))}


def changed_methods(change):
    """How many methods a changed class gained, lost or changed."""
    methods = change.get("methods", {})
    return sum(len(methods.get(k, [])) for k in ("added", "removed", "changed"))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("jars", nargs="+")
    parser.add_argument("-o", "--output", required=True)
    parser.add_argument("-j", "--jobs", type=int)
    args = parser.parse_args()
    classes = index_jars(args.jars, args.jobs)
    with open(args.output, "w") as stream:
        json.dump(classes, stream, separators=(",", ":"))
    print(f"{len(classes)} classes, {sum(len(c['methods']) for c in classes.values())} methods")
    return 0


if __name__ == "__main__":
    sys.exit(main())
