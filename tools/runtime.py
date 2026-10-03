#!/usr/bin/env python3
"""Prepare an isolated, authenticated-by-launcher game runtime; local smoke tests use demo mode."""

from concurrent.futures import ThreadPoolExecutor
import argparse
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
import urllib.error
import zipfile

try:
    from . import lab
except ImportError:
    import lab

ROOT = lab.ROOT
CACHE = ROOT / ".local" / "runtime"


def fetch(url, path, checksum, algorithm="sha1"):
    for attempt in range(3):
        try:
            return lab.download(url, path, checksum, algorithm)
        except (OSError, urllib.error.URLError):
            if attempt == 2:
                raise
            time.sleep(attempt + 1)


def fabric_libraries():
    profile = json.loads((ROOT / "packs/fabric.lock.json").read_text())
    jars = []
    for library in profile["libraries"]:
        group, artifact, version = library["name"].split(":")
        relative = f"{group.replace('.', '/')}/{artifact}/{version}/{artifact}-{version}.jar"
        jars.append(fetch(library["url"] + relative, CACHE / "fabric" / relative,
                          library["sha256"], "sha256"))
    return jars


def library_allowed(library, system=None, architecture=None):
    system = system or {"Darwin": "osx", "Linux": "linux", "Windows": "windows"}[platform.system()]
    architecture = (architecture or platform.machine()).lower()
    rules = library.get("rules", [])
    allowed = not rules
    for rule in rules:
        operating_system = rule.get("os", {})
        if operating_system.get("name", system) != system:
            continue
        if "arch" in operating_system and not re.fullmatch(operating_system["arch"], architecture):
            continue
        if "version" in operating_system and not re.search(operating_system["version"], platform.release()):
            continue
        if rule.get("features"):
            continue
        allowed = rule["action"] == "allow"
    return allowed


def native_library_matches(library, architecture=None):
    """Modern manifests list both x64 and ARM natives with OS-only rules."""
    architecture = (architecture or platform.machine()).lower()
    name = library["name"]
    if ":natives-" not in name:
        return True
    arm = architecture in {"arm64", "aarch64"}
    if "-arm64" in name or "-aarch64" in name:
        return arm
    return not arm


def prepare_client(assets=True):
    lock, directory = lab.config()
    game = lock["minecraft"]
    manifest_file = fetch(game["manifest_url"], directory / "manifest.json", game["manifest_sha1"])
    manifest = json.loads(manifest_file.read_text())
    if manifest["id"] != game["version"] or manifest["javaVersion"]["majorVersion"] != game["java_major"]:
        raise ValueError("Game manifest does not match research lock")
    artifact = manifest["downloads"]["client"]
    jars = [fetch(artifact["url"], directory / "jars/client.jar", artifact["sha1"])]
    natives = CACHE / "natives"
    natives.mkdir(parents=True, exist_ok=True)
    for library in manifest["libraries"]:
        if not library_allowed(library) or not native_library_matches(library):
            continue
        artifact = library.get("downloads", {}).get("artifact")
        if not artifact:
            continue
        jar = fetch(artifact["url"], directory / "libraries/client" / artifact["path"], artifact["sha1"])
        jars.append(jar)
        with zipfile.ZipFile(jar) as archive:
            for name in archive.namelist():
                if name.endswith((".so", ".dll", ".dylib", ".jnilib")):
                    (natives / Path(name).name).write_bytes(archive.read(name))
    jars.extend(fabric_libraries())
    if assets:
        info = manifest["assetIndex"]
        index = fetch(info["url"], CACHE / "assets/indexes" / f"{info['id']}.json", info["sha1"])
        objects = json.loads(index.read_text())["objects"]
        unique = {v["hash"] for v in objects.values()}
        def asset(checksum):
            return fetch(f"https://resources.download.minecraft.net/{checksum[:2]}/{checksum}",
                         CACHE / "assets/objects" / checksum[:2] / checksum, checksum)
        print(f"Verifying/downloading {len(unique)} unique game assets...", flush=True)
        with ThreadPoolExecutor(max_workers=12) as pool:
            list(pool.map(asset, sorted(unique)))
    return manifest, jars, natives


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["prepare", "demo"])
    parser.add_argument("--instance", type=Path, default=CACHE / "demo")
    parser.add_argument("--heap", default="4G")
    args = parser.parse_args()
    if not re.fullmatch(r"[1-9][0-9]*[MGmg]", args.heap):
        parser.error("--heap must look like 4G")
    manifest, jars, natives = prepare_client()
    if args.command == "prepare":
        return
    instance = args.instance.resolve()
    instance.mkdir(parents=True, exist_ok=True)
    command = ["java", "-Xms512M", f"-Xmx{args.heap}", "--enable-native-access=ALL-UNNAMED",
               f"-Djava.library.path={natives}", f"-Dorg.lwjgl.system.SharedLibraryExtractPath={natives}",
               "-Dfabric.log.disableAnsi=true", "-cp", os.pathsep.join(map(str, jars)),
               "net.fabricmc.loader.impl.launch.knot.KnotClient",
               "--username", "HorizonDemo", "--version", manifest["id"], "--gameDir", str(instance),
               "--assetsDir", str(CACHE / "assets"), "--assetIndex", manifest["assetIndex"]["id"],
               "--accessToken", "0", "--uuid", "00000000000000000000000000000000", "--demo",
               "--width", "1280", "--height", "800"]
    subprocess.run(command, cwd=instance, check=True)


if __name__ == "__main__":
    main()
