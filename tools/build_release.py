#!/usr/bin/env python3
"""Create downloadable client/server starters and hashes from original project artifacts."""

import hashlib
from pathlib import Path

try:
    from . import build_pack
except ImportError:
    import build_pack

ROOT = build_pack.ROOT


def main():
    lock = build_pack.read_lock()
    version = lock["pack_version"]
    mod = ROOT / "dist" / f"horizon-lab-{version}.jar"
    if not mod.exists():
        raise FileNotFoundError("Run tools/build_mod.py first")
    packs = [build_pack.build(profile, mod) for profile in build_pack.PROFILES]
    client = {pack.name: pack.read_bytes() for pack in packs}
    client["START-HERE.txt"] = (ROOT / "packs/START-HERE.txt").read_text().replace("@PACK_VERSION@", version).encode()
    client["Mac-Start.command"] = (ROOT / "packs/Mac-Start.command").read_text().replace("@PACK_VERSION@", version).encode()
    client["LICENSE"] = (ROOT / "LICENSE").read_bytes()
    build_pack.write_zip(ROOT / "dist" / f"horizon-mac-starter-{version}.zip", client)
    server = {"LICENSE": client["LICENSE"], "dist/" + mod.name: mod.read_bytes(),
              "START-HERE.txt": (ROOT / "docs/server-quickstart.txt").read_bytes(),
              "research-lock.json": (ROOT / "research-lock.json").read_bytes()}
    for name in ["lab.py", "runtime.py", "server.py", "compare_benchmarks.py"]:
        server["tools/" + name] = (ROOT / "tools" / name).read_bytes()
    for name in ["mods.lock.json", "fabric.lock.json"]:
        server["packs/" + name] = (ROOT / "packs" / name).read_bytes()
    build_pack.write_zip(ROOT / "dist" / f"horizon-server-starter-{version}.zip", server)
    artifacts = sorted(path for path in (ROOT / "dist").glob("*") if version in path.name)
    checksums = []
    for path in artifacts:
        if path.suffix in [".jar", ".mrpack", ".zip"]:
            checksums.append(hashlib.sha256(path.read_bytes()).hexdigest() + "  " + path.name)
    (ROOT / "dist/SHA256SUMS.txt").write_text("\n".join(checksums) + "\n")
    print("Built client/server starters and SHA256SUMS.txt")


if __name__ == "__main__":
    main()
