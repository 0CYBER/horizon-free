#!/usr/bin/env python3
"""Make a valid mod package report point layout zero, for the server refusal check.

Only the exported horizon_layout_ext function body changes. Rehashing mod.wasm keeps
package integrity valid: the loader must reach the point-layout check, not fail SHA512.
"""
import hashlib
import re
import sys
import tomllib
import zipfile
from pathlib import Path


def uint(data, offset):
    value = shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if not byte & 128:
            return value, offset
        shift += 7


def encoded(value):
    out = bytearray()
    while value >= 128:
        out.append((value & 127) | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def mismatch(module):
    if module[:8] != b"\0asm\x01\0\0\0":
        raise ValueError("expected a core wasm module")
    sections = []
    offset = 8
    while offset < len(module):
        kind = module[offset]
        size, start = uint(module, offset + 1)
        sections.append((kind, module[start:start + size]))
        offset = start + size
    exports = next(payload for kind, payload in sections if kind == 7)
    count, offset = uint(exports, 0)
    target = None
    for _ in range(count):
        size, offset = uint(exports, offset)
        name = exports[offset:offset + size]
        offset += size
        kind = exports[offset]
        index, offset = uint(exports, offset + 1)
        if name == b"horizon_layout_ext" and kind == 0:
            target = index
    if target is None:
        raise ValueError("package has no point-layout export")
    # Function indices count imported functions first. The function section lists
    # defined functions only, so total defined bodies also gives the import offset
    # once the import section's function descriptors are counted.
    imported = 0
    for kind, payload in sections:
        if kind != 2:
            continue
        count, offset = uint(payload, 0)
        for _ in range(count):
            for _ in range(2):
                size, offset = uint(payload, offset)
                offset += size
            descriptor = payload[offset]
            offset += 1
            if descriptor == 0:
                _, offset = uint(payload, offset)
                imported += 1
            elif descriptor in (1, 2):
                if descriptor == 1:
                    offset += 1  # funcref/externref table element type
                flags, offset = uint(payload, offset)
                _, offset = uint(payload, offset)
                if flags & 1:
                    _, offset = uint(payload, offset)
            elif descriptor == 3:
                offset += 2  # value type and mutability
            else:
                raise ValueError(f"unsupported import descriptor {descriptor}")
    output = bytearray(module[:8])
    replaced = False
    for kind, payload in sections:
        if kind == 10:
            count, offset = uint(payload, 0)
            bodies = bytearray(encoded(count))
            for index in range(count):
                size, start = uint(payload, offset)
                body = payload[start:start + size]
                offset = start + size
                if index + imported == target:
                    body = b"\x00\x42\x00\x0b"  # no locals; i64.const 0; end
                    replaced = True
                bodies += encoded(len(body)) + body
            payload = bytes(bodies)
        output += bytes([kind]) + encoded(len(payload)) + payload
    if not replaced:
        raise ValueError("point-layout export is not a defined function")
    return bytes(output)


def main():
    source, destination = map(Path, sys.argv[1:])
    with zipfile.ZipFile(source) as package:
        files = {name: package.read(name) for name in package.namelist()}
    manifest = tomllib.loads(files["horizon-mod.toml"].decode())
    files["mod.wasm"] = mismatch(files["mod.wasm"])
    digest = hashlib.sha512(files["mod.wasm"]).hexdigest()
    files["horizon-mod.toml"] = re.sub(
        rb'(?m)^sha512 = "[a-f0-9]+"$', f'sha512 = "{digest}"'.encode(),
        files["horizon-mod.toml"],
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as package:
        for name, data in files.items():
            package.writestr(name, data)
    print(manifest["id"])


if __name__ == "__main__":
    main()
