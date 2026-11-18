#!/usr/bin/env python3
"""Compare the imported and exported names of two wasm modules.

    python3 tools/ci/wasm_names.py linux/horizon-wasm-std.wasm windows/horizon-wasm-std.wasm

Members of a wasm mod group link by symbol name, so the std member built on any host must name its
symbols the same (#18261). Prints the names only one of them has and exits 1 when there are any.
"""
import sys


def leb(data, pos):
    value = shift = 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        shift += 7
        if byte < 0x80:
            return value, pos


def name(data, pos):
    size, pos = leb(data, pos)
    return data[pos:pos + size].decode(), pos + size


def limits(data, pos):
    flags = data[pos]
    _, pos = leb(data, pos + 1)
    if flags & 1:
        _, pos = leb(data, pos)
    return pos


def names(path):
    """{("import", module, field), ("export", name)} of the module at `path`."""
    data = open(path, "rb").read()
    if data[:4] != b"\0asm":
        sys.exit(f"{path}: not a wasm module")
    found = set()
    pos = 8
    while pos < len(data):
        section = data[pos]
        size, pos = leb(data, pos + 1)
        end = pos + size
        if end > len(data):
            sys.exit(f"{path}: section {section} ends at byte {end}, past the file's {len(data)}")
        if section in (2, 7):
            count, pos = leb(data, pos)
            for _ in range(count):
                if section == 2:
                    module, pos = name(data, pos)
                    field, pos = name(data, pos)
                    kind = data[pos]
                    pos += 1
                    if kind == 0:
                        _, pos = leb(data, pos)
                    elif kind == 1:
                        pos = limits(data, pos + 1)
                    elif kind == 2:
                        pos = limits(data, pos)
                    elif kind == 3:
                        pos += 2
                    elif kind == 4:
                        _, pos = leb(data, pos + 1)
                    else:
                        sys.exit(f"{path}: import kind {kind}")
                    found.add(("import", module, field))
                else:
                    export, pos = name(data, pos)
                    _, pos = leb(data, pos + 1)
                    found.add(("export", export))
        pos = end
    return found


def show(entry):
    # The module/field boundary stays visible: either part may contain a dot.
    if entry[0] == "import":
        return f"import module={entry[1]!r} field={entry[2]!r}"
    return f"export {entry[1]!r}"


def main(argv):
    if len(argv) != 3:
        sys.exit(__doc__)
    left, right = names(argv[1]), names(argv[2])
    for entry in sorted(left - right):
        print(f"only in {argv[1]}: {show(entry)}")
    for entry in sorted(right - left):
        print(f"only in {argv[2]}: {show(entry)}")
    if left != right:
        return 1
    print(f"{len(left)} imported and exported names, the same in both")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
