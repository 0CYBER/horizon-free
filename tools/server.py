#!/usr/bin/env python3
"""Prepare/run a vanilla-rules Fabric server with pinned memory and simulation optimizers."""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

try:
    from . import lab, runtime
except ImportError:
    import lab
    import runtime

ROOT = lab.ROOT


def prepare(instance, baseline=False):
    lock, directory = lab.config()
    game = lock["minecraft"]
    manifest_file = runtime.fetch(game["manifest_url"], directory / "manifest.json", game["manifest_sha1"])
    manifest = json.loads(manifest_file.read_text())
    artifact = manifest["downloads"]["server"]
    launcher = runtime.fetch(artifact["url"], directory / "jars/server.jar", artifact["sha1"])
    server, libraries = lab.unpack_server(launcher, directory, game["version"])
    libraries = [server, *libraries, *runtime.fabric_libraries()]
    pack_lock = json.loads((ROOT / "packs/mods.lock.json").read_text())
    mods = pack_lock["projects"]
    selection = ["fabric-api"] if baseline else ["fabric-api", "lithium", "ferrite-core"]
    mod_dir = instance / "mods"
    mod_dir.mkdir(parents=True, exist_ok=True)
    for name in selection:
        entry = mods[name]
        source = runtime.fetch(entry["url"], ROOT / ".local/pack-cache" / entry["filename"],
                               entry["hashes"]["sha512"], "sha512")
        shutil.copyfile(source, mod_dir / entry["filename"])
    original = ROOT / "dist" / f"horizon-lab-{pack_lock['pack_version']}.jar"
    if not original.exists():
        raise FileNotFoundError("Build the original mod first with tools/build_mod.py, or use the server starter ZIP")
    runtime.install_original_mod(original, mod_dir)
    return libraries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "run"])
    parser.add_argument("--instance", type=Path, default=ROOT / ".local/server/optimized")
    parser.add_argument("--heap", default="4G")
    parser.add_argument("--baseline", action="store_true", help="Use a separate instance for the baseline")
    parser.add_argument("--compact-headers", action="store_true", help="Enable Java 25 compact object headers for this server process")
    args = parser.parse_args()
    if not re.fullmatch(r"[1-9][0-9]*[MGmg]", args.heap):
        parser.error("--heap must look like 4G")
    instance = args.instance.resolve()
    if args.baseline and args.instance == parser.get_default("instance"):
        instance = ROOT / ".local/server/baseline"
    libraries = prepare(instance, args.baseline)
    if args.command == "prepare":
        print(f"Prepared server at {instance}. Read https://www.minecraft.net/eula before setting eula=true locally.")
        return
    eula = instance / "eula.txt"
    if not eula.exists() or not re.search(r"(?m)^\s*eula\s*=\s*true\s*$", eula.read_text()):
        if not eula.exists():
            eula.write_text("# Read https://www.minecraft.net/eula then change this value yourself.\neula=false\n")
        print(f"Server prepared. Read Minecraft's EULA and set eula=true in {eula} before running.")
        return
    subprocess.run(["java", "-Xms512M", f"-Xmx{args.heap}", *(["-XX:+UseCompactObjectHeaders"] if args.compact_headers else []),
                    "--enable-native-access=ALL-UNNAMED",
                    "-cp", os.pathsep.join(map(str, libraries)), "net.fabricmc.loader.impl.launch.knot.KnotServer",
                    "--nogui"], cwd=instance, check=True)


if __name__ == "__main__":
    main()
